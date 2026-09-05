//! Transcription provider settings (split from `shortcut/mod.rs`; same behavior, same paths).

use crate::context::AppContext;
use crate::settings;

pub fn move_transcription_provider_priority(
    ctx: &AppContext,
    provider_id: &str,
    up: bool,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    let len = settings.transcription_providers.len();
    if len <= 1 {
        return Ok(());
    }

    let Some(index) = settings
        .transcription_providers
        .iter()
        .position(|p| p.id == provider_id)
    else {
        return Err(format!("Provider '{}' not found", provider_id));
    };

    if (up && index == 0) || (!up && index + 1 >= len) {
        return Ok(());
    }

    let target_index = if up { index - 1 } else { index + 1 };
    settings.transcription_providers.swap(index, target_index);
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed(
        "transcription_providers_reordered",
        serde_json::json!(provider_id),
    );
    Ok(())
}

pub fn toggle_transcription_provider_enabled(
    ctx: &AppContext,
    provider_id: String,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    if let Some(p) = settings.transcription_provider_mut(&provider_id) {
        p.enabled = enabled;
        settings::write_settings(ctx, settings);
        ctx.notify_setting_changed(
            "transcription_provider_enabled",
            serde_json::json!(&provider_id),
        );
        Ok(())
    } else {
        Err(format!(
            "Transcription provider '{}' not found",
            provider_id
        ))
    }
}

pub fn change_transcription_api_key_setting(
    ctx: &AppContext,
    provider_id: String,
    key: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings
        .transcription_api_keys
        .insert(provider_id.clone(), key);
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("transcription_api_keys", serde_json::json!(&provider_id));
    Ok(())
}

pub fn change_transcription_model_setting(
    ctx: &AppContext,
    provider_id: String,
    model: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    if let Some(p) = settings.transcription_provider_mut(&provider_id) {
        p.model = model.clone();
    }
    settings
        .transcription_models
        .insert(provider_id.clone(), model);
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("transcription_models", serde_json::json!(&provider_id));
    Ok(())
}

pub fn change_transcription_base_url_setting(
    ctx: &AppContext,
    provider_id: String,
    base_url: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    if let Some(p) = settings.transcription_provider_mut(&provider_id) {
        p.base_url = base_url;
        settings::write_settings(ctx, settings);
        ctx.notify_setting_changed(
            "transcription_provider_base_url",
            serde_json::json!(&provider_id),
        );
        Ok(())
    } else {
        Err(format!(
            "Transcription provider '{}' not found",
            provider_id
        ))
    }
}

pub fn change_transcription_timeout_setting(
    ctx: &AppContext,
    provider_id: String,
    timeout_seconds: u32,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    if let Some(p) = settings.transcription_provider_mut(&provider_id) {
        p.timeout_seconds = timeout_seconds;
        settings::write_settings(ctx, settings);
        ctx.notify_setting_changed(
            "transcription_provider_timeout",
            serde_json::json!(&provider_id),
        );
        Ok(())
    } else {
        Err(format!(
            "Transcription provider '{}' not found",
            provider_id
        ))
    }
}

pub fn update_deepgram_config(
    ctx: &AppContext,
    update_fn: impl FnOnce(&mut settings::DeepgramConfig),
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    if let Some(p) = settings.transcription_provider_mut("deepgram") {
        let mut cfg = p.deepgram.clone().unwrap_or_default();
        update_fn(&mut cfg);
        p.deepgram = Some(cfg);
        settings::write_settings(ctx, settings);
        ctx.notify_setting_changed("deepgram_config", serde_json::json!("deepgram"));
        Ok(())
    } else {
        Err("Transcription provider 'deepgram' not found".to_string())
    }
}

pub async fn test_transcription_provider_connection(
    ctx: &AppContext,
    provider_id: String,
) -> Result<(String, u128), String> {
    let settings = settings::get_settings(ctx);
    let provider = settings
        .transcription_provider(&provider_id)
        .ok_or_else(|| format!("Provider '{}' not found", provider_id))?;

    let api_key = settings
        .transcription_api_keys
        .get(&provider_id)
        .cloned()
        .unwrap_or_default();

    let model = settings
        .transcription_models
        .get(&provider_id)
        .cloned()
        .unwrap_or_else(|| provider.model.clone());

    let language = &settings.selected_language;

    crate::stt_client::test_transcription_provider(provider, api_key, &model, language).await
}
