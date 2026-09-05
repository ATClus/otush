//! Audio DSP settings (split from `shortcut/mod.rs`; same behavior, same paths).

use crate::context::AppContext;
use crate::settings::{self};

pub fn change_audio_input_gain_setting(ctx: &AppContext, gain: f32) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.audio_input_gain = gain;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("audio_input_gain", serde_json::json!(gain));
    Ok(())
}

pub fn change_audio_normalization_setting(ctx: &AppContext, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.audio_normalization_enabled = enabled;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("audio_normalization_enabled", serde_json::json!(enabled));
    Ok(())
}

pub fn change_audio_high_pass_filter_setting(
    ctx: &AppContext,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.audio_high_pass_filter_enabled = enabled;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("audio_high_pass_filter_enabled", serde_json::json!(enabled));
    Ok(())
}

pub fn change_audio_noise_reduction_setting(ctx: &AppContext, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.audio_noise_reduction_enabled = enabled;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("audio_noise_reduction_enabled", serde_json::json!(enabled));
    Ok(())
}

pub fn change_audio_noise_gate_threshold_setting(
    ctx: &AppContext,
    threshold_db: f32,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.audio_noise_gate_threshold_db = threshold_db;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed(
        "audio_noise_gate_threshold_db",
        serde_json::json!(threshold_db),
    );
    Ok(())
}
