//! Post-processing provider settings (split from `shortcut/mod.rs`; same behavior, same paths).

use crate::context::AppContext;
use crate::settings::{self, LLMPrompt};

pub fn change_post_process_base_url_setting(
    ctx: &AppContext,
    provider_id: String,
    base_url: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    let label = settings
        .post_process_provider(&provider_id)
        .map(|provider| provider.label.clone())
        .ok_or_else(|| format!("Provider '{}' not found", provider_id))?;

    let provider = settings
        .post_process_provider_mut(&provider_id)
        .expect("Provider looked up above must exist");

    if provider.id != "custom" {
        return Err(format!(
            "Provider '{}' does not allow editing the base URL",
            label
        ));
    }

    provider.base_url = base_url;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("post_process_base_url", serde_json::json!(provider_id));
    Ok(())
}

/// Generic helper to validate provider exists
fn validate_provider_exists(
    settings: &settings::AppSettings,
    provider_id: &str,
) -> Result<(), String> {
    if !settings
        .post_process_providers
        .iter()
        .any(|provider| provider.id == provider_id)
    {
        return Err(format!("Provider '{}' not found", provider_id));
    }
    Ok(())
}

pub fn change_post_process_api_key_setting(
    ctx: &AppContext,
    provider_id: String,
    api_key: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    validate_provider_exists(&settings, &provider_id)?;
    settings
        .post_process_api_keys
        .insert(provider_id.clone(), api_key);
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("post_process_api_key", serde_json::json!(provider_id));
    Ok(())
}

pub fn change_post_process_model_setting(
    ctx: &AppContext,
    provider_id: String,
    model: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    validate_provider_exists(&settings, &provider_id)?;
    settings
        .post_process_models
        .insert(provider_id.clone(), model);
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("post_process_model", serde_json::json!(provider_id));
    Ok(())
}

pub fn change_post_process_timeout_setting(
    ctx: &AppContext,
    provider_id: String,
    timeout_seconds: u32,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    validate_provider_exists(&settings, &provider_id)?;
    if let Some(provider) = settings.post_process_provider_mut(&provider_id) {
        provider.timeout_seconds = timeout_seconds;
        settings::write_settings(ctx, settings);
        ctx.notify_setting_changed(
            "post_process_provider_timeout",
            serde_json::json!(provider_id),
        );
    }
    Ok(())
}

pub fn add_post_process_prompt(
    ctx: &AppContext,
    name: String,
    prompt: String,
) -> Result<LLMPrompt, String> {
    let mut settings = settings::get_settings(ctx);

    // Generate unique ID using timestamp and random component
    let id = format!("prompt_{}", chrono::Utc::now().timestamp_millis());

    let new_prompt = LLMPrompt {
        id,
        name,
        prompt,
        preferred_provider_id: None,
    };

    settings.post_process_prompts.push(new_prompt.clone());
    ctx.notify_setting_changed(
        "post_process_prompts_structure",
        serde_json::json!(&settings.post_process_prompts),
    );
    settings::write_settings(ctx, settings);

    Ok(new_prompt)
}

pub fn update_post_process_prompt_name(
    ctx: &AppContext,
    id: String,
    name: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);

    if let Some(existing_prompt) = settings
        .post_process_prompts
        .iter_mut()
        .find(|p| p.id == id)
    {
        existing_prompt.name = name.clone();
        ctx.notify_setting_changed(
            "post_process_prompt_name",
            serde_json::json!({ "id": id, "name": name }),
        );
        settings::write_settings(ctx, settings);
        Ok(())
    } else {
        Err(format!("Prompt with id '{}' not found", id))
    }
}

pub fn update_post_process_prompt_content(
    ctx: &AppContext,
    id: String,
    prompt: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);

    if let Some(existing_prompt) = settings
        .post_process_prompts
        .iter_mut()
        .find(|p| p.id == id)
    {
        existing_prompt.prompt = prompt;
        ctx.notify_setting_changed("post_process_prompt_content", serde_json::json!(&id));
        settings::write_settings(ctx, settings);
        Ok(())
    } else {
        Err(format!("Prompt with id '{}' not found", id))
    }
}

pub fn delete_post_process_prompt(ctx: &AppContext, id: String) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);

    // Don't allow deleting the last prompt
    if settings.post_process_prompts.len() <= 1 {
        return Err("Cannot delete the last prompt".to_string());
    }

    // Find and remove the prompt
    let original_len = settings.post_process_prompts.len();
    settings.post_process_prompts.retain(|p| p.id != id);

    if settings.post_process_prompts.len() == original_len {
        return Err(format!("Prompt with id '{}' not found", id));
    }

    // If the deleted prompt was selected, select the first one or None
    if settings.post_process_selected_prompt_id.as_ref() == Some(&id) {
        settings.post_process_selected_prompt_id =
            settings.post_process_prompts.first().map(|p| p.id.clone());
    }

    ctx.notify_setting_changed(
        "post_process_prompts_structure",
        serde_json::json!(&settings.post_process_prompts),
    );
    settings::write_settings(ctx, settings);
    Ok(())
}

pub fn remove_post_process_prompt(ctx: &AppContext, id: String) -> Result<(), String> {
    delete_post_process_prompt(ctx, id)
}

pub fn set_post_process_selected_prompt(ctx: &AppContext, id: String) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);

    // Verify the prompt exists
    if !settings.post_process_prompts.iter().any(|p| p.id == id) {
        return Err(format!("Prompt with id '{}' not found", id));
    }

    settings.post_process_selected_prompt_id = Some(id);
    ctx.notify_setting_changed(
        "post_process_selected_prompt_id",
        serde_json::json!(&settings.post_process_selected_prompt_id),
    );
    settings::write_settings(ctx, settings);
    Ok(())
}

pub fn set_post_process_prompt_preferred_provider(
    ctx: &AppContext,
    prompt_id: String,
    preferred_provider_id: Option<String>,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);

    if let Some(prompt) = settings
        .post_process_prompts
        .iter_mut()
        .find(|p| p.id == prompt_id)
    {
        prompt.preferred_provider_id = preferred_provider_id;
        ctx.notify_setting_changed(
            "post_process_prompts",
            serde_json::json!(&settings.post_process_prompts),
        );
        settings::write_settings(ctx, settings);
        Ok(())
    } else {
        Err(format!("Prompt with id '{}' not found", prompt_id))
    }
}

pub fn move_post_process_provider_priority(
    ctx: &AppContext,
    provider_id: &str,
    up: bool,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    let len = settings.post_process_providers.len();
    if len <= 1 {
        return Ok(());
    }

    let Some(index) = settings
        .post_process_providers
        .iter()
        .position(|p| p.id == provider_id)
    else {
        return Err(format!("Provider '{}' not found", provider_id));
    };

    if up && index > 0 {
        settings.post_process_providers.swap(index, index - 1);
    } else if !up && index + 1 < len {
        settings.post_process_providers.swap(index, index + 1);
    } else {
        return Ok(());
    }

    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("post_process_providers", serde_json::json!("reordered"));
    ctx.notify_setting_changed(
        "post_process_providers_reordered",
        serde_json::json!(provider_id),
    );
    Ok(())
}

pub fn toggle_post_process_provider_enabled(
    ctx: &AppContext,
    provider_id: String,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    if let Some(p) = settings
        .post_process_providers
        .iter_mut()
        .find(|p| p.id == provider_id)
    {
        p.enabled = enabled;
        settings::write_settings(ctx, settings);
        ctx.notify_setting_changed("post_process_providers", serde_json::json!(&provider_id));
        ctx.notify_setting_changed(
            "post_process_provider_enabled",
            serde_json::json!(&provider_id),
        );
        Ok(())
    } else {
        Err(format!("Provider '{}' not found", provider_id))
    }
}

pub fn set_post_process_provider_reasoning(
    ctx: &AppContext,
    provider_id: String,
    effort: crate::settings::ReasoningEffort,
    budget_tokens: Option<u32>,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    if let Some(p) = settings
        .post_process_providers
        .iter_mut()
        .find(|p| p.id == provider_id)
    {
        p.reasoning = crate::settings::ProviderReasoningConfig {
            effort,
            budget_tokens,
        };
        settings::write_settings(ctx, settings);
        ctx.notify_setting_changed("post_process_providers", serde_json::json!(provider_id));
        Ok(())
    } else {
        Err(format!("Provider '{}' not found", provider_id))
    }
}

pub async fn test_post_process_provider_connection(
    ctx: &AppContext,
    provider_id: String,
) -> Result<(String, u128), String> {
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

    let model = settings
        .post_process_models
        .get(&provider_id)
        .cloned()
        .unwrap_or_else(|| crate::settings::default_model_for_provider(&provider_id));

    if model.is_empty() {
        return Err("No model selected for this provider".to_string());
    }

    crate::llm_client::test_provider_connection(provider, api_key, &model).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{AppContext, AppPaths, EventBus};
    use crate::managers::audio::AudioRecordingManager;
    use crate::managers::history::HistoryManager;
    use crate::managers::model::ModelManager;
    use crate::managers::transcription::TranscriptionManager;
    use crate::TranscriptionCoordinator;
    use std::sync::Arc;

    fn create_test_context() -> (AppContext, tempfile::TempDir) {
        let temp_dir = tempfile::tempdir().unwrap();
        let paths = AppPaths {
            data_dir: temp_dir.path().to_path_buf(),
            resource_dir: temp_dir.path().to_path_buf(),
            log_dir: temp_dir.path().to_path_buf(),
        };
        paths.ensure_dirs().unwrap();
        let bus = EventBus::new();
        let model = Arc::new(ModelManager::new(&paths, bus.clone()).unwrap());
        let transcription =
            Arc::new(TranscriptionManager::new(&paths, bus.clone(), model.clone()).unwrap());
        let audio = Arc::new(
            AudioRecordingManager::new(&paths, bus.clone(), transcription.stream_router()).unwrap(),
        );
        let history = Arc::new(HistoryManager::new(&paths, bus.clone()).unwrap());
        let coordinator = Arc::new(TranscriptionCoordinator::new());
        let ctx = AppContext {
            paths,
            bus,
            model,
            transcription,
            audio,
            history,
            coordinator,
        };
        (ctx, temp_dir)
    }

    #[test]
    fn test_update_prompt_name_and_content_independent() {
        let (ctx, _dir) = create_test_context();
        let new_prompt = add_post_process_prompt(
            &ctx,
            "Initial Name".to_string(),
            "Initial content: ${output}".to_string(),
        )
        .unwrap();

        assert_eq!(new_prompt.name, "Initial Name");
        assert_eq!(new_prompt.prompt, "Initial content: ${output}");

        // 1. Update only name
        update_post_process_prompt_name(&ctx, new_prompt.id.clone(), "Updated Name".to_string())
            .unwrap();

        let s = settings::get_settings(&ctx);
        let p = s
            .post_process_prompts
            .iter()
            .find(|p| p.id == new_prompt.id)
            .unwrap();
        assert_eq!(p.name, "Updated Name");
        assert_eq!(p.prompt, "Initial content: ${output}");

        // 2. Update only content
        update_post_process_prompt_content(
            &ctx,
            new_prompt.id.clone(),
            "Brand new instructions: ${output}".to_string(),
        )
        .unwrap();

        let s2 = settings::get_settings(&ctx);
        let p2 = s2
            .post_process_prompts
            .iter()
            .find(|p| p.id == new_prompt.id)
            .unwrap();
        assert_eq!(p2.name, "Updated Name");
        assert_eq!(p2.prompt, "Brand new instructions: ${output}");
    }
}
