//! Compute accelerator settings (split from `shortcut/mod.rs`; same behavior, same paths).

use crate::context::AppContext;
use crate::settings;

/// Save accelerator settings and make the next model use reload with them.
/// The currently running transcription, if any, keeps its existing engine.
fn save_accelerator_and_reload_next_use(ctx: &AppContext, s: settings::AppSettings) {
    settings::write_settings(ctx, s);
    ctx.transcription.reload_model_on_next_use();
}

pub fn change_transcribe_accelerator_setting(
    ctx: &AppContext,
    accelerator: settings::TranscribeAcceleratorSetting,
) -> Result<(), String> {
    let mut s = settings::get_settings(ctx);
    s.transcribe_accelerator = accelerator;
    save_accelerator_and_reload_next_use(ctx, s);
    ctx.notify_setting_changed("transcribe_accelerator", serde_json::json!(accelerator));
    Ok(())
}

pub fn change_ort_accelerator_setting(
    ctx: &AppContext,
    accelerator: settings::OrtAcceleratorSetting,
) -> Result<(), String> {
    let mut s = settings::get_settings(ctx);
    s.ort_accelerator = accelerator;
    save_accelerator_and_reload_next_use(ctx, s);
    ctx.notify_setting_changed("ort_accelerator", serde_json::json!(accelerator));
    Ok(())
}

pub fn toggle_local_transcription_setting(ctx: &AppContext, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.local_transcription_enabled = enabled;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("local_transcription_enabled", serde_json::json!(enabled));
    Ok(())
}
