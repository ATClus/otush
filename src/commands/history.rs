use super::errors::{CommandError, CommandResult};
use crate::actions::process_transcription_output;
use crate::context::{AppContext, AppEvent};
use crate::managers::history::{HistoryUpdatePayload, PaginatedHistory};
use rodio::OutputStreamBuilder;
use std::fs::File;
use std::io::BufReader;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;

static CURRENT_PLAYING_ID: AtomicI64 = AtomicI64::new(0);
static STOP_PLAYBACK_FLAG: LazyLock<Mutex<Option<Arc<AtomicBool>>>> =
    LazyLock::new(|| Mutex::new(None));

/// Check if audio for the given history entry is currently playing.
pub fn is_playing_history_audio(id: i64) -> bool {
    CURRENT_PLAYING_ID.load(Ordering::SeqCst) == id
}

/// Stop any currently playing history audio.
pub fn stop_history_audio() {
    CURRENT_PLAYING_ID.store(0, Ordering::SeqCst);
    let mut guard = STOP_PLAYBACK_FLAG.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(flag) = guard.take() {
        flag.store(true, Ordering::SeqCst);
    }
}

/// Toggle playback of history entry audio.
/// Returns Ok(true) if playback started, or Ok(false) if playback stopped.
pub async fn toggle_play_history_audio(ctx: &AppContext, id: i64) -> CommandResult<bool> {
    if CURRENT_PLAYING_ID.load(Ordering::SeqCst) == id {
        stop_history_audio();
        ctx.bus
            .send(AppEvent::HistoryUpdated(HistoryUpdatePayload::Toggled {
                id,
            }));
        return Ok(false);
    }

    stop_history_audio();

    let entry = ctx
        .history
        .get_entry_by_id(id)
        .await?
        .ok_or(CommandError::HistoryEntryNotFound(id))?;

    let audio_path = ctx.history.get_audio_file_path(&entry.file_name);
    if !audio_path.exists() {
        return Err(CommandError::audio_file_missing(
            &audio_path,
            &entry.file_name,
        ));
    }

    let stop_flag = Arc::new(AtomicBool::new(false));
    {
        let mut guard = STOP_PLAYBACK_FLAG.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some(Arc::clone(&stop_flag));
    }
    CURRENT_PLAYING_ID.store(id, Ordering::SeqCst);

    let bus = ctx.bus.clone();

    thread::spawn(move || {
        let play_result = (|| -> Result<(), Box<dyn std::error::Error>> {
            let stream_builder = OutputStreamBuilder::from_default_device()?;
            let stream_handle = stream_builder.open_stream()?;
            let mixer = stream_handle.mixer();
            let file = File::open(&audio_path)?;
            let buf_reader = BufReader::new(file);
            let sink = rodio::play(mixer, buf_reader)?;

            while !sink.empty() {
                if stop_flag.load(Ordering::SeqCst) {
                    sink.stop();
                    break;
                }
                thread::sleep(std::time::Duration::from_millis(50));
            }
            Ok(())
        })();

        if let Err(e) = play_result {
            log::error!("Playback error: {e}");
        }

        if CURRENT_PLAYING_ID.load(Ordering::SeqCst) == id {
            CURRENT_PLAYING_ID.store(0, Ordering::SeqCst);
            bus.send(AppEvent::HistoryUpdated(HistoryUpdatePayload::Toggled {
                id,
            }));
        }
    });

    Ok(true)
}

/// Query paginated transcription history entries.
pub async fn get_history_entries(
    ctx: &AppContext,
    cursor: Option<i64>,
    limit: Option<usize>,
) -> CommandResult<PaginatedHistory> {
    Ok(ctx.history.get_history_entries(cursor, limit).await?)
}

/// Toggle the starred/saved bookmark state for a history entry.
pub async fn toggle_history_entry_saved(ctx: &AppContext, id: i64) -> CommandResult<()> {
    Ok(ctx.history.toggle_saved_status(id).await?)
}

/// Permanently delete a history database record and its associated audio file.
pub async fn delete_history_entry(ctx: &AppContext, id: i64) -> CommandResult<()> {
    Ok(ctx.history.delete_entry(id).await?)
}

/// Re-run speech-to-text transcription on an archived historical audio file.
pub async fn retry_history_entry_transcription(ctx: &AppContext, id: i64) -> CommandResult<()> {
    let entry = ctx
        .history
        .get_entry_by_id(id)
        .await?
        .ok_or(CommandError::HistoryEntryNotFound(id))?;

    let audio_path = ctx.history.get_audio_file_path(&entry.file_name);
    let samples = crate::audio_toolkit::read_wav_samples(&audio_path)
        .map_err(|e| CommandError::AudioLoad(e.to_string()))?;

    if samples.is_empty() {
        return Err(CommandError::EmptyRecording);
    }

    let settings = ctx.settings();
    let transcription = if settings.local_transcription_enabled {
        ctx.transcription.initiate_model_load();
        let tm = ctx.transcription.clone();
        tokio::task::spawn_blocking(move || tm.transcribe(samples))
            .await
            .map_err(CommandError::transcription_join)??
    } else {
        crate::stt_client::transcribe_with_fallback(&settings, &samples)
            .await
            .map_err(CommandError::Input)?
    };

    if transcription.is_empty() {
        return Err(CommandError::NoSpeech);
    }

    let processed =
        process_transcription_output(ctx, &transcription, entry.post_process_requested, false)
            .await;
    ctx.history
        .update_transcription(
            id,
            transcription,
            processed.post_processed_text,
            processed.post_process_prompt,
        )
        .map(|_| ())
        .map_err(CommandError::from)
}

pub async fn update_history_limit(ctx: &AppContext, limit: usize) -> CommandResult<()> {
    let mut settings = crate::settings::get_settings(ctx);
    settings.history_limit = limit;
    crate::settings::write_settings(ctx, settings);

    ctx.history.cleanup_old_entries()?;

    Ok(())
}
