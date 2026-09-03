//! Transcription engine control commands.

#![allow(dead_code)]
use crate::context::AppContext;
use crate::settings::{get_settings, write_settings, ModelUnloadTimeout};
use serde::Serialize;

/// Information about whether a model is currently in memory.
#[derive(Serialize)]
pub struct ModelLoadStatus {
    pub is_loaded: bool,
    pub current_model: Option<String>,
}

/// Set and persist the model idle unload timeout policy.
pub fn set_model_unload_timeout(ctx: &AppContext, timeout: ModelUnloadTimeout) {
    let mut settings = get_settings(ctx);
    settings.model_unload_timeout = timeout;
    write_settings(ctx, settings);
}

/// Query whether a model is loaded in memory and which model ID it is.
pub fn get_model_load_status(ctx: &AppContext) -> Result<ModelLoadStatus, String> {
    Ok(ModelLoadStatus {
        is_loaded: ctx.transcription.is_model_loaded(),
        current_model: ctx.transcription.get_current_model(),
    })
}

/// Manually unload the currently resident model to free RAM/VRAM.
pub fn unload_model_manually(ctx: &AppContext) -> Result<(), String> {
    ctx.transcription
        .unload_model()
        .map_err(|e| format!("Failed to unload model: {}", e))
}

/// Transcribe an audio/video file from disk with progress reporting, optional post-processing, and history persistence.
pub async fn transcribe_media_file(
    ctx: &AppContext,
    path: &std::path::Path,
    prompt_id: Option<String>,
    progress_callback: Option<crate::audio_toolkit::audio::decoder::ProgressCallback>,
) -> Result<crate::audio_toolkit::TranscriptDocument, String> {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("audio_file")
        .to_string();

    let cb_clone = progress_callback.clone();
    let path_buf = path.to_path_buf();

    // 1. Decode media file
    let decoded = tokio::task::spawn_blocking(move || {
        crate::audio_toolkit::decode_media_file(&path_buf, cb_clone)
    })
    .await
    .map_err(|e| format!("Decoding task panicked: {}", e))?
    .map_err(|e| format!("Failed to decode media file: {}", e))?;

    // 2. Save audio WAV copy to recordings_dir first so samples can be moved without cloning
    let safe_base = file_name.replace(|c: char| !c.is_alphanumeric() && c != '.' && c != '_', "_");
    let target_recording_name = format!("{}_{}.wav", chrono::Utc::now().timestamp(), safe_base);
    let target_wav_path = ctx.history.get_audio_file_path(&target_recording_name);
    let _ = crate::audio_toolkit::save_wav_file(&target_wav_path, &decoded.samples);

    // 3. Transcribe samples (zero-copy ownership transfer of decoded.samples)
    let tm = ctx.transcription.clone();
    let title = file_name.clone();
    let samples = decoded.samples;
    let cb_stt = progress_callback.clone();

    let mut doc =
        tokio::task::spawn_blocking(move || tm.transcribe_with_segments(samples, title, cb_stt))
            .await
            .map_err(|e| format!("Transcription task panicked: {}", e))?
            .map_err(|e| format!("Transcription failed: {}", e))?;

    let full_text = doc
        .segments
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");

    // 4. Optional LLM Post-Processing
    let settings = ctx.settings();
    let mut post_processed_text: Option<String> = None;
    let mut used_prompt_title: Option<String> = None;

    if let Some(pid) = prompt_id {
        if let Some(prompt) = settings.post_process_prompts.iter().find(|p| p.id == pid) {
            used_prompt_title = Some(prompt.name.clone());
            if let Some(transformed) =
                crate::actions::post_process_text_with_prompt(&settings, &full_text, prompt).await
            {
                doc.summary_or_post_processed = Some(transformed.clone());
                post_processed_text = Some(transformed);
            }
        }
    } else if settings.post_process_enabled && !full_text.trim().is_empty() {
        if let Some(transformed) =
            crate::actions::post_process_transcription(&settings, &full_text).await
        {
            doc.summary_or_post_processed = Some(transformed.clone());
            post_processed_text = Some(transformed);
        }
    }

    // 5. Save transcription entry to history
    let _ = ctx.history.save_entry(
        target_recording_name,
        full_text,
        post_processed_text.is_some(),
        post_processed_text,
        used_prompt_title,
        Some("file"),
    );

    Ok(doc)
}
