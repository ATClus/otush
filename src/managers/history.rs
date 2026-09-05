//! Transcription history, notes, todos, and documents storage (SQLite).
//!
//! [`HistoryManager`] owns a dedicated rusqlite connection over the history
//! database (migrations via `rusqlite_migration`, WAL mode with busy timeout).
//! All writes run off the GTK main thread; UI updates flow back through the
//! [`EventBus`](crate::context::EventBus) as `AppEvent::HistoryUpdated`.
//! Audio files referenced by entries live under `recordings/`.

use crate::context::{AppEvent, AppPaths, EventBus};
use anyhow::{anyhow, Result};
use chrono::{DateTime, Local, Utc};
use log::{debug, error, info};
use rusqlite::{params, Connection, OptionalExtension};
use rusqlite_migration::{Migrations, M};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Database migrations for transcription history.
/// Each migration is applied in order. The library tracks which migrations
/// have been applied using SQLite's user_version pragma.
///
/// Note: For users upgrading from tauri-plugin-sql, migrate_from_tauri_plugin_sql()
/// converts the old _sqlx_migrations table tracking to the user_version pragma,
/// ensuring migrations don't re-run on existing databases.
static MIGRATIONS: &[M] = &[
    M::up(
        "CREATE TABLE IF NOT EXISTS transcription_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            file_name TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            saved BOOLEAN NOT NULL DEFAULT 0,
            title TEXT NOT NULL,
            transcription_text TEXT NOT NULL
        );",
    ),
    M::up("ALTER TABLE transcription_history ADD COLUMN post_processed_text TEXT;"),
    M::up("ALTER TABLE transcription_history ADD COLUMN post_process_prompt TEXT;"),
    M::up("ALTER TABLE transcription_history ADD COLUMN post_process_requested BOOLEAN NOT NULL DEFAULT 0;"),
    M::up("CREATE INDEX IF NOT EXISTS idx_history_saved_timestamp ON transcription_history (saved, timestamp DESC);"),
    M::up("ALTER TABLE transcription_history ADD COLUMN entry_kind TEXT NOT NULL DEFAULT 'transcription';"),
    M::up("UPDATE transcription_history SET entry_kind = 'post_process' WHERE post_process_requested = 1;"),
    M::up(
        "CREATE TABLE IF NOT EXISTS suite_notes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            title TEXT NOT NULL,
            content TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            tags TEXT NOT NULL DEFAULT '',
            pinned BOOLEAN NOT NULL DEFAULT 0
        );",
    ),
    M::up(
        "CREATE TABLE IF NOT EXISTS suite_todos (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task TEXT NOT NULL,
            completed BOOLEAN NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            completed_at INTEGER,
            priority INTEGER NOT NULL DEFAULT 0,
            due_date INTEGER
        );",
    ),
    M::up(
        "CREATE TABLE IF NOT EXISTS suite_docs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            title TEXT NOT NULL,
            file_name TEXT NOT NULL,
            parsed_content TEXT NOT NULL,
            doc_type TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );",
    ),
    // --- agent chats (AI chat overlay) ---
    M::up(
        "CREATE TABLE IF NOT EXISTS agent_chats (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_id TEXT NOT NULL,
            title TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );",
    ),
    M::up("CREATE INDEX IF NOT EXISTS idx_agent_chats_agent ON agent_chats (agent_id, updated_at DESC);"),
    M::up(
        "CREATE TABLE IF NOT EXISTS agent_messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            chat_id INTEGER NOT NULL REFERENCES agent_chats(id) ON DELETE CASCADE,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            tool_name TEXT,
            created_at INTEGER NOT NULL
        );",
    ),
    M::up("CREATE INDEX IF NOT EXISTS idx_agent_messages_chat ON agent_messages (chat_id, id);"),
    // --- basic RAG: chunk store + FTS5 index ---
    M::up(
        "CREATE TABLE IF NOT EXISTS rag_docs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source_kind TEXT NOT NULL,
            source_id TEXT NOT NULL,
            title TEXT NOT NULL,
            uri TEXT NOT NULL DEFAULT ''
        );",
    ),
    M::up(
        "CREATE TABLE IF NOT EXISTS rag_chunks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            doc_id INTEGER NOT NULL REFERENCES rag_docs(id) ON DELETE CASCADE,
            ord INTEGER NOT NULL,
            title TEXT NOT NULL DEFAULT '',
            content TEXT NOT NULL
        );",
    ),
    M::up(
        "CREATE VIRTUAL TABLE IF NOT EXISTS rag_chunks_fts USING fts5(title, content, content='rag_chunks', content_rowid='id', tokenize='unicode61');",
    ),
    M::up("INSERT INTO rag_chunks_fts(rag_chunks_fts) VALUES('rebuild');"),
    M::up(
        "CREATE TRIGGER IF NOT EXISTS rag_chunks_ai AFTER INSERT ON rag_chunks BEGIN
            INSERT INTO rag_chunks_fts(rowid, title, content) VALUES(new.id, new.title, new.content);
        END;",
    ),
    M::up(
        "CREATE TRIGGER IF NOT EXISTS rag_chunks_ad AFTER DELETE ON rag_chunks BEGIN
            INSERT INTO rag_chunks_fts(rag_chunks_fts, rowid, title, content) VALUES('delete', old.id, old.title, old.content);
        END;",
    ),
    // --- semantic RAG: per-chunk embedding vectors (NULL = not yet embedded) ---
    // Stored as little-endian f32 bytes (see `llm_client::encode_embedding`);
    // cosine similarity is computed in Rust over FTS candidates, so no
    // native vector extension is needed. `embedding_model` guards against
    // mixing vectors from different models in one ranking.
    M::up("ALTER TABLE rag_chunks ADD COLUMN embedding BLOB;"),
    M::up("ALTER TABLE rag_chunks ADD COLUMN embedding_model TEXT;"),
];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SuiteNote {
    pub id: i64,
    pub title: String,
    pub content: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub tags: String,
    pub pinned: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SuiteTodo {
    pub id: i64,
    pub task: String,
    pub completed: bool,
    pub created_at: i64,
    pub completed_at: Option<i64>,
    pub priority: i32,
    pub due_date: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SuiteDoc {
    pub id: i64,
    pub title: String,
    pub file_name: String,
    pub parsed_content: String,
    pub doc_type: String,
    pub created_at: i64,
}

fn default_entry_kind() -> String {
    "transcription".to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaginatedHistory {
    pub entries: Vec<HistoryEntry>,
    pub has_more: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum HistoryUpdatePayload {
    #[serde(rename = "added")]
    Added { entry: HistoryEntry },
    #[serde(rename = "updated")]
    Updated { entry: HistoryEntry },
    #[serde(rename = "deleted")]
    Deleted { id: i64 },
    #[serde(rename = "toggled")]
    Toggled { id: i64 },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: i64,
    pub file_name: String,
    pub timestamp: i64,
    pub saved: bool,
    pub title: String,
    pub transcription_text: String,
    pub post_processed_text: Option<String>,
    pub post_process_prompt: Option<String>,
    pub post_process_requested: bool,
    #[serde(default = "default_entry_kind")]
    pub entry_kind: String,
}

pub struct HistoryManager {
    bus: EventBus,
    paths: AppPaths,
    recordings_dir: PathBuf,
    db_path: PathBuf,
}

impl HistoryManager {
    pub fn new(paths: &AppPaths, bus: EventBus) -> Result<Self> {
        // Create recordings directory in app data dir
        let recordings_dir = paths.recordings_dir();
        let db_path = paths.data_dir.join("history.db");

        // Ensure recordings directory exists
        if !recordings_dir.exists() {
            fs::create_dir_all(&recordings_dir)?;
            debug!("Created recordings directory: {:?}", recordings_dir);
        }

        let manager = Self {
            bus,
            paths: paths.clone(),
            recordings_dir,
            db_path,
        };

        // Initialize database and run migrations synchronously
        manager.init_database()?;

        Ok(manager)
    }

    fn init_database(&self) -> Result<()> {
        info!("Initializing database at {:?}", self.db_path);

        let mut conn = Connection::open(&self.db_path)?;

        // Handle migration from tauri-plugin-sql to rusqlite_migration
        // tauri-plugin-sql used _sqlx_migrations table, rusqlite_migration uses user_version pragma
        self.migrate_from_tauri_plugin_sql(&conn)?;

        // Create migrations object and run to latest version
        let migrations = Migrations::new(MIGRATIONS.to_vec());

        // Validate migrations in debug builds
        #[cfg(debug_assertions)]
        migrations.validate().expect("Invalid migrations");

        // Get current version before migration
        let version_before: i32 =
            conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        debug!("Database version before migration: {}", version_before);

        // Apply any pending migrations
        migrations.to_latest(&mut conn)?;

        // Get version after migration
        let version_after: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

        if version_after > version_before {
            info!(
                "Database migrated from version {} to {}",
                version_before, version_after
            );
        } else {
            debug!("Database already at latest version {}", version_after);
        }

        Ok(())
    }

    /// Migrate from tauri-plugin-sql's migration tracking to rusqlite_migration's.
    /// tauri-plugin-sql used a _sqlx_migrations table, while rusqlite_migration uses
    /// SQLite's user_version pragma. This function checks if the old system was in use
    /// and sets the user_version accordingly so migrations don't re-run.
    fn migrate_from_tauri_plugin_sql(&self, conn: &Connection) -> Result<()> {
        // Check if the old _sqlx_migrations table exists
        let has_sqlx_migrations: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='_sqlx_migrations'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(false);

        if !has_sqlx_migrations {
            return Ok(());
        }

        // Check current user_version
        let current_version: i32 =
            conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

        if current_version > 0 {
            // Already migrated to rusqlite_migration system
            return Ok(());
        }

        // Get the highest version from the old migrations table
        let old_version: i32 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations WHERE success = 1",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if old_version > 0 {
            info!(
                "Migrating from tauri-plugin-sql (version {}) to rusqlite_migration",
                old_version
            );

            // Set user_version to match the old migration state
            conn.pragma_update(None, "user_version", old_version)?;

            // Optionally drop the old migrations table (keeping it doesn't hurt)
            // conn.execute("DROP TABLE IF EXISTS _sqlx_migrations", [])?;

            info!(
                "Migration tracking converted: user_version set to {}",
                old_version
            );
        }

        Ok(())
    }

    fn get_connection(&self) -> Result<Connection> {
        let conn = Connection::open(&self.db_path)?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")?;
        Ok(conn)
    }

    fn map_history_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryEntry> {
        let entry_kind: String = row
            .get("entry_kind")
            .unwrap_or_else(|_| "transcription".to_string());
        Ok(HistoryEntry {
            id: row.get("id")?,
            file_name: row.get("file_name")?,
            timestamp: row.get("timestamp")?,
            saved: row.get("saved")?,
            title: row.get("title")?,
            transcription_text: row.get("transcription_text")?,
            post_processed_text: row.get("post_processed_text")?,
            post_process_prompt: row.get("post_process_prompt")?,
            post_process_requested: row.get("post_process_requested")?,
            entry_kind,
        })
    }

    pub fn recordings_dir(&self) -> &std::path::Path {
        &self.recordings_dir
    }

    /// Save a new history entry to the database.
    /// The WAV file should already have been written to the recordings directory.
    pub fn save_entry(
        &self,
        file_name: String,
        transcription_text: String,
        post_process_requested: bool,
        post_processed_text: Option<String>,
        post_process_prompt: Option<String>,
        entry_kind: Option<&str>,
    ) -> Result<HistoryEntry> {
        let timestamp = Utc::now().timestamp();
        let title = self.format_timestamp_title(timestamp);
        let entry_kind_str = entry_kind.unwrap_or(if post_process_requested {
            "post_process"
        } else {
            "transcription"
        });

        let conn = self.get_connection()?;
        conn.execute(
            "INSERT INTO transcription_history (
                file_name,
                timestamp,
                saved,
                title,
                transcription_text,
                post_processed_text,
                post_process_prompt,
                post_process_requested,
                entry_kind
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                &file_name,
                timestamp,
                false,
                &title,
                &transcription_text,
                &post_processed_text,
                &post_process_prompt,
                post_process_requested,
                entry_kind_str,
            ],
        )?;

        let entry = HistoryEntry {
            id: conn.last_insert_rowid(),
            file_name,
            timestamp,
            saved: false,
            title,
            transcription_text,
            post_processed_text,
            post_process_prompt,
            post_process_requested,
            entry_kind: entry_kind_str.to_string(),
        };

        debug!("Saved history entry with id {}", entry.id);

        self.cleanup_old_entries()?;

        // Emit typed event for real-time frontend updates
        self.bus
            .send(AppEvent::HistoryUpdated(HistoryUpdatePayload::Added {
                entry: entry.clone(),
            }));

        Ok(entry)
    }

    /// Update an existing history entry with new transcription results (used by retry).
    pub fn update_transcription(
        &self,
        id: i64,
        transcription_text: String,
        post_processed_text: Option<String>,
        post_process_prompt: Option<String>,
    ) -> Result<HistoryEntry> {
        let conn = self.get_connection()?;
        let updated = conn.execute(
            "UPDATE transcription_history
             SET transcription_text = ?1,
                 post_processed_text = ?2,
                 post_process_prompt = ?3
             WHERE id = ?4",
            params![
                transcription_text,
                post_processed_text,
                post_process_prompt,
                id
            ],
        )?;

        if updated == 0 {
            return Err(anyhow!("History entry {} not found", id));
        }

        let entry = conn
            .query_row(
                "SELECT id, file_name, timestamp, saved, title, transcription_text, post_processed_text, post_process_prompt, post_process_requested, entry_kind
                 FROM transcription_history WHERE id = ?1",
                params![id],
                Self::map_history_entry,
            )?;

        debug!("Updated transcription for history entry {}", id);

        self.bus
            .send(AppEvent::HistoryUpdated(HistoryUpdatePayload::Updated {
                entry: entry.clone(),
            }));

        Ok(entry)
    }

    pub fn cleanup_old_entries(&self) -> Result<()> {
        let retention_period =
            crate::settings::read_settings_from(&self.paths.settings_store_path())
                .recording_retention_period;

        match retention_period {
            crate::settings::RecordingRetentionPeriod::Never => {
                // Don't delete anything
                Ok(())
            }
            crate::settings::RecordingRetentionPeriod::PreserveLimit => {
                // Use the old count-based logic with history_limit
                let limit = crate::settings::read_settings_from(&self.paths.settings_store_path())
                    .history_limit;
                self.cleanup_by_count(limit)
            }
            _ => {
                // Use time-based logic
                self.cleanup_by_time(retention_period)
            }
        }
    }

    fn delete_entries_and_files(&self, entries: &[(i64, String)]) -> Result<usize> {
        if entries.is_empty() {
            return Ok(0);
        }

        let mut conn = self.get_connection()?;
        let tx = conn.transaction()?;
        let mut deleted_count = 0;

        {
            let mut stmt = tx.prepare_cached("DELETE FROM transcription_history WHERE id = ?1")?;
            for (id, _) in entries {
                stmt.execute(params![id])?;
            }
        }

        tx.commit()?;

        for (_, file_name) in entries {
            let file_path = self.recordings_dir.join(file_name);
            if file_path.exists() {
                if let Err(e) = fs::remove_file(&file_path) {
                    error!("Failed to delete WAV file {}: {}", file_name, e);
                } else {
                    debug!("Deleted old WAV file: {}", file_name);
                    deleted_count += 1;
                }
            }
        }

        Ok(deleted_count)
    }

    fn cleanup_by_count(&self, limit: usize) -> Result<()> {
        let conn = self.get_connection()?;

        // Query only unsaved entries exceeding the retention limit using OFFSET
        let mut stmt = conn.prepare_cached(
            "SELECT id, file_name FROM transcription_history WHERE saved = 0 ORDER BY timestamp DESC LIMIT -1 OFFSET ?1"
        )?;

        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok((row.get::<_, i64>("id")?, row.get::<_, String>("file_name")?))
        })?;

        let mut entries_to_delete: Vec<(i64, String)> = Vec::new();
        for row in rows {
            entries_to_delete.push(row?);
        }

        if !entries_to_delete.is_empty() {
            let deleted_count = self.delete_entries_and_files(&entries_to_delete)?;
            if deleted_count > 0 {
                debug!("Cleaned up {} old history entries by count", deleted_count);
            }
        }

        Ok(())
    }

    fn cleanup_by_time(
        &self,
        retention_period: crate::settings::RecordingRetentionPeriod,
    ) -> Result<()> {
        let conn = self.get_connection()?;

        // Calculate cutoff timestamp (current time minus retention period)
        let now = Utc::now().timestamp();
        let cutoff_timestamp = match retention_period {
            crate::settings::RecordingRetentionPeriod::Days3 => now - (3 * 24 * 60 * 60), // 3 days in seconds
            crate::settings::RecordingRetentionPeriod::Weeks2 => now - (2 * 7 * 24 * 60 * 60), // 2 weeks in seconds
            crate::settings::RecordingRetentionPeriod::Months3 => now - (3 * 30 * 24 * 60 * 60), // 3 months in seconds (approximate)
            _ => unreachable!("Should not reach here"),
        };

        // Get all unsaved entries older than the cutoff timestamp
        let mut stmt = conn.prepare(
            "SELECT id, file_name FROM transcription_history WHERE saved = 0 AND timestamp < ?1",
        )?;

        let rows = stmt.query_map(params![cutoff_timestamp], |row| {
            Ok((row.get::<_, i64>("id")?, row.get::<_, String>("file_name")?))
        })?;

        let mut entries_to_delete: Vec<(i64, String)> = Vec::new();
        for row in rows {
            entries_to_delete.push(row?);
        }

        let deleted_count = self.delete_entries_and_files(&entries_to_delete)?;

        if deleted_count > 0 {
            debug!(
                "Cleaned up {} old history entries based on retention period",
                deleted_count
            );
        }

        Ok(())
    }

    pub async fn get_history_entries(
        &self,
        cursor: Option<i64>,
        limit: Option<usize>,
    ) -> Result<PaginatedHistory> {
        let conn = self.get_connection()?;
        let limit = limit.map(|l| l.min(100));

        let mut entries: Vec<HistoryEntry> = match (cursor, limit) {
            (Some(cursor_id), Some(lim)) => {
                let fetch_count = (lim + 1) as i64;
                let mut stmt = conn.prepare(
                    "SELECT id, file_name, timestamp, saved, title, transcription_text, post_processed_text, post_process_prompt, post_process_requested, entry_kind
                     FROM transcription_history
                     WHERE id < ?1
                     ORDER BY id DESC
                     LIMIT ?2",
                )?;
                let result = stmt
                    .query_map(params![cursor_id, fetch_count], Self::map_history_entry)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                result
            }
            (None, Some(lim)) => {
                let fetch_count = (lim + 1) as i64;
                let mut stmt = conn.prepare(
                    "SELECT id, file_name, timestamp, saved, title, transcription_text, post_processed_text, post_process_prompt, post_process_requested, entry_kind
                     FROM transcription_history
                     ORDER BY id DESC
                     LIMIT ?1",
                )?;
                let result = stmt
                    .query_map(params![fetch_count], Self::map_history_entry)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                result
            }
            (_, None) => {
                let mut stmt = conn.prepare(
                    "SELECT id, file_name, timestamp, saved, title, transcription_text, post_processed_text, post_process_prompt, post_process_requested, entry_kind
                     FROM transcription_history
                     ORDER BY id DESC",
                )?;
                let result = stmt
                    .query_map([], Self::map_history_entry)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                result
            }
        };

        let has_more = limit.is_some_and(|lim| entries.len() > lim);
        if has_more {
            entries.pop();
        }

        Ok(PaginatedHistory { entries, has_more })
    }

    #[cfg(test)]
    fn get_latest_entry_with_conn(conn: &Connection) -> Result<Option<HistoryEntry>> {
        let mut stmt = conn.prepare(
            "SELECT
                id,
                file_name,
                timestamp,
                saved,
                title,
                transcription_text,
                post_processed_text,
                post_process_prompt,
                post_process_requested,
                entry_kind
             FROM transcription_history
             ORDER BY timestamp DESC
             LIMIT 1",
        )?;

        let entry = stmt.query_row([], Self::map_history_entry).optional()?;
        Ok(entry)
    }

    /// Get the latest entry with non-empty transcription text.
    pub fn get_latest_completed_entry(&self) -> Result<Option<HistoryEntry>> {
        let conn = self.get_connection()?;
        Self::get_latest_completed_entry_with_conn(&conn)
    }

    fn get_latest_completed_entry_with_conn(conn: &Connection) -> Result<Option<HistoryEntry>> {
        let mut stmt = conn.prepare(
            "SELECT
                id,
                file_name,
                timestamp,
                saved,
                title,
                transcription_text,
                post_processed_text,
                post_process_prompt,
                post_process_requested,
                entry_kind
             FROM transcription_history
             WHERE transcription_text != ''
             ORDER BY timestamp DESC
             LIMIT 1",
        )?;

        let entry = stmt.query_row([], Self::map_history_entry).optional()?;
        Ok(entry)
    }

    pub async fn toggle_saved_status(&self, id: i64) -> Result<()> {
        let conn = self.get_connection()?;

        // Get current saved status
        let current_saved: bool = conn.query_row(
            "SELECT saved FROM transcription_history WHERE id = ?1",
            params![id],
            |row| row.get("saved"),
        )?;

        let new_saved = !current_saved;

        conn.execute(
            "UPDATE transcription_history SET saved = ?1 WHERE id = ?2",
            params![new_saved, id],
        )?;

        debug!("Toggled saved status for entry {}: {}", id, new_saved);

        // Emit history updated event
        self.bus
            .send(AppEvent::HistoryUpdated(HistoryUpdatePayload::Toggled {
                id,
            }));

        Ok(())
    }

    pub fn get_audio_file_path(&self, file_name: &str) -> PathBuf {
        self.recordings_dir.join(file_name)
    }

    pub async fn get_entry_by_id(&self, id: i64) -> Result<Option<HistoryEntry>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT
                id,
                file_name,
                timestamp,
                saved,
                title,
                transcription_text,
                post_processed_text,
                post_process_prompt,
                post_process_requested,
                entry_kind
             FROM transcription_history
             WHERE id = ?1",
        )?;

        let entry = stmt.query_row([id], Self::map_history_entry).optional()?;

        Ok(entry)
    }

    pub async fn delete_entry(&self, id: i64) -> Result<()> {
        let conn = self.get_connection()?;

        // Get the entry to find the file name
        if let Some(entry) = self.get_entry_by_id(id).await? {
            // Delete the audio file first
            let file_path = self.get_audio_file_path(&entry.file_name);
            if file_path.exists() {
                if let Err(e) = fs::remove_file(&file_path) {
                    error!("Failed to delete audio file {}: {}", entry.file_name, e);
                    // Continue with database deletion even if file deletion fails
                }
            }
        }

        // Delete from database
        conn.execute(
            "DELETE FROM transcription_history WHERE id = ?1",
            params![id],
        )?;

        debug!("Deleted history entry with id: {}", id);

        // Emit history updated event
        self.bus
            .send(AppEvent::HistoryUpdated(HistoryUpdatePayload::Deleted {
                id,
            }));

        Ok(())
    }

    fn format_timestamp_title(&self, timestamp: i64) -> String {
        if let Some(utc_datetime) = DateTime::from_timestamp(timestamp, 0) {
            // Convert UTC to local timezone
            let local_datetime = utc_datetime.with_timezone(&Local);
            local_datetime.format("%B %e, %Y - %l:%M%p").to_string()
        } else {
            format!("Recording {}", timestamp)
        }
    }

    // ========================================================================
    // Suite Notes Methods
    // ========================================================================

    pub fn save_note(
        &self,
        title: String,
        content: String,
        tags: Option<String>,
    ) -> Result<SuiteNote> {
        let conn = self.get_connection()?;
        let now = Utc::now().timestamp();
        let tags_str = tags.unwrap_or_default();
        let title_clean = if title.trim().is_empty() {
            "Untitled Note".to_string()
        } else {
            title.trim().to_string()
        };

        conn.execute(
            "INSERT INTO suite_notes (title, content, created_at, updated_at, tags, pinned)
             VALUES (?1, ?2, ?3, ?4, ?5, 0)",
            params![title_clean, content, now, now, tags_str],
        )?;

        let id = conn.last_insert_rowid();
        Ok(SuiteNote {
            id,
            title: title_clean,
            content,
            created_at: now,
            updated_at: now,
            tags: tags_str,
            pinned: false,
        })
    }

    pub fn update_note(
        &self,
        id: i64,
        title: String,
        content: String,
        tags: Option<String>,
    ) -> Result<()> {
        let conn = self.get_connection()?;
        let now = Utc::now().timestamp();
        let tags_str = tags.unwrap_or_default();
        let title_clean = if title.trim().is_empty() {
            "Untitled Note".to_string()
        } else {
            title.trim().to_string()
        };

        conn.execute(
            "UPDATE suite_notes SET title = ?1, content = ?2, updated_at = ?3, tags = ?4 WHERE id = ?5",
            params![title_clean, content, now, tags_str, id],
        )?;
        Ok(())
    }

    pub fn delete_note(&self, id: i64) -> Result<()> {
        let conn = self.get_connection()?;
        conn.execute("DELETE FROM suite_notes WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn toggle_pin_note(&self, id: i64) -> Result<bool> {
        let conn = self.get_connection()?;
        let current: bool = conn.query_row(
            "SELECT pinned FROM suite_notes WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        let new_state = !current;
        conn.execute(
            "UPDATE suite_notes SET pinned = ?1 WHERE id = ?2",
            params![new_state, id],
        )?;
        Ok(new_state)
    }

    pub fn list_notes(&self, query: Option<&str>) -> Result<Vec<SuiteNote>> {
        let conn = self.get_connection()?;
        let mut notes = Vec::new();
        match query {
            Some(q) if !q.trim().is_empty() => {
                let pattern = format!("%{}%", q.trim());
                let mut stmt = conn.prepare(
                    "SELECT id, title, content, created_at, updated_at, tags, pinned
                     FROM suite_notes
                     WHERE title LIKE ?1 OR content LIKE ?1 OR tags LIKE ?1
                     ORDER BY pinned DESC, updated_at DESC",
                )?;
                let rows = stmt.query_map(params![pattern], |row| {
                    Ok(SuiteNote {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        content: row.get(2)?,
                        created_at: row.get(3)?,
                        updated_at: row.get(4)?,
                        tags: row.get(5)?,
                        pinned: row.get(6)?,
                    })
                })?;
                for r in rows {
                    notes.push(r?);
                }
                Ok(notes)
            }
            _ => {
                let mut stmt = conn.prepare(
                    "SELECT id, title, content, created_at, updated_at, tags, pinned
                     FROM suite_notes
                     ORDER BY pinned DESC, updated_at DESC",
                )?;
                let rows = stmt.query_map([], |row| {
                    Ok(SuiteNote {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        content: row.get(2)?,
                        created_at: row.get(3)?,
                        updated_at: row.get(4)?,
                        tags: row.get(5)?,
                        pinned: row.get(6)?,
                    })
                })?;
                for r in rows {
                    notes.push(r?);
                }
                Ok(notes)
            }
        }
    }

    // ========================================================================
    // Suite Todos Methods
    // ========================================================================

    pub fn save_todo(
        &self,
        task: String,
        priority: i32,
        due_date: Option<i64>,
    ) -> Result<SuiteTodo> {
        let conn = self.get_connection()?;
        let now = Utc::now().timestamp();
        let task_clean = task.trim().to_string();

        conn.execute(
            "INSERT INTO suite_todos (task, completed, created_at, priority, due_date)
             VALUES (?1, 0, ?2, ?3, ?4)",
            params![task_clean, now, priority, due_date],
        )?;

        let id = conn.last_insert_rowid();
        Ok(SuiteTodo {
            id,
            task: task_clean,
            completed: false,
            created_at: now,
            completed_at: None,
            priority,
            due_date,
        })
    }

    pub fn toggle_todo(&self, id: i64) -> Result<bool> {
        let conn = self.get_connection()?;
        let current: bool = conn.query_row(
            "SELECT completed FROM suite_todos WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        let new_state = !current;
        let now = if new_state {
            Some(Utc::now().timestamp())
        } else {
            None
        };
        conn.execute(
            "UPDATE suite_todos SET completed = ?1, completed_at = ?2 WHERE id = ?3",
            params![new_state, now, id],
        )?;
        Ok(new_state)
    }

    pub fn delete_todo(&self, id: i64) -> Result<()> {
        let conn = self.get_connection()?;
        conn.execute("DELETE FROM suite_todos WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn list_todos(&self, include_completed: bool) -> Result<Vec<SuiteTodo>> {
        let conn = self.get_connection()?;
        let mut todos = Vec::new();
        let sql = if include_completed {
            "SELECT id, task, completed, created_at, completed_at, priority, due_date
             FROM suite_todos
             ORDER BY completed ASC, priority DESC, created_at DESC"
        } else {
            "SELECT id, task, completed, created_at, completed_at, priority, due_date
             FROM suite_todos
             WHERE completed = 0
             ORDER BY priority DESC, created_at DESC"
        };
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(SuiteTodo {
                id: row.get(0)?,
                task: row.get(1)?,
                completed: row.get(2)?,
                created_at: row.get(3)?,
                completed_at: row.get(4)?,
                priority: row.get(5)?,
                due_date: row.get(6)?,
            })
        })?;
        for r in rows {
            todos.push(r?);
        }
        Ok(todos)
    }

    pub fn clear_completed_todos(&self) -> Result<usize> {
        let conn = self.get_connection()?;
        let affected = conn.execute("DELETE FROM suite_todos WHERE completed = 1", [])?;
        Ok(affected)
    }

    // ========================================================================
    // Suite Docs Methods
    // ========================================================================

    pub fn save_doc(
        &self,
        title: String,
        file_name: String,
        parsed_content: String,
        doc_type: String,
    ) -> Result<SuiteDoc> {
        let conn = self.get_connection()?;
        let now = Utc::now().timestamp();
        conn.execute(
            "INSERT INTO suite_docs (title, file_name, parsed_content, doc_type, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![title, file_name, parsed_content, doc_type, now],
        )?;
        let id = conn.last_insert_rowid();
        Ok(SuiteDoc {
            id,
            title,
            file_name,
            parsed_content,
            doc_type,
            created_at: now,
        })
    }

    pub fn list_docs(&self) -> Result<Vec<SuiteDoc>> {
        let conn = self.get_connection()?;
        let mut docs = Vec::new();
        let mut stmt = conn.prepare(
            "SELECT id, title, file_name, parsed_content, doc_type, created_at
             FROM suite_docs
             ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SuiteDoc {
                id: row.get(0)?,
                title: row.get(1)?,
                file_name: row.get(2)?,
                parsed_content: row.get(3)?,
                doc_type: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;
        for r in rows {
            docs.push(r?);
        }
        Ok(docs)
    }

    pub fn delete_doc(&self, id: i64) -> Result<()> {
        let conn = self.get_connection()?;
        conn.execute("DELETE FROM suite_docs WHERE id = ?1", params![id])?;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentChat {
    pub id: i64,
    pub agent_id: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentMessage {
    pub id: i64,
    pub chat_id: i64,
    pub role: String,
    pub content: String,
    pub tool_name: Option<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RagHit {
    pub chunk_id: i64,
    pub doc_id: i64,
    pub title: String,
    pub uri: String,
    pub snippet: String,
    pub rank: f64,
}

/// One chunk row needed for vector backfill: id + text + current model tag.
#[derive(Clone, Debug)]
pub struct RagChunkForEmbedding {
    pub chunk_id: i64,
    pub content: String,
}

impl HistoryManager {
    /// Create a chat for `agent_id` and return it.
    pub fn create_agent_chat(&self, agent_id: &str, title: &str) -> Result<AgentChat> {
        let conn = self.get_connection()?;
        let now = Utc::now().timestamp();
        let title_clean = if title.trim().is_empty() {
            "New chat".to_string()
        } else {
            title.trim().chars().take(80).collect()
        };
        conn.execute(
            "INSERT INTO agent_chats (agent_id, title, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![agent_id, title_clean, now, now],
        )?;
        let id = conn.last_insert_rowid();
        Ok(AgentChat {
            id,
            agent_id: agent_id.to_string(),
            title: title_clean,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn list_agent_chats(&self, agent_id: Option<&str>) -> Result<Vec<AgentChat>> {
        let conn = self.get_connection()?;
        let sql = match agent_id {
            Some(_) => {
                "SELECT id, agent_id, title, created_at, updated_at FROM agent_chats
                 WHERE agent_id = ?1 ORDER BY updated_at DESC LIMIT 100"
            }
            None => {
                "SELECT id, agent_id, title, created_at, updated_at FROM agent_chats
                 ORDER BY updated_at DESC LIMIT 100"
            }
        };
        let mut stmt = conn.prepare(sql)?;
        let rows: Vec<AgentChat> = match agent_id {
            Some(id) => stmt
                .query_map(params![id], row_agent_chat)?
                .collect::<Result<Vec<_>, _>>()?,
            None => stmt
                .query_map([], row_agent_chat)?
                .collect::<Result<Vec<_>, _>>()?,
        };
        return Ok(rows);

        fn row_agent_chat(row: &rusqlite::Row) -> rusqlite::Result<AgentChat> {
            Ok(AgentChat {
                id: row.get(0)?,
                agent_id: row.get(1)?,
                title: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            })
        }
    }

    /// Rename a chat (title trimmed to 80 chars; empty keeps the old title).
    pub fn rename_agent_chat(&self, chat_id: i64, title: &str) -> Result<()> {
        let title = title.trim();
        if title.is_empty() {
            return Ok(());
        }
        let title: String = title.chars().take(80).collect();
        let conn = self.get_connection()?;
        conn.execute(
            "UPDATE agent_chats SET title = ?1, updated_at = ?2 WHERE id = ?3",
            params![title, Utc::now().timestamp(), chat_id],
        )?;
        Ok(())
    }

    pub fn delete_agent_chat(&self, chat_id: i64) -> Result<()> {
        let conn = self.get_connection()?;
        conn.execute(
            "DELETE FROM agent_messages WHERE chat_id = ?1",
            params![chat_id],
        )?;
        conn.execute("DELETE FROM agent_chats WHERE id = ?1", params![chat_id])?;
        Ok(())
    }

    /// Delete chats older than `retention_days` (0 disables cleanup).
    pub fn cleanup_agent_chats(&self, retention_days: u32) -> Result<usize> {
        if retention_days == 0 {
            return Ok(0);
        }
        let cutoff = Utc::now().timestamp() - i64::from(retention_days) * 86_400;
        let conn = self.get_connection()?;
        let stale: Vec<i64> = conn
            .prepare("SELECT id FROM agent_chats WHERE updated_at < ?1")?
            .query_map(params![cutoff], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        for id in &stale {
            conn.execute("DELETE FROM agent_messages WHERE chat_id = ?1", params![id])?;
            conn.execute("DELETE FROM agent_chats WHERE id = ?1", params![id])?;
        }
        Ok(stale.len())
    }

    /// Append a message and bump the chat's `updated_at`. Returns the row.
    pub fn append_agent_message(
        &self,
        chat_id: i64,
        role: &str,
        content: &str,
        tool_name: Option<&str>,
    ) -> Result<AgentMessage> {
        let conn = self.get_connection()?;
        let now = Utc::now().timestamp();
        conn.execute(
            "INSERT INTO agent_messages (chat_id, role, content, tool_name, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![chat_id, role, content, tool_name, now],
        )?;
        let id = conn.last_insert_rowid();
        conn.execute(
            "UPDATE agent_chats SET updated_at = ?1 WHERE id = ?2",
            params![now, chat_id],
        )?;
        Ok(AgentMessage {
            id,
            chat_id,
            role: role.to_string(),
            content: content.to_string(),
            tool_name: tool_name.map(str::to_string),
            created_at: now,
        })
    }

    pub fn list_agent_messages(&self, chat_id: i64) -> Result<Vec<AgentMessage>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, chat_id, role, content, tool_name, created_at
             FROM agent_messages WHERE chat_id = ?1 ORDER BY id ASC LIMIT 500",
        )?;
        let rows = stmt.query_map(params![chat_id], |row| {
            Ok(AgentMessage {
                id: row.get(0)?,
                chat_id: row.get(1)?,
                role: row.get(2)?,
                content: row.get(3)?,
                tool_name: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Replace every chunk of one indexed document (delete + reinsert keeps
    /// the FTS index in sync through the `rag_chunks_*` triggers).
    pub fn index_rag_document(
        &self,
        source_kind: &str,
        source_id: &str,
        title: &str,
        uri: &str,
        chunks: &[String],
    ) -> Result<i64> {
        let conn = self.get_connection()?;
        let existing: Option<i64> = conn
            .query_row(
                "SELECT id FROM rag_docs WHERE source_kind = ?1 AND source_id = ?2",
                params![source_kind, source_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(doc_id) = existing {
            conn.execute("DELETE FROM rag_chunks WHERE doc_id = ?1", params![doc_id])?;
            conn.execute(
                "UPDATE rag_docs SET title = ?1, uri = ?2 WHERE id = ?3",
                params![title, uri, doc_id],
            )?;
            insert_chunks(&conn, doc_id, title, chunks)?;
            return Ok(doc_id);
        }
        conn.execute(
            "INSERT INTO rag_docs (source_kind, source_id, title, uri) VALUES (?1, ?2, ?3, ?4)",
            params![source_kind, source_id, title, uri],
        )?;
        let doc_id = conn.last_insert_rowid();
        insert_chunks(&conn, doc_id, title, chunks)?;
        return Ok(doc_id);

        fn insert_chunks(
            conn: &Connection,
            doc_id: i64,
            title: &str,
            chunks: &[String],
        ) -> Result<()> {
            for (ord, chunk) in chunks.iter().enumerate() {
                conn.execute(
                    "INSERT INTO rag_chunks (doc_id, ord, title, content) VALUES (?1, ?2, ?3, ?4)",
                    params![doc_id, ord as i64, title, chunk],
                )?;
            }
            Ok(())
        }
    }

    /// Drop the whole RAG index (documents + chunks; FTS follows via trigger).
    pub fn clear_rag_index(&self) -> Result<()> {
        let conn = self.get_connection()?;
        conn.execute("DELETE FROM rag_chunks", [])?;
        conn.execute("DELETE FROM rag_docs", [])?;
        Ok(())
    }

    /// Chunks still missing an embedding for `model` (or tagged with a
    /// different model), oldest first, capped for bounded background jobs.
    pub fn rag_chunks_missing_embedding(
        &self,
        model: &str,
        limit: u32,
    ) -> Result<Vec<RagChunkForEmbedding>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, content FROM rag_chunks
             WHERE embedding IS NULL OR embedding_model IS NULL OR embedding_model != ?1
             ORDER BY id ASC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![model, limit.max(1) as i64], |row| {
            Ok(RagChunkForEmbedding {
                chunk_id: row.get(0)?,
                content: row.get(1)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Store one chunk embedding (bytes from
    /// [`crate::llm_client::encode_embedding`]) tagged with its model.
    pub fn store_chunk_embedding(
        &self,
        chunk_id: i64,
        model: &str,
        embedding: &[u8],
    ) -> Result<()> {
        let conn = self.get_connection()?;
        conn.execute(
            "UPDATE rag_chunks SET embedding = ?1, embedding_model = ?2 WHERE id = ?3",
            params![embedding, model, chunk_id],
        )?;
        Ok(())
    }

    /// Counts for the Agents-page status line: (total chunks, embedded with
    /// `model`). `model` empty/`None` counts any embedding.
    pub fn rag_embedding_stats(&self, model: Option<&str>) -> (u64, u64) {
        let conn = match self.get_connection() {
            Ok(conn) => conn,
            Err(_) => return (0, 0),
        };
        let total: u64 = conn
            .query_row("SELECT count(*) FROM rag_chunks", [], |row| row.get(0))
            .unwrap_or(0);
        let embedded: u64 = match model {
            Some(model) if !model.trim().is_empty() => conn
                .query_row(
                    "SELECT count(*) FROM rag_chunks WHERE embedding IS NOT NULL AND embedding_model = ?1",
                    params![model],
                    |row| row.get(0),
                )
                .unwrap_or(0),
            _ => conn
                .query_row(
                    "SELECT count(*) FROM rag_chunks WHERE embedding IS NOT NULL",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or(0),
        };
        (total, embedded)
    }

    /// Hybrid retrieval: BM25 full-text search over indexed chunks. Never fails the chat when
    /// the index is missing/empty: returns an empty vec instead.
    pub fn rag_search(&self, query: &str, top_k: u32) -> Vec<RagHit> {
        let query = query.trim();
        if query.is_empty() {
            return Vec::new();
        }
        let conn = match self.get_connection() {
            Ok(conn) => conn,
            Err(_) => return Vec::new(),
        };
        // Quote each term so user input cannot break the MATCH syntax.
        let terms: Vec<String> = query
            .split_whitespace()
            .take(10)
            .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
            .collect();
        if terms.is_empty() {
            return Vec::new();
        }
        let match_expr = terms.join(" OR ");
        let limit = top_k.clamp(1, 10) as i64;
        let mut stmt = match conn.prepare(
            "SELECT c.id, c.doc_id, d.title, d.uri,
                    snippet(rag_chunks_fts, 1, '[', ']', '…', 24),
                    bm25(rag_chunks_fts)
             FROM rag_chunks_fts
             JOIN rag_chunks c ON c.id = rag_chunks_fts.rowid
             JOIN rag_docs d ON d.id = c.doc_id
             WHERE rag_chunks_fts MATCH ?1
             ORDER BY bm25(rag_chunks_fts) LIMIT ?2",
        ) {
            Ok(stmt) => stmt,
            Err(_) => return Vec::new(),
        };
        let rows = match stmt.query_map(params![match_expr, limit], |row| {
            Ok(RagHit {
                chunk_id: row.get(0)?,
                doc_id: row.get(1)?,
                title: row.get(2)?,
                uri: row.get(3)?,
                snippet: row.get(4)?,
                rank: row.get(5)?,
            })
        }) {
            Ok(rows) => rows,
            Err(_) => return Vec::new(),
        };
        rows.filter_map(|r| r.ok()).collect()
    }

    /// Fetch stored embeddings for `chunk_ids` tagged with `model`, as
    /// `(chunk_id, vector)`. Chunks without a matching embedding are absent;
    /// callers fuse with the BM25 ranking (RRF) instead of failing.
    pub fn rag_chunk_embeddings(&self, chunk_ids: &[i64], model: &str) -> Vec<(i64, Vec<f32>)> {
        if chunk_ids.is_empty() {
            return Vec::new();
        }
        let conn = match self.get_connection() {
            Ok(conn) => conn,
            Err(_) => return Vec::new(),
        };
        let placeholders: Vec<String> = chunk_ids.iter().map(|_| "?".to_string()).collect();
        let sql = format!(
            "SELECT id, embedding FROM rag_chunks WHERE embedding_model = ?1 AND id IN ({})",
            placeholders.join(",")
        );
        let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(chunk_ids.len() + 1);
        params.push(&model);
        for id in chunk_ids {
            params.push(id);
        }
        let mut stmt = match conn.prepare(&sql) {
            Ok(stmt) => stmt,
            Err(_) => return Vec::new(),
        };
        let rows = match stmt.query_map(params.as_slice(), |row| {
            let id: i64 = row.get(0)?;
            let blob: Vec<u8> = row.get(1)?;
            Ok((id, blob))
        }) {
            Ok(rows) => rows,
            Err(_) => return Vec::new(),
        };
        rows.filter_map(|row| row.ok())
            .filter_map(|(id, blob)| {
                crate::llm_client::decode_embedding(&blob).map(|vector| (id, vector))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};

    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch(
            "CREATE TABLE transcription_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_name TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                saved BOOLEAN NOT NULL DEFAULT 0,
                title TEXT NOT NULL,
                transcription_text TEXT NOT NULL,
                post_processed_text TEXT,
                post_process_prompt TEXT,
                post_process_requested BOOLEAN NOT NULL DEFAULT 0,
                entry_kind TEXT NOT NULL DEFAULT 'transcription'
            );",
        )
        .expect("create transcription_history table");
        conn
    }

    fn insert_entry(conn: &Connection, timestamp: i64, text: &str, post_processed: Option<&str>) {
        insert_entry_with_kind(conn, timestamp, text, post_processed, "transcription");
    }

    fn insert_entry_with_kind(
        conn: &Connection,
        timestamp: i64,
        text: &str,
        post_processed: Option<&str>,
        kind: &str,
    ) {
        conn.execute(
            "INSERT INTO transcription_history (
                file_name,
                timestamp,
                saved,
                title,
                transcription_text,
                post_processed_text,
                post_process_prompt,
                post_process_requested,
                entry_kind
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                format!("Otush-{}.wav", timestamp),
                timestamp,
                false,
                format!("Recording {}", timestamp),
                text,
                post_processed,
                Option::<String>::None,
                false,
                kind,
            ],
        )
        .expect("insert history entry");
    }

    #[test]
    fn get_latest_entry_returns_none_when_empty() {
        let conn = setup_conn();
        let entry = HistoryManager::get_latest_entry_with_conn(&conn).expect("fetch latest entry");
        assert!(entry.is_none());
    }

    #[test]
    fn get_latest_entry_returns_newest_entry() {
        let conn = setup_conn();
        insert_entry(&conn, 100, "first", None);
        insert_entry_with_kind(&conn, 200, "second", Some("processed"), "post_process");

        let entry = HistoryManager::get_latest_entry_with_conn(&conn)
            .expect("fetch latest entry")
            .expect("entry exists");

        assert_eq!(entry.timestamp, 200);
        assert_eq!(entry.transcription_text, "second");
        assert_eq!(entry.post_processed_text.as_deref(), Some("processed"));
        assert_eq!(entry.entry_kind, "post_process");
    }

    #[test]
    fn get_latest_completed_entry_skips_empty_entries() {
        let conn = setup_conn();
        insert_entry(&conn, 100, "completed", None);
        insert_entry(&conn, 200, "", None);

        let entry = HistoryManager::get_latest_completed_entry_with_conn(&conn)
            .expect("fetch latest completed entry")
            .expect("completed entry exists");

        assert_eq!(entry.timestamp, 100);
        assert_eq!(entry.transcription_text, "completed");
        assert_eq!(entry.entry_kind, "transcription");
    }

    #[test]
    fn test_suite_migrations_and_crud() {
        let mut conn = Connection::open_in_memory().expect("open in-memory db");
        let migrations = Migrations::new(MIGRATIONS.to_vec());
        migrations
            .to_latest(&mut conn)
            .expect("apply migrations to latest");

        // Test notes
        conn.execute(
            "INSERT INTO suite_notes (title, content, created_at, updated_at, tags, pinned)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                "Meeting Idea",
                "Build GNOME suite",
                1000,
                1000,
                "work",
                true
            ],
        )
        .expect("insert note");

        let note_count: i64 = conn
            .query_row("SELECT count(*) FROM suite_notes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(note_count, 1);

        // Test todos
        conn.execute(
            "INSERT INTO suite_todos (task, completed, created_at, priority, due_date)
             VALUES (?1, 0, ?2, 1, NULL)",
            params!["Review pull request", 1000],
        )
        .expect("insert todo");

        let todo_count: i64 = conn
            .query_row("SELECT count(*) FROM suite_todos", [], |r| r.get(0))
            .unwrap();
        assert_eq!(todo_count, 1);

        // Test docs
        conn.execute(
            "INSERT INTO suite_docs (title, file_name, parsed_content, doc_type, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                "Architecture",
                "arch.pdf",
                "# Markdown Content",
                "pdf",
                1000
            ],
        )
        .expect("insert doc");

        let doc_count: i64 = conn
            .query_row("SELECT count(*) FROM suite_docs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(doc_count, 1);
    }

    #[test]
    fn agent_chats_crud_and_rag_round_trip() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let paths = AppPaths {
            data_dir: temp_dir.path().to_path_buf(),
            resource_dir: temp_dir.path().to_path_buf(),
            log_dir: temp_dir.path().to_path_buf(),
        };
        let manager = HistoryManager::new(&paths, EventBus::new()).expect("manager");

        let chat = manager
            .create_agent_chat("chat-assistant", "Test chat")
            .expect("create chat");
        manager
            .append_agent_message(chat.id, "user", "What is Vulkan?", None)
            .expect("append user");
        manager
            .append_agent_message(chat.id, "assistant", "A graphics API.", None)
            .expect("append assistant");
        let messages = manager.list_agent_messages(chat.id).expect("list");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");

        // FTS5 must be live: index → search → hit with snippet.
        manager
            .index_rag_document(
                "suite_doc",
                "1",
                "GPU Guide",
                "otush://suite_doc/1",
                &["Vulkan is a low-overhead graphics API for GPUs.".to_string()],
            )
            .expect("index doc");
        let hits = manager.rag_search("Vulkan graphics", 4);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "GPU Guide");
        assert!(hits[0].snippet.contains("Vulkan"));

        // Re-index replaces chunks; empty query returns nothing.
        manager
            .index_rag_document(
                "suite_doc",
                "1",
                "GPU Guide",
                "otush://suite_doc/1",
                &["Unrelated pottery content.".to_string()],
            )
            .expect("re-index");
        assert!(manager.rag_search("Vulkan graphics", 4).is_empty());
        assert!(manager.rag_search("   ", 4).is_empty());

        manager.delete_agent_chat(chat.id).expect("delete chat");
        assert!(manager
            .list_agent_messages(chat.id)
            .expect("list")
            .is_empty());
    }

    #[test]
    fn agent_chat_rename_trims_and_ignores_empty() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let paths = AppPaths {
            data_dir: temp_dir.path().to_path_buf(),
            resource_dir: temp_dir.path().to_path_buf(),
            log_dir: temp_dir.path().to_path_buf(),
        };
        let manager = HistoryManager::new(&paths, EventBus::new()).expect("manager");
        let chat = manager
            .create_agent_chat("chat-assistant", "Old title")
            .expect("create chat");
        manager
            .rename_agent_chat(chat.id, "  New title  ")
            .expect("rename");
        let chats = manager.list_agent_chats(None).expect("list");
        assert_eq!(chats[0].title, "New title");
        manager
            .rename_agent_chat(chat.id, "   ")
            .expect("empty rename");
        let chats = manager.list_agent_chats(None).expect("list");
        assert_eq!(chats[0].title, "New title");
    }

    #[test]
    fn rag_embedding_backfill_and_hybrid_lookup() {
        use crate::llm_client;
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let paths = AppPaths {
            data_dir: temp_dir.path().to_path_buf(),
            resource_dir: temp_dir.path().to_path_buf(),
            log_dir: temp_dir.path().to_path_buf(),
        };
        let manager = HistoryManager::new(&paths, EventBus::new()).expect("manager");
        manager
            .index_rag_document(
                "suite_doc",
                "9",
                "Vector Doc",
                "otush://suite_doc/9",
                &[
                    "Rust ownership moves values between scopes.".to_string(),
                    "Sourdough starter needs daily feeding.".to_string(),
                ],
            )
            .expect("index doc");

        // Fresh chunks miss embeddings for the model.
        let missing = manager
            .rag_chunks_missing_embedding("test-model", 100)
            .expect("missing");
        assert_eq!(missing.len(), 2);
        let (total, embedded) = manager.rag_embedding_stats(Some("test-model"));
        assert_eq!((total, embedded), (2, 0));

        // Store one vector; stats and filtered lookup follow.
        let bytes = llm_client::encode_embedding(&[1.0, 0.0, 0.0]);
        manager
            .store_chunk_embedding(missing[0].chunk_id, "test-model", &bytes)
            .expect("store");
        let (_, embedded) = manager.rag_embedding_stats(Some("test-model"));
        assert_eq!(embedded, 1);
        let missing = manager
            .rag_chunks_missing_embedding("test-model", 100)
            .expect("missing");
        assert_eq!(missing.len(), 1);
        // Other models don't see this vector (no cross-model mixing).
        assert!(manager
            .rag_chunk_embeddings(&[missing[0].chunk_id], "other-model")
            .is_empty());
        let got = manager.rag_chunk_embeddings(&[missing[0].chunk_id - 1], "test-model");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].1, vec![1.0f32, 0.0, 0.0]);
    }
}
