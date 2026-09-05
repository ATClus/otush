//! Audio/paste UI settings (split from `shortcut/mod.rs`; same behavior, same paths).

use crate::context::AppContext;
use crate::settings::{self, VadBackend};

pub fn change_append_trailing_space_setting(ctx: &AppContext, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.append_trailing_space = enabled;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("append_trailing_space", serde_json::json!(enabled));
    Ok(())
}

pub fn change_vad_enabled_setting(ctx: &AppContext, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.vad_enabled = enabled;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("vad_enabled", serde_json::json!(enabled));
    Ok(())
}

pub async fn change_vad_backend_setting(
    ctx: &AppContext,
    backend: VadBackend,
) -> Result<(), String> {
    if settings::get_settings(ctx).vad_backend == backend {
        return Ok(());
    }

    // Construct/swap the detector and, when necessary, reopen cpal away from
    // the main thread. Persist only after the runtime change succeeds so a
    // rejected in-progress switch or failed microphone reopen rolls back cleanly.
    let manager = ctx.audio.clone();
    crate::runtime::spawn_blocking(move || manager.update_vad_backend(backend))
        .await
        .map_err(|e| format!("audio task join failed: {e}"))?
        .map_err(|e| format!("Failed to update VAD backend: {e}"))?;

    let mut current_settings = settings::get_settings(ctx);
    current_settings.vad_backend = backend;
    settings::write_settings(ctx, current_settings);
    ctx.notify_setting_changed("vad_backend", serde_json::json!(backend));
    Ok(())
}

pub fn change_filler_word_removal_enabled_setting(
    ctx: &AppContext,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.filler_word_removal_enabled = enabled;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("filler_word_removal_enabled", serde_json::json!(enabled));
    Ok(())
}
