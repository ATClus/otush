//! Web provider settings (split from `shortcut/mod.rs`; same behavior, same paths).

use crate::context::AppContext;
use crate::settings;

pub fn toggle_web_provider_enabled(
    ctx: &AppContext,
    provider_id: String,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    if let Some(p) = settings
        .web_providers
        .iter_mut()
        .find(|p| p.id == provider_id)
    {
        p.enabled = enabled;
        settings::write_settings(ctx, settings);
        ctx.notify_setting_changed("web_provider_enabled", serde_json::json!(provider_id));
        Ok(())
    } else {
        Err(format!("Web provider '{}' not found", provider_id))
    }
}

pub fn change_web_provider_api_key_setting(
    ctx: &AppContext,
    provider_id: String,
    api_key: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.web_api_keys.insert(provider_id.clone(), api_key);
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("web_provider_api_key", serde_json::json!(provider_id));
    Ok(())
}

pub fn change_web_provider_base_url_setting(
    ctx: &AppContext,
    provider_id: String,
    base_url: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    if let Some(p) = settings
        .web_providers
        .iter_mut()
        .find(|p| p.id == provider_id)
    {
        p.base_url = base_url;
        settings::write_settings(ctx, settings);
        ctx.notify_setting_changed("web_provider_base_url", serde_json::json!(provider_id));
        Ok(())
    } else {
        Err(format!("Web provider '{}' not found", provider_id))
    }
}

pub fn change_web_provider_timeout_setting(
    ctx: &AppContext,
    provider_id: String,
    timeout_seconds: u32,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    if let Some(p) = settings
        .web_providers
        .iter_mut()
        .find(|p| p.id == provider_id)
    {
        p.timeout_seconds = timeout_seconds;
        settings::write_settings(ctx, settings);
        ctx.notify_setting_changed("web_provider_timeout", serde_json::json!(provider_id));
        Ok(())
    } else {
        Err(format!("Web provider '{}' not found", provider_id))
    }
}

pub async fn test_web_provider_connection(
    ctx: &AppContext,
    provider_id: String,
) -> Result<(String, u128), String> {
    let settings = settings::get_settings(ctx);
    let provider = settings
        .web_providers
        .iter()
        .find(|p| p.id == provider_id)
        .ok_or_else(|| format!("Web provider '{}' not found", provider_id))?;

    let api_key = settings
        .web_api_keys
        .get(&provider_id)
        .cloned()
        .unwrap_or_default();

    match provider.id.as_str() {
        "tavily" => crate::web_client::tavily_test_connection(&provider.base_url, &api_key).await,
        "firecrawl" => {
            crate::web_client::firecrawl_test_connection(&provider.base_url, &api_key).await
        }
        _ => Err(format!("Unknown web provider: {}", provider.id)),
    }
}

pub async fn fetch_llm_provider_models(
    ctx: &AppContext,
    provider_id: String,
) -> Result<Vec<String>, String> {
    let settings = settings::get_settings(ctx);
    let provider = settings
        .post_process_providers
        .iter()
        .find(|p| p.id == provider_id)
        .ok_or_else(|| format!("Provider '{}' not found", provider_id))?;

    let api_key = settings
        .post_process_api_keys
        .get(&provider_id)
        .cloned()
        .unwrap_or_default();

    crate::llm_client::fetch_models(provider, api_key).await
}
