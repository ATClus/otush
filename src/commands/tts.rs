//! Text-to-Speech playback commands (Reader + Chat speak modes).
//!
//! Synthesis runs through [`crate::tts_client::synthesize_with_fallback`];
//! audio bytes are played on a dedicated worker thread via rodio, honoring
//! the selected output device and feedback volume (same pattern as
//! [`crate::audio_feedback`] and history playback). The reader mode
//! pre-fetches one chunk ahead while the current chunk plays; any new
//! `speak_*` call or [`stop_speaking`] invalidates the previous queue via a
//! generation counter (same pattern as
//! [`crate::managers::audio::AudioRecordingManager`]).
//!
//! Progress surfaces through [`crate::context::AppEvent::TtsStateChanged`];
//! widgets must subscribe and marshal with `glib::MainContext::invoke`.

use super::errors::{CommandError, CommandResult};
use crate::context::{AppContext, AppEvent, TtsSource, TtsState};
use crate::settings::{get_settings, TtsFormat};
use crate::tts_client::{
    chunk_text_for_tts, max_chars_for_provider, plain_text_for_tts, TtsAudio, TtsRequest,
};
use cpal::traits::{DeviceTrait, HostTrait};
use rodio::OutputStreamBuilder;
use std::io::{BufReader, Cursor};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;

/// Current speak generation; bumped on every new utterance or stop so stale
/// worker loops exit early.
static TTS_GENERATION: AtomicU64 = AtomicU64::new(0);
/// True while a chunk is audibly playing (not paused).
static TTS_PLAYING: AtomicBool = AtomicBool::new(false);
/// Pause flag consumed by the playback loop.
static TTS_PAUSED: AtomicBool = AtomicBool::new(false);
/// Stop flag consumed by the playback loop.
static TTS_STOP_FLAG: LazyLock<Mutex<Option<Arc<AtomicBool>>>> = LazyLock::new(|| Mutex::new(None));

/// Whether TTS audio is currently playing (or paused mid-utterance).
pub fn is_speaking() -> bool {
    TTS_PLAYING.load(Ordering::SeqCst)
}

/// Whether playback is currently paused.
pub fn is_paused() -> bool {
    TTS_PAUSED.load(Ordering::SeqCst)
}

/// Stop any in-flight TTS playback and invalidate its queue.
pub fn stop_speaking(ctx: &AppContext, source: TtsSource) {
    TTS_GENERATION.fetch_add(1, Ordering::AcqRel);
    TTS_PAUSED.store(false, Ordering::SeqCst);
    TTS_PLAYING.store(false, Ordering::SeqCst);
    if let Some(flag) = TTS_STOP_FLAG
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
    {
        flag.store(true, Ordering::SeqCst);
    }
    ctx.bus
        .send(AppEvent::TtsStateChanged(TtsState::Stopped { source }));
}

/// Pause playback after the current chunk (resumable via [`resume_speaking`]).
pub fn pause_speaking(ctx: &AppContext, source: TtsSource) {
    if TTS_PLAYING.load(Ordering::SeqCst) {
        TTS_PAUSED.store(true, Ordering::SeqCst);
        ctx.bus
            .send(AppEvent::TtsStateChanged(TtsState::Paused { source }));
    }
}

/// Resume a paused utterance.
pub fn resume_speaking(ctx: &AppContext, source: TtsSource) {
    if TTS_PAUSED.load(Ordering::SeqCst) {
        TTS_PAUSED.store(false, Ordering::SeqCst);
        ctx.bus
            .send(AppEvent::TtsStateChanged(TtsState::Resumed { source }));
    }
}

/// Speak arbitrary text (reader mode entry point for notes/web/docs).
pub async fn speak_text(ctx: &AppContext, text: String) -> CommandResult<()> {
    speak_inner(ctx, text, TtsSource::Reader, true).await
}

/// Speak one chat answer (single request, cancels any previous speech).
pub async fn speak_chat_message(ctx: &AppContext, text: String) -> CommandResult<()> {
    speak_inner(ctx, text, TtsSource::Chat, false).await
}

async fn speak_inner(
    ctx: &AppContext,
    text: String,
    source: TtsSource,
    chunk: bool,
) -> CommandResult<()> {
    if crate::tts_client::plain_text_for_tts(&text).is_empty() {
        return Err(CommandError::TtsNothingToRead);
    }
    if ctx.audio.is_recording() {
        return Err(CommandError::Tts(
            "Stop recording before playing speech".to_string(),
        ));
    }
    // Never overlap history audio or feedback chimes.
    super::history::stop_history_audio();

    let settings = get_settings(ctx);
    let chunk_chars = if chunk {
        let configured = settings.tts_reader_chunk_chars.max(256) as usize;
        let provider_cap = settings
            .active_tts_provider()
            .map(|p| max_chars_for_provider(&p.id))
            .unwrap_or(2000);
        configured.min(provider_cap)
    } else {
        usize::MAX
    };
    let chunks = chunk_text_for_tts(&text, chunk_chars);
    if chunks.is_empty() {
        return Err(CommandError::TtsNothingToRead);
    }

    // Invalidate any previous utterance before starting the new worker.
    stop_speaking(ctx, source);
    let generation = TTS_GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    let stop_flag = Arc::new(AtomicBool::new(false));
    *TTS_STOP_FLAG.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::clone(&stop_flag));
    TTS_PLAYING.store(true, Ordering::SeqCst);
    TTS_PAUSED.store(false, Ordering::SeqCst);

    let total = chunks.len();
    ctx.bus.send(AppEvent::TtsStateChanged(TtsState::Started {
        total,
        source,
    }));

    let worker_ctx = ctx.clone();
    crate::runtime::spawn(async move {
        let mut index = 0usize;
        // One-chunk pre-fetch: synthesize chunk N+1 while chunk N plays.
        let mut prefetched: Option<TtsAudio> = None;
        while index < total {
            if stop_flag.load(Ordering::SeqCst)
                || TTS_GENERATION.load(Ordering::Acquire) != generation
            {
                break;
            }
            let settings = get_settings(&worker_ctx);
            let req = TtsRequest {
                text: chunks[index].clone(),
                voice: String::new(),
                model: String::new(),
                language: settings.selected_language.clone(),
                speaking_rate: settings.effective_tts_speaking_rate(),
                format: settings.tts_format,
            };
            let audio = if let Some(ready) = prefetched.take() {
                ready
            } else {
                match crate::tts_client::synthesize_with_fallback(&settings, &req).await {
                    Ok(audio) => audio,
                    Err(e) => {
                        TTS_PLAYING.store(false, Ordering::SeqCst);
                        worker_ctx
                            .bus
                            .send(AppEvent::TtsStateChanged(TtsState::Error {
                                source,
                                message: e.clone(),
                            }));
                        worker_ctx.report_error("tts_synthesis", CommandError::Tts(e));
                        return;
                    }
                }
            };

            // Pre-fetch the next chunk concurrently with playback.
            let next_handle = if index + 1 < total {
                let worker_ctx = worker_ctx.clone();
                let next_text = chunks[index + 1].clone();
                Some(crate::runtime::spawn(async move {
                    let settings = get_settings(&worker_ctx);
                    let req = TtsRequest {
                        text: next_text,
                        voice: String::new(),
                        model: String::new(),
                        language: settings.selected_language.clone(),
                        speaking_rate: settings.effective_tts_speaking_rate(),
                        format: settings.tts_format,
                    };
                    crate::tts_client::synthesize_with_fallback(&settings, &req)
                        .await
                        .ok()
                }))
            } else {
                None
            };

            if let Err(e) = play_audio_blocking(&worker_ctx, &audio, &stop_flag, generation).await {
                if TTS_GENERATION.load(Ordering::Acquire) == generation
                    && !stop_flag.load(Ordering::SeqCst)
                {
                    TTS_PLAYING.store(false, Ordering::SeqCst);
                    worker_ctx
                        .bus
                        .send(AppEvent::TtsStateChanged(TtsState::Error {
                            source,
                            message: e.clone(),
                        }));
                    worker_ctx.report_error("tts_playback", CommandError::Tts(e));
                }
                return;
            }

            if let Some(handle) = next_handle {
                match handle.await {
                    Ok(Some(next_audio)) => prefetched = Some(next_audio),
                    Ok(None) => {
                        // Pre-fetch failed (provider error); the next loop
                        // iteration retries through the fallback chain.
                    }
                    Err(_) => {
                        if TTS_GENERATION.load(Ordering::Acquire) != generation {
                            return;
                        }
                    }
                }
            }

            index += 1;
            if TTS_GENERATION.load(Ordering::Acquire) == generation
                && !stop_flag.load(Ordering::SeqCst)
            {
                worker_ctx
                    .bus
                    .send(AppEvent::TtsStateChanged(TtsState::ChunkProgress {
                        done: index,
                        total,
                        source,
                    }));
            }
        }

        let still_current = TTS_GENERATION.load(Ordering::Acquire) == generation
            && !stop_flag.load(Ordering::SeqCst);
        TTS_PLAYING.store(false, Ordering::SeqCst);
        TTS_PAUSED.store(false, Ordering::SeqCst);
        if still_current {
            worker_ctx
                .bus
                .send(AppEvent::TtsStateChanged(TtsState::Stopped { source }));
        }
    })
    .await
    .map_err(CommandError::task_join)?;

    Ok(())
}

async fn play_audio_blocking(
    ctx: &AppContext,
    audio: &TtsAudio,
    stop_flag: &Arc<AtomicBool>,
    generation: u64,
) -> Result<(), String> {
    let settings = get_settings(ctx);
    let selected_device = settings.selected_output_device.clone();
    let volume = settings.audio_feedback_volume.clamp(0.0, 1.0);
    let bytes = audio.bytes.clone();
    let format = audio.format;
    let stop_flag = Arc::clone(stop_flag);

    log::debug!(
        "TTS playback start: bytes={} format={:?} device={:?} volume={}",
        bytes.len(),
        format,
        selected_device,
        volume,
    );
    // Magic-byte check: Deepgram errors arrive as JSON even on 200 paths in
    // some proxies — fail fast with a readable message instead of silence.
    let looks_like_audio = match format {
        TtsFormat::Mp3 => {
            bytes.len() > 3 && bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0
                || bytes.starts_with(b"ID3")
        }
        TtsFormat::Wav => bytes.starts_with(b"RIFF"),
    };
    if !looks_like_audio {
        let head: String = bytes
            .iter()
            .take(200)
            .map(|b| {
                if b.is_ascii_graphic() || *b == b' ' {
                    *b as char
                } else {
                    '.'
                }
            })
            .collect();
        log::warn!("TTS bytes are not {:?} audio; head={:?}", format, head);
        return Err(format!(
            "TTS returned non-audio data (expected {:?}): {head}",
            format
        ));
    }

    crate::runtime::spawn_blocking(move || {
        // Honor pause: spin in short sleeps so resume is snappy and stop
        // still preempts while paused.
        let wait_pause = || {
            while TTS_PAUSED.load(Ordering::SeqCst) {
                if stop_flag.load(Ordering::SeqCst)
                    || TTS_GENERATION.load(Ordering::Acquire) != generation
                {
                    return false;
                }
                thread::sleep(std::time::Duration::from_millis(50));
            }
            true
        };
        if !wait_pause() {
            return Ok(());
        }

        let stream_builder = resolve_output_stream(&selected_device)
            .map_err(|e| format!("Failed to open output device: {e}"))?;
        let stream_handle = stream_builder
            .open_stream()
            .map_err(|e| format!("Failed to open output stream: {e}"))?;
        let mixer = stream_handle.mixer();

        let cursor = Cursor::new(bytes);
        let reader = BufReader::new(cursor);
        let sink = match format {
            TtsFormat::Mp3 | TtsFormat::Wav => rodio::play(mixer, reader),
        }
        .map_err(|e| {
            log::warn!("TTS rodio decode failed: {e}");
            format!("Failed to decode TTS audio: {e}")
        })?;
        sink.set_volume(volume);
        log::debug!("TTS sink playing (volume={volume})");

        while !sink.empty() {
            if stop_flag.load(Ordering::SeqCst)
                || TTS_GENERATION.load(Ordering::Acquire) != generation
            {
                sink.stop();
                break;
            }
            if TTS_PAUSED.load(Ordering::SeqCst) {
                sink.pause();
                if !wait_pause() {
                    sink.stop();
                    break;
                }
                sink.play();
            }
            thread::sleep(std::time::Duration::from_millis(50));
        }
        Ok::<_, String>(())
    })
    .await
    .map_err(|e| format!("TTS playback task failed: {e}"))?
}

fn resolve_output_stream(
    selected_device: &Option<String>,
) -> Result<OutputStreamBuilder, Box<dyn std::error::Error>> {
    if let Some(name) = selected_device {
        if name != "Default" {
            let host = crate::audio_toolkit::get_cpal_host();
            for device in host.output_devices()? {
                if device.name()? == *name {
                    return Ok(OutputStreamBuilder::from_device(device)?);
                }
            }
            log::warn!("TTS output device '{name}' not found, using default");
        }
    }
    Ok(OutputStreamBuilder::from_default_device()?)
}

/// Test a TTS provider connection (short synthesis, no playback).
/// Returns `(audio_bytes, elapsed_ms)`.
pub async fn test_tts_connection(
    ctx: &AppContext,
    provider_id: String,
) -> CommandResult<(usize, u128)> {
    let settings = get_settings(ctx);
    let provider = settings
        .tts_provider(&provider_id)
        .cloned()
        .ok_or_else(|| CommandError::Tts(format!("Unknown TTS provider '{provider_id}'")))?;
    let api_key = settings.tts_api_key(&provider_id);
    let voice = settings
        .tts_voices
        .get(&provider_id)
        .cloned()
        .unwrap_or_default();
    let model = settings
        .tts_models
        .get(&provider_id)
        .cloned()
        .unwrap_or_default();
    crate::tts_client::test_tts_provider(&provider, &settings, api_key, &voice, &model)
        .await
        .map_err(CommandError::Tts)
}

/// Speakable-plain-text preview (used by the reader UI for empty checks).
pub fn speakable_text(text: &str) -> String {
    plain_text_for_tts(text)
}
