//! Host abstraction for the backend, plus the backend → UI event bus.
//!
//! The Rust core (managers, audio pipeline, transcription, downloads, history)
//! never talks to a GUI toolkit. It talks to [`AppContext`] — paths, settings,
//! shared managers and the [`EventBus`] — and the GNOME (GTK4/libadwaita) shell
//! subscribes to [`AppEvent`]s to update widgets.

use crate::managers::audio::AudioRecordingManager;
use crate::managers::history::{HistoryManager, HistoryUpdatePayload};
use crate::managers::model::ModelManager;
use crate::managers::transcription::{
    ModelStateEvent, StreamPhaseEvent, StreamTextEvent, TranscriptionManager,
};
use crate::settings::{AppSettings, Theme};
use crate::TranscriptionCoordinator;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// App-wide filesystem locations, portable-aware.
#[derive(Clone, Debug)]
pub struct AppPaths {
    /// User data root (settings, models, history DB, recordings, logs).
    pub data_dir: PathBuf,
    /// Bundled resources (sounds, tray icons, default settings, VAD model).
    pub resource_dir: PathBuf,
    /// Log directory.
    pub log_dir: PathBuf,
}

impl AppPaths {
    /// Resolve app dirs. `data_dir` honors portable mode; otherwise it is
    /// `$XDG_DATA_HOME/com.clusterat.otush`.
    pub fn resolve() -> Self {
        let data_dir = crate::portable::data_root();
        let log_dir = crate::portable::log_root(&data_dir);
        let resource_dir = crate::resources::resource_dir();
        Self {
            data_dir,
            resource_dir,
            log_dir,
        }
    }

    /// Directory where downloaded models live (and are extracted).
    pub fn models_dir(&self) -> PathBuf {
        self.data_dir.join("models")
    }

    /// Directory where recordings are archived.
    pub fn recordings_dir(&self) -> PathBuf {
        self.data_dir.join("recordings")
    }

    /// Absolute path of the JSON settings store (`settings_store.json`).
    pub fn settings_store_path(&self) -> PathBuf {
        self.data_dir.join(crate::settings::SETTINGS_STORE_PATH)
    }

    /// Ensure the data + log directories exist.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(&self.log_dir)?;
        Ok(())
    }
}

/// Recording error surfaced to the UI (toast/banner).
#[derive(Clone, Debug, serde::Serialize)]
pub struct RecordingErrorEvent {
    pub error_type: String,
    pub detail: Option<String>,
}

/// Model download progress event surfaced to the UI.
#[derive(Clone, Debug)]
pub struct ModelDownloadProgressEvent {
    pub url: String,
    pub filename: String,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub percentage: f64,
    pub speed_mb_s: f64,
}

/// Model download completion event surfaced to the UI.
#[derive(Clone, Debug)]
pub struct ModelDownloadFinishedEvent {
    pub filename: String,
    pub success: bool,
    pub error: Option<String>,
}

/// One agent chat message surfaced to the UI.
#[derive(Clone, Debug)]
pub struct AgentMessageEvent {
    pub id: i64,
    pub role: String,
    pub content: String,
    pub tool_name: Option<String>,
}

/// Which TTS speak mode produced the audio: long-form reading (notes, web,
/// docs) or short chat answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum TtsSource {
    Reader,
    Chat,
}

/// Playback state of the TTS engine, surfaced to the UI (mini-player, chat
/// speaker buttons).
#[derive(Clone, Debug)]
pub enum TtsState {
    /// Synthesis/playback started for `total` chunks.
    Started {
        total: usize,
        source: TtsSource,
    },
    /// Chunk `done` of `total` finished (1-based `done`).
    ChunkProgress {
        done: usize,
        total: usize,
        source: TtsSource,
    },
    Paused {
        source: TtsSource,
    },
    Resumed {
        source: TtsSource,
    },
    Stopped {
        source: TtsSource,
    },
    Error {
        source: TtsSource,
        message: String,
    },
}
/// One backend → UI event. The GTK shell subscribes via [`EventBus::subscribe`].
#[derive(Clone, Debug)]
pub enum AppEvent {
    // --- settings ---
    /// A setting changed (name + new value as JSON).
    SettingsChanged {
        setting: String,
        value: serde_json::Value,
    },
    ThemeChanged(Theme),
    CheckForUpdates,
    // --- model lifecycle ---
    ModelStateChanged(ModelStateEvent),
    ModelsUpdated,
    ModelDeleted(String),
    ModelDownloadProgress(ModelDownloadProgressEvent),
    ModelDownloadFinished(ModelDownloadFinishedEvent),
    // --- transcription / streaming ---
    StreamText(StreamTextEvent),
    StreamPhase(StreamPhaseEvent),
    TranscriptionError(String),
    // --- audio / recording ---
    RecordingError(RecordingErrorEvent),
    /// Mic level for the overlay level meter (throttled).
    MicLevel(f32),
    /// TTS playback state (reader mini-player, chat speaker buttons).
    TtsStateChanged(TtsState),
    ShowOverlay,
    HideOverlay,
    // --- clipboard / paste ---
    PasteError,
    TextCopiedToClipboard {
        message: String,
    },
    // --- history ---
    HistoryUpdated(HistoryUpdatePayload),
    // --- agents (AI chat overlay) ---
    /// The agent/chat catalog changed (agents edited, chats added/deleted).
    AgentChatsChanged,
    /// A chat message was added or replaced (placeholder → final text).
    AgentMessageAdded {
        chat_id: i64,
        message: AgentMessageEvent,
    },
    /// One tool step executed during a turn (timeline UI).
    AgentStep {
        chat_id: i64,
        tool: String,
        summary: String,
    },
    /// One streamed content token (SSE delta) of the running turn. The UI
    /// appends it to the live assistant bubble; deltas are never persisted
    /// (only the final text is stored on `AgentDone`).
    AgentToken {
        chat_id: i64,
        delta: String,
    },
    /// A turn finished (`message_id` = placeholder row to replace).
    AgentDone {
        chat_id: i64,
        message_id: Option<i64>,
        truncated: bool,
    },
    /// A fire-and-forget UI command failed (reported via
    /// [`AppContext::report_error`]); the shell shows a toast.
    CommandFailed {
        context: String,
        message: String,
    },
    // --- debug ---
    /// A log line, forwarded only while debug mode is on (live log viewer).
    LogRecord(String),
}

/// A single event-bus subscriber callback.
pub type EventBusSubscriber = Arc<dyn Fn(AppEvent) + Send + Sync>;

/// Handle to a bus subscription. Dropping the handle does **not** unsubscribe
/// (callbacks must stay `'static`); call [`Subscription::unsubscribe`] — e.g.
/// from a window's `connect_destroy` — to stop receiving events and let the
/// bus release the callback.
pub struct Subscription {
    bus: EventBus,
    id: u64,
}

impl Subscription {
    /// Remove the callback from the bus. Idempotent.
    pub fn unsubscribe(&self) {
        self.bus.unsubscribe(self.id);
    }
}

/// Broadcast bus from backend threads to one-or-more UI subscribers.
///
/// Subscribers register a callback (typically marshaling onto the GTK main
/// loop via `glib::MainContext::default().invoke(...)`); `send` fans out to
/// all live subscribers from any thread.
///
/// Prefer [`EventBus::subscribe`] (which returns a [`Subscription`]) for
/// short-lived UI such as palettes and pages, and unsubscribe when the widget
/// is destroyed so repeated open/close cycles cannot accumulate callbacks.
#[derive(Clone, Default)]
pub struct EventBus {
    subscribers: Arc<Mutex<SubscriberList>>,
}

#[derive(Default)]
struct SubscriberList {
    next_id: u64,
    callbacks: Vec<(u64, EventBusSubscriber)>,
}

impl EventBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a subscriber callback and return a [`Subscription`] handle.
    /// The callback may be invoked from any thread, so it should marshal to
    /// the UI thread (e.g. with `glib::MainContext::default().invoke(...)`)
    /// and must never block.
    pub fn subscribe(&self, callback: impl Fn(AppEvent) + Send + Sync + 'static) -> Subscription {
        let mut guard = self.subscribers.lock().unwrap_or_else(|e| e.into_inner());
        guard.next_id += 1;
        let id = guard.next_id;
        guard.callbacks.push((id, Arc::new(callback)));
        Subscription {
            bus: self.clone(),
            id,
        }
    }

    /// Remove a previously registered callback. Idempotent.
    pub fn unsubscribe(&self, id: u64) {
        let mut guard = self.subscribers.lock().unwrap_or_else(|e| e.into_inner());
        guard.callbacks.retain(|(cid, _)| *cid != id);
    }

    /// Fan out to all subscribers without allocating a vector on send.
    pub fn send(&self, event: AppEvent) {
        let callbacks: Vec<EventBusSubscriber> = {
            let guard = self.subscribers.lock().unwrap_or_else(|e| e.into_inner());
            guard.callbacks.iter().map(|(_, cb)| cb.clone()).collect()
        };
        if callbacks.is_empty() {
            return;
        }

        let count = callbacks.len();
        for (i, callback) in callbacks.iter().enumerate() {
            if i + 1 == count {
                callback(event);
                break;
            } else {
                callback(event.clone());
            }
        }
    }
}

/// Everything the backend needs: paths, settings, managers and the event bus.
/// Cheap to clone (`Arc` fields + cloneable bus); shared across threads.
#[derive(Clone)]
pub struct AppContext {
    pub paths: AppPaths,
    pub bus: EventBus,
    pub model: Arc<ModelManager>,
    pub transcription: Arc<TranscriptionManager>,
    pub audio: Arc<AudioRecordingManager>,
    pub history: Arc<HistoryManager>,
    pub coordinator: Arc<TranscriptionCoordinator>,
}

impl AppContext {
    pub fn settings(&self) -> AppSettings {
        crate::settings::get_settings(self)
    }

    pub fn write_settings(&self, settings: &AppSettings) {
        crate::settings::write_settings(self, settings.clone());
    }

    /// Emit the standard `settings-changed` event the UI listens for.
    pub fn notify_setting_changed(&self, setting: &str, value: serde_json::Value) {
        self.bus.send(AppEvent::SettingsChanged {
            setting: setting.to_string(),
            value,
        });
    }

    /// Report a fire-and-forget UI failure: always logged (warn), and
    /// surfaced as a toast via the event bus so the failure is visible
    /// instead of silently swallowed with `let _ = …`.
    pub fn report_error(&self, context: &str, err: impl std::fmt::Display) {
        log::warn!("{context}: {err}");
        self.bus.send(AppEvent::CommandFailed {
            context: context.to_string(),
            message: err.to_string(),
        });
    }

    pub fn models_dir(&self) -> PathBuf {
        self.paths.models_dir()
    }

    pub fn resource_dir(&self) -> &Path {
        &self.paths.resource_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn unsubscribed_callbacks_stop_receiving_events() {
        let bus = EventBus::new();
        let hits = Arc::new(AtomicUsize::new(0));

        let hits_clone = hits.clone();
        let sub = bus.subscribe(move |_| {
            hits_clone.fetch_add(1, Ordering::SeqCst);
        });
        bus.send(AppEvent::PasteError);
        assert_eq!(hits.load(Ordering::SeqCst), 1);

        sub.unsubscribe();
        bus.send(AppEvent::PasteError);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn unsubscribe_is_idempotent_and_foreign_ids_are_ignored() {
        let bus = EventBus::new();
        let sub = bus.subscribe(|_| {});
        sub.unsubscribe();
        sub.unsubscribe();
        bus.unsubscribe(u64::MAX);
        // No panic; an empty bus is a silent no-op.
        bus.send(AppEvent::PasteError);
    }
}
