//! Agent types: resolved provider/model credentials and chat plumbing.
//!
//! Model resolution order is `model_override` →
//! `post_process_models[provider_id]` → error. Providers without an API key
//! are skipped exactly like the post-processing fallback chain, except
//! `custom`/`ollama` which may run keyless.

use crate::settings::{AgentConfig, AppSettings, PostProcessProvider};

/// An agent with its provider, effective model, and API key resolved.
#[derive(Clone, Debug)]
pub struct AgentResolved {
    pub agent: AgentConfig,
    pub provider: PostProcessProvider,
    pub model: String,
    pub api_key: String,
}

/// Chat message roles persisted in `agent_messages`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Assistant,
    Tool,
}

impl ChatRole {
    pub fn as_str(self) -> &'static str {
        match self {
            ChatRole::User => "user",
            ChatRole::Assistant => "assistant",
            ChatRole::Tool => "tool",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "assistant" => ChatRole::Assistant,
            "tool" => ChatRole::Tool,
            _ => ChatRole::User,
        }
    }
}

/// A validated tool call the runner is allowed to execute.
#[derive(Clone, Debug)]
pub struct ResolvedTool {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Resolve an agent id to provider credentials + effective model.
///
/// Returns a human-actionable error naming Settings → Providers when the
/// agent, its provider, or its model cannot be resolved. Unlike
/// post-processing there is no cross-provider fallback: the agent binding is
/// explicit.
pub fn resolve_agent(settings: &AppSettings, agent_id: &str) -> Result<AgentResolved, String> {
    let agent = settings
        .agent(agent_id)
        .ok_or_else(|| format!("Agent '{agent_id}' not found"))?;
    if !agent.enabled {
        return Err(format!("Agent '{}' is disabled", agent.name));
    }
    let provider = settings
        .post_process_provider(&agent.provider_id)
        .ok_or_else(|| {
            format!(
                "Agent '{}' is bound to unknown provider '{}'",
                agent.name, agent.provider_id
            )
        })?;
    if !provider.enabled {
        return Err(format!(
            "Provider '{}' (agent '{}') is disabled. Enable it in Settings → Providers.",
            provider.label, agent.name
        ));
    }

    let model = agent
        .model_override
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_string)
        .or_else(|| {
            settings
                .post_process_models
                .get(&provider.id)
                .cloned()
                .filter(|m| !m.trim().is_empty())
        })
        .ok_or_else(|| {
            format!(
                "No model configured for provider '{}' (agent '{}'). Set one in Settings → Providers or as a model override in Settings → Agents.",
                provider.label, agent.name
            )
        })?;

    let api_key = settings
        .post_process_api_keys
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();
    if api_key.trim().is_empty() && provider.id != "custom" && provider.id != "ollama" {
        return Err(format!(
            "API key is missing for provider '{}' (agent '{}'). Add it in Settings → Providers.",
            provider.label, agent.name
        ));
    }

    Ok(AgentResolved {
        agent: agent.clone(),
        provider: provider.clone(),
        model,
        api_key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::defaults::get_default_settings;

    fn settings_with_agents() -> AppSettings {
        let mut settings = get_default_settings();
        settings
            .post_process_api_keys
            .insert("openai".to_string(), "sk-test".to_string());
        settings
    }

    #[test]
    fn seed_agents_default_to_all_tools_with_step_cap() {
        let settings = settings_with_agents();
        let agent = settings.agent("chat-assistant").expect("seed agent");
        assert_eq!(agent.provider_id, "openai");
        assert!(agent.enabled);
        assert_eq!(
            agent.effective_tools().len(),
            crate::settings::schema::AGENT_TOOL_NAMES.len()
        );
        assert_eq!(agent.effective_max_steps(), 6);
        assert_eq!(agent.effective_top_k(), 4);
        let researcher = settings
            .agent("research-assistant")
            .expect("seed researcher");
        assert_eq!(researcher.effective_max_steps(), 8);
    }

    #[test]
    fn model_override_wins_over_provider_map() {
        let mut settings = settings_with_agents();
        settings
            .post_process_models
            .insert("openai".to_string(), "map-model".to_string());
        let agent = settings
            .agents
            .iter_mut()
            .find(|a| a.id == "chat-assistant")
            .expect("seed agent");
        agent.model_override = Some("override-model".to_string());

        let resolved = resolve_agent(&settings, "chat-assistant").expect("resolves");
        assert_eq!(resolved.model, "override-model");
    }

    #[test]
    fn provider_map_model_used_without_override() {
        let mut settings = settings_with_agents();
        settings
            .post_process_models
            .insert("openai".to_string(), "map-model".to_string());
        settings
            .post_process_api_keys
            .insert("openai".to_string(), "sk-test".to_string());

        let resolved = resolve_agent(&settings, "chat-assistant").expect("resolves");
        assert_eq!(resolved.model, "map-model");
        assert_eq!(resolved.api_key, "sk-test");
    }

    #[test]
    fn missing_model_is_actionable() {
        // Providers without a seeded default (e.g. a custom endpoint) still
        // fail with an actionable message.
        let mut settings = settings_with_agents();
        let agent = settings
            .agents
            .iter_mut()
            .find(|a| a.id == "chat-assistant")
            .expect("seed agent");
        agent.provider_id = "custom".to_string();
        let err = resolve_agent(&settings, "chat-assistant").expect_err("no model seeded");
        assert!(err.contains("No model configured"), "{err}");
    }

    #[test]
    fn unknown_agent_is_actionable() {
        let settings = settings_with_agents();
        let err = resolve_agent(&settings, "nope").expect_err("unknown agent");
        assert!(err.contains("not found"), "{err}");
    }

    #[test]
    fn disabled_provider_is_actionable() {
        let mut settings = settings_with_agents();
        settings
            .post_process_models
            .insert("openai".to_string(), "m".to_string());
        if let Some(p) = settings
            .post_process_providers
            .iter_mut()
            .find(|p| p.id == "openai")
        {
            p.enabled = false;
        }
        let err = resolve_agent(&settings, "chat-assistant").expect_err("disabled");
        assert!(err.contains("disabled"), "{err}");
    }
}
