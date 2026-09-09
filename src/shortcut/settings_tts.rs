//! TTS provider settings (same pattern as `settings_transcription`).

use crate::context::AppContext;
use crate::settings;

pub fn move_tts_provider_priority(
    ctx: &AppContext,
    provider_id: &str,
    up: bool,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    let len = settings.tts_providers.len();
    if len <= 1 {
        return Ok(());
    }

    let Some(index) = settings
        .tts_providers
        .iter()
        .position(|p| p.id == provider_id)
    else {
        return Err(format!("TTS provider '{provider_id}' not found"));
    };

    if (up && index == 0) || (!up && index + 1 >= len) {
        return Ok(());
    }

    let target_index = if up { index - 1 } else { index + 1 };
    settings.tts_providers.swap(index, target_index);
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("tts_providers_reordered", serde_json::json!(provider_id));
    Ok(())
}

pub fn toggle_tts_provider_enabled(
    ctx: &AppContext,
    provider_id: String,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    if let Some(p) = settings.tts_provider_mut(&provider_id) {
        p.enabled = enabled;
        settings::write_settings(ctx, settings);
        ctx.notify_setting_changed("tts_provider_enabled", serde_json::json!(&provider_id));
        Ok(())
    } else {
        Err(format!("TTS provider '{provider_id}' not found"))
    }
}

pub fn set_tts_active_provider(ctx: &AppContext, provider_id: String) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    if settings.tts_provider(&provider_id).is_none() {
        return Err(format!("TTS provider '{provider_id}' not found"));
    }
    settings.tts_active_provider_id = provider_id.clone();
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("tts_active_provider", serde_json::json!(&provider_id));
    Ok(())
}

pub fn change_tts_api_key_setting(
    ctx: &AppContext,
    provider_id: String,
    key: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.tts_api_keys.insert(provider_id.clone(), key);
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("tts_api_keys", serde_json::json!(&provider_id));
    Ok(())
}

pub fn change_tts_model_setting(
    ctx: &AppContext,
    provider_id: String,
    model: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    if let Some(p) = settings.tts_provider_mut(&provider_id) {
        p.model = model.clone();
    }
    settings.tts_models.insert(provider_id.clone(), model);
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("tts_models", serde_json::json!(&provider_id));
    Ok(())
}

pub fn change_tts_voice_setting(
    ctx: &AppContext,
    provider_id: String,
    voice: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    if let Some(p) = settings.tts_provider_mut(&provider_id) {
        p.voice = voice.clone();
    }
    settings.tts_voices.insert(provider_id.clone(), voice);
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("tts_voices", serde_json::json!(&provider_id));
    Ok(())
}

pub fn change_tts_base_url_setting(
    ctx: &AppContext,
    provider_id: String,
    base_url: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    if let Some(p) = settings.tts_provider_mut(&provider_id) {
        p.base_url = base_url;
        settings::write_settings(ctx, settings);
        ctx.notify_setting_changed("tts_provider_base_url", serde_json::json!(&provider_id));
        Ok(())
    } else {
        Err(format!("TTS provider '{provider_id}' not found"))
    }
}

pub fn change_tts_timeout_setting(
    ctx: &AppContext,
    provider_id: String,
    timeout_seconds: u32,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    if let Some(p) = settings.tts_provider_mut(&provider_id) {
        p.timeout_seconds = timeout_seconds;
        settings::write_settings(ctx, settings);
        ctx.notify_setting_changed("tts_provider_timeout", serde_json::json!(&provider_id));
        Ok(())
    } else {
        Err(format!("TTS provider '{provider_id}' not found"))
    }
}

pub fn change_tts_speaking_rate_setting(ctx: &AppContext, rate: f32) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.tts_speaking_rate = rate.clamp(0.5, 2.0);
    let applied = settings.tts_speaking_rate;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("tts_speaking_rate", serde_json::json!(applied));
    Ok(())
}

pub fn change_tts_format_setting(
    ctx: &AppContext,
    format: settings::TtsFormat,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.tts_format = format;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("tts_format", serde_json::json!(format!("{format:?}")));
    Ok(())
}

pub fn change_tts_auto_read_chat_setting(ctx: &AppContext, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.tts_auto_read_chat = enabled;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("tts_auto_read_chat", serde_json::json!(enabled));
    Ok(())
}

pub fn change_tts_reader_chunk_chars_setting(ctx: &AppContext, chars: u32) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.tts_reader_chunk_chars = chars.clamp(256, 5000);
    let applied = settings.tts_reader_chunk_chars;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("tts_reader_chunk_chars", serde_json::json!(applied));
    Ok(())
}

pub async fn test_tts_provider_connection(
    ctx: &AppContext,
    provider_id: String,
) -> Result<(usize, u128), String> {
    crate::commands::tts::test_tts_connection(ctx, provider_id)
        .await
        .map_err(|e| e.to_string())
}
