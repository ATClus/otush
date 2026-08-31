use crate::context::{AppContext, AppEvent};
use crate::managers::model::ModelInfo;
use crate::managers::transcription::{ModelStateEvent, TranscriptionManager};
use crate::settings::{get_settings, write_settings, ModelUnloadTimeout};
use std::sync::Arc;

/// Retrieve the list of all cataloged and locally installed models.
pub async fn get_available_models(ctx: &AppContext) -> Result<Vec<ModelInfo>, String> {
    Ok(ctx.model.get_available_models())
}

/// Retrieve model metadata for a specific model ID.
pub async fn get_model_info(
    ctx: &AppContext,
    model_id: String,
) -> Result<Option<ModelInfo>, String> {
    Ok(ctx.model.get_model_info(&model_id))
}

/// Open the local models directory in the user's file manager (Nautilus).
pub fn open_models_directory(ctx: &AppContext) -> Result<(), String> {
    let dir = ctx.model.models_dir();
    opener::open(dir).map_err(|e| format!("Failed to open models folder: {e}"))
}

/// Re-scan local sources for models added since launch.
pub async fn rescan_local_models(ctx: &AppContext) -> Result<(), String> {
    let mm = ctx.model.clone();
    crate::runtime::spawn_blocking(move || mm.rescan_local_models())
        .await
        .map_err(|e| format!("rescan task panicked: {e}"))?
        .map_err(|e| e.to_string())
}

/// Download a model from a URL or Hugging Face repository spec.
pub async fn download_model_from_url(ctx: &AppContext, url: String) -> Result<String, String> {
    crate::managers::model::download::download_model(ctx, &url)
        .await
        .map(|p| p.to_string_lossy().to_string())
}

/// Cancel an ongoing model download.
pub fn cancel_download() {
    crate::managers::model::download::cancel_download();
}

/// Explicitly load the currently selected model into memory.
pub async fn load_selected_model(ctx: &AppContext) -> Result<(), String> {
    let settings = get_settings(ctx);
    if settings.selected_model.is_empty() {
        return Err("No model is currently selected".to_string());
    }
    let tm = ctx.transcription.clone();
    let model_id = settings.selected_model.clone();
    crate::runtime::spawn_blocking(move || tm.load_model(&model_id))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
        .map_err(|e| e.to_string())
}

/// Delete a model's files from disk and unload it if active.
pub async fn delete_model(ctx: &AppContext, model_id: String) -> Result<(), String> {
    // If deleting the active model, unload it and clear the setting
    let settings = get_settings(ctx);
    if settings.selected_model == model_id {
        ctx.transcription
            .unload_model()
            .map_err(|e| format!("Failed to unload model: {}", e))?;

        let mut settings = get_settings(ctx);
        settings.selected_model = String::new();
        write_settings(ctx, settings);
    }

    ctx.model.delete_model(&model_id).map_err(|e| e.to_string())
}

/// Shared logic for switching the active model, used by both the UI and the
/// tray menu handler.
///
/// Validates the model, updates the persisted setting, and loads the model
/// unless the unload timeout is set to "Immediately" (in which case the model
/// will be loaded on-demand during the next transcription).
pub fn switch_active_model(ctx: &AppContext, model_id: &str) -> Result<(), String> {
    let model_manager = &ctx.model;
    let transcription_manager = &ctx.transcription;

    // Atomically claim the loading slot — prevents concurrent model loads
    // from tray double-clicks or overlapping commands. The guard resets the
    // flag on drop (including early returns, errors, and panics).
    let _loading_guard = transcription_manager
        .try_start_loading()
        .ok_or_else(|| "Model load already in progress".to_string())?;

    // Check if model exists and is available
    let model_info = model_manager
        .get_model_info(model_id)
        .ok_or_else(|| format!("Model not found: {}", model_id))?;

    if !model_info.is_downloaded {
        return Err(format!("Model not downloaded: {}", model_id));
    }

    let settings = get_settings(ctx);
    let unload_timeout = settings.model_unload_timeout;
    let old_model = settings.selected_model.clone();
    let old_onboarding_completed = settings.onboarding_completed;

    // Persist the new selection early so the frontend sees the correct model
    // when it reacts to events emitted by load_model.
    let mut settings = settings;
    settings.selected_model = model_id.to_string();
    settings.onboarding_completed = true;

    write_settings(ctx, settings);

    // Skip eager loading if unload is set to "Immediately" — the model
    // will be loaded on-demand during the next transcription.
    if unload_timeout == ModelUnloadTimeout::Immediately {
        // Notify UI — load_model won't be called so no events would otherwise
        // be emitted.
        ctx.bus.send(AppEvent::ModelStateChanged(ModelStateEvent {
            event_type: "selection_changed".to_string(),
            model_id: Some(model_id.to_string()),
            model_name: Some(model_info.name),
            error: None,
        }));
        log::info!(
            "Model selection changed to {} (not loading — unload set to Immediately).",
            model_id
        );
        return Ok(());
    }

    // Load the model. On failure, revert the persisted selection.
    if let Err(e) = transcription_manager.load_model(model_id) {
        let mut settings = get_settings(ctx);
        settings.selected_model = old_model;
        settings.onboarding_completed = old_onboarding_completed;
        write_settings(ctx, settings);
        return Err(e.to_string());
    }

    Ok(())
}

pub async fn set_active_model(ctx: &AppContext, model_id: String) -> Result<(), String> {
    switch_active_model(ctx, &model_id)
}

pub async fn get_current_model(ctx: &AppContext) -> Result<String, String> {
    let settings = get_settings(ctx);
    Ok(settings.selected_model)
}

pub async fn get_transcription_model_status(ctx: &AppContext) -> Result<Option<String>, String> {
    Ok(ctx.transcription.get_current_model())
}

pub async fn is_model_loading(ctx: &AppContext) -> Result<bool, String> {
    // Check if transcription manager has a loaded model
    let current_model = ctx.transcription.get_current_model();
    Ok(current_model.is_none())
}

/// Keep the type alias used by callers that pass the transcription manager
/// around (e.g. tray menu handlers).
pub type TranscriptionManagerRef = Arc<TranscriptionManager>;
