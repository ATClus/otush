#![allow(dead_code)]
use crate::actions::process_transcription_output;
use crate::context::AppContext;
use crate::managers::history::PaginatedHistory;

pub async fn get_history_entries(
    ctx: &AppContext,
    cursor: Option<i64>,
    limit: Option<usize>,
) -> Result<PaginatedHistory, String> {
    ctx.history
        .get_history_entries(cursor, limit)
        .await
        .map_err(|e| e.to_string())
}

pub async fn toggle_history_entry_saved(ctx: &AppContext, id: i64) -> Result<(), String> {
    ctx.history
        .toggle_saved_status(id)
        .await
        .map_err(|e| e.to_string())
}

pub async fn get_audio_file_path(ctx: &AppContext, file_name: String) -> Result<String, String> {
    let path = ctx.history.get_audio_file_path(&file_name);
    path.to_str()
        .ok_or_else(|| "Invalid file path".to_string())
        .map(|s| s.to_string())
}

pub async fn delete_history_entry(ctx: &AppContext, id: i64) -> Result<(), String> {
    ctx.history
        .delete_entry(id)
        .await
        .map_err(|e| e.to_string())
}

pub async fn retry_history_entry_transcription(ctx: &AppContext, id: i64) -> Result<(), String> {
    let entry = ctx
        .history
        .get_entry_by_id(id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("History entry {} not found", id))?;

    let audio_path = ctx.history.get_audio_file_path(&entry.file_name);
    let samples = crate::audio_toolkit::read_wav_samples(&audio_path)
        .map_err(|e| format!("Failed to load audio: {}", e))?;

    if samples.is_empty() {
        return Err("Recording has no audio samples".to_string());
    }

    ctx.transcription.initiate_model_load();

    let tm = ctx.transcription.clone();
    let transcription = tokio::task::spawn_blocking(move || tm.transcribe(samples))
        .await
        .map_err(|e| format!("Transcription task panicked: {}", e))?
        .map_err(|e| e.to_string())?;

    if transcription.is_empty() {
        return Err("Recording contains no speech".to_string());
    }

    let processed =
        process_transcription_output(ctx, &transcription, entry.post_process_requested).await;
    ctx.history
        .update_transcription(
            id,
            transcription,
            processed.post_processed_text,
            processed.post_process_prompt,
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub async fn update_history_limit(ctx: &AppContext, limit: usize) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(ctx);
    settings.history_limit = limit;
    crate::settings::write_settings(ctx, settings);

    ctx.history
        .cleanup_old_entries()
        .map_err(|e| e.to_string())?;

    Ok(())
}

pub async fn update_recording_retention_period(
    ctx: &AppContext,
    period: String,
) -> Result<(), String> {
    use crate::settings::RecordingRetentionPeriod;

    let retention_period = match period.as_str() {
        "never" => RecordingRetentionPeriod::Never,
        "preserve_limit" => RecordingRetentionPeriod::PreserveLimit,
        "days3" => RecordingRetentionPeriod::Days3,
        "weeks2" => RecordingRetentionPeriod::Weeks2,
        "months3" => RecordingRetentionPeriod::Months3,
        _ => return Err(format!("Invalid retention period: {}", period)),
    };

    let mut settings = crate::settings::get_settings(ctx);
    settings.recording_retention_period = retention_period;
    crate::settings::write_settings(ctx, settings);

    ctx.history
        .cleanup_old_entries()
        .map_err(|e| e.to_string())?;

    Ok(())
}
