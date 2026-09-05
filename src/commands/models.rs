use super::errors::{CommandError, CommandResult};
use crate::context::{AppContext, AppEvent};
use crate::managers::transcription::ModelStateEvent;
use crate::settings::{get_settings, write_settings, ModelUnloadTimeout};

/// Re-scan local sources for models added since launch.
pub async fn rescan_local_models(ctx: &AppContext) -> CommandResult<()> {
    let mm = ctx.model.clone();
    crate::runtime::spawn_blocking(move || mm.rescan_local_models())
        .await
        .map_err(CommandError::rescan_join)?
        .map_err(CommandError::from)
}

/// Delete a model's files from disk and unload it if active.
pub async fn delete_model(ctx: &AppContext, model_id: String) -> CommandResult<()> {
    // If deleting the active model, unload it and clear the setting
    let settings = get_settings(ctx);
    if settings.selected_model == model_id {
        ctx.transcription
            .unload_model()
            .map_err(|e| CommandError::ModelUnload(e.to_string()))?;

        let mut settings = get_settings(ctx);
        settings.selected_model = String::new();
        write_settings(ctx, settings);
    }

    Ok(ctx.model.delete_model(&model_id)?)
}

/// Shared logic for switching the active model, used by both the UI and the
/// tray menu handler.
///
/// Validates the model, updates the persisted setting, and loads the model
/// unless the unload timeout is set to "Immediately" (in which case the model
/// will be loaded on-demand during the next transcription).
pub fn switch_active_model(ctx: &AppContext, model_id: &str) -> CommandResult<()> {
    let model_manager = &ctx.model;
    let transcription_manager = &ctx.transcription;

    // Atomically claim the loading slot — prevents concurrent model loads
    // from tray double-clicks or overlapping commands. The guard resets the
    // flag on drop (including early returns, errors, and panics).
    let _loading_guard = transcription_manager
        .try_start_loading()
        .ok_or(CommandError::ModelLoadBusy)?;

    // Check if model exists and is available
    let model_info = model_manager
        .get_model_info(model_id)
        .ok_or_else(|| CommandError::ModelNotFound(model_id.to_string()))?;

    if !model_info.is_downloaded {
        return Err(CommandError::ModelNotDownloaded(model_id.to_string()));
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
        return Err(CommandError::from(e));
    }

    Ok(())
}
