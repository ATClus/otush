//! Read-only web tool catalog + executor for agents.
//!
//! The four tools wrap [`crate::web_client`] (Tavily search/extract,
//! Firecrawl search/scrape). All are read-only and auto-approved. Credentials
//! come from `web_providers` + `web_api_keys` at call time; timeouts follow
//! the web provider's `timeout_seconds`.

use crate::llm_client::ToolDefinition;
use crate::settings::{AppSettings, AGENT_TOOL_NAMES};

/// Result of executing one tool call.
#[derive(Clone, Debug)]
pub struct ToolExecution {
    /// Markdown-ish payload fed back to the model as `role: "tool"`.
    pub content: String,
    /// One-line summary shown in the overlay timeline.
    pub summary: String,
}

/// JSON Schemas exposed to the model for the agent's enabled tools.
pub fn definitions(enabled_tools: &[String]) -> Vec<ToolDefinition> {
    let mut defs = Vec::new();
    for name in enabled_tools {
        match name.as_str() {
            "tavily_search" => defs.push(ToolDefinition::new(
                "tavily_search",
                "Real-time web search via Tavily. Returns answer plus ranked results with URLs.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Search query" },
                        "search_depth": { "type": "string", "enum": ["basic", "advanced"], "default": "basic" },
                        "max_results": { "type": "integer", "minimum": 1, "maximum": 10, "default": 5 }
                    },
                    "required": ["query"]
                }),
            )),
            "tavily_extract" => defs.push(ToolDefinition::new(
                "tavily_extract",
                "Extract clean article content from explicit web URLs via Tavily.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "urls": { "type": "array", "items": { "type": "string" }, "description": "URLs to extract (max 5)" }
                    },
                    "required": ["urls"]
                }),
            )),
            "firecrawl_search" => defs.push(ToolDefinition::new(
                "firecrawl_search",
                "LLM-ready web search via Firecrawl with markdown snippets.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Search query" },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 10, "default": 5 }
                    },
                    "required": ["query"]
                }),
            )),
            "firecrawl_scrape" => defs.push(ToolDefinition::new(
                "firecrawl_scrape",
                "Scrape one URL into clean Markdown via Firecrawl.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "URL to scrape" }
                    },
                    "required": ["url"]
                }),
            )),
            _ => {}
        }
    }
    defs
}

fn web_creds(settings: &AppSettings, id: &str) -> Result<(String, String), String> {
    let provider = settings
        .web_providers
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| format!("Web provider '{id}' is not configured"))?;
    if !provider.enabled {
        return Err(format!(
            "Web provider '{}' is disabled. Enable it in Settings → Providers.",
            provider.label
        ));
    }
    let api_key = settings.web_api_keys.get(id).cloned().unwrap_or_default();
    if api_key.trim().is_empty() {
        return Err(format!(
            "API key is missing for '{}'. Add it in Settings → Providers.",
            provider.label
        ));
    }
    Ok((provider.base_url.clone(), api_key))
}

/// Execute one validated tool call. Unknown tools are rejected (never sent to
/// the network); provider failures are returned as `Err` so the runner can
/// feed them back to the model as a `tool` message.
pub async fn execute_tool(
    settings: &AppSettings,
    name: &str,
    args: &serde_json::Value,
) -> Result<ToolExecution, String> {
    if !AGENT_TOOL_NAMES.contains(&name) {
        return Err(format!("Tool '{name}' is not available"));
    }
    match name {
        "tavily_search" => {
            let (base_url, api_key) = web_creds(settings, "tavily")?;
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "tavily_search requires a non-empty 'query'".to_string())?;
            let depth = args
                .get("search_depth")
                .and_then(|v| v.as_str())
                .unwrap_or("basic");
            let depth = if depth == "advanced" {
                "advanced"
            } else {
                "basic"
            };
            let max = args
                .get("max_results")
                .and_then(|v| v.as_u64())
                .unwrap_or(5)
                .clamp(1, 10) as u32;
            let res =
                crate::web_client::tavily_search(&base_url, &api_key, query, depth, max).await?;
            let mut content = String::new();
            if let Some(answer) = res.answer.filter(|a| !a.trim().is_empty()) {
                content.push_str(&answer);
                content.push_str("\n\n");
            }
            for item in res.results.iter().take(max as usize) {
                content.push_str(&format!(
                    "- [{}]({})\n{}\n",
                    item.title, item.url, item.content
                ));
            }
            Ok(ToolExecution {
                summary: format!("tavily_search '{query}' → {} results", res.results.len()),
                content: content.chars().take(6000).collect(),
            })
        }
        "tavily_extract" => {
            let (base_url, api_key) = web_creds(settings, "tavily")?;
            let urls: Vec<String> = args
                .get("urls")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .take(5)
                        .map(str::to_string)
                        .collect()
                })
                .filter(|urls: &Vec<String>| !urls.is_empty())
                .ok_or_else(|| "tavily_extract requires a non-empty 'urls' array".to_string())?;
            let items =
                crate::web_client::tavily_extract(&base_url, &api_key, urls.clone()).await?;
            let mut content = String::new();
            for item in &items {
                content.push_str(&format!(
                    "## {}\n{}\n{}\n\n",
                    item.url, item.url, item.raw_content
                ));
            }
            Ok(ToolExecution {
                summary: format!("tavily_extract {} urls → {} pages", urls.len(), items.len()),
                content: content.chars().take(8000).collect(),
            })
        }
        "firecrawl_search" => {
            let (base_url, api_key) = web_creds(settings, "firecrawl")?;
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "firecrawl_search requires a non-empty 'query'".to_string())?;
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(5)
                .clamp(1, 10) as u32;
            let items =
                crate::web_client::firecrawl_search(&base_url, &api_key, query, limit).await?;
            let mut content = String::new();
            for item in items.iter().take(limit as usize) {
                let title = item.title.as_deref().unwrap_or("(untitled)");
                let url = item.url.as_deref().unwrap_or("");
                let body = item
                    .markdown
                    .as_deref()
                    .or(item.description.as_deref())
                    .unwrap_or("");
                content.push_str(&format!("- [{title}]({url})\n{body}\n"));
            }
            Ok(ToolExecution {
                summary: format!("firecrawl_search '{query}' → {} results", items.len()),
                content: content.chars().take(6000).collect(),
            })
        }
        "firecrawl_scrape" => {
            let (base_url, api_key) = web_creds(settings, "firecrawl")?;
            let url = args
                .get("url")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "firecrawl_scrape requires a non-empty 'url'".to_string())?;
            let markdown = crate::web_client::firecrawl_scrape(&base_url, &api_key, url).await?;
            Ok(ToolExecution {
                summary: format!("firecrawl_scrape {url}"),
                content: markdown.chars().take(8000).collect(),
            })
        }
        _ => Err(format!("Tool '{name}' is not available")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitions_cover_all_catalog_tools() {
        let all: Vec<String> = AGENT_TOOL_NAMES.iter().map(|s| s.to_string()).collect();
        let defs = definitions(&all);
        assert_eq!(defs.len(), AGENT_TOOL_NAMES.len());
        for def in &defs {
            let round = serde_json::to_value(serde_json::json!({
                "type": "function",
                "function": { "name": def.name, "description": def.description, "parameters": def.parameters }
            }))
            .expect("tool schema serializes");
            assert_eq!(round["function"]["name"], def.name.as_str());
        }
    }

    #[test]
    fn unknown_tool_names_are_skipped() {
        let defs = definitions(&["tavily_search".to_string(), "nope".to_string()]);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "tavily_search");
    }
}
