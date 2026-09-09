//! Read-only web tool catalog + executor for agents.
//!
//! The eight tools wrap [`crate::web_client`] (Tavily search/extract/crawl/
//! map, Firecrawl search/scrape/map/crawl). All are read-only and
//! auto-approved. Credentials and timeouts come from `web_providers` +
//! `web_api_keys` at call time; the configured `timeout_seconds` bounds
//! every network call including crawl polling.

use crate::llm_client::ToolDefinition;
use crate::settings::{AppSettings, AGENT_TOOL_NAMES};
use std::time::Duration;

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
                "Real-time web search via Tavily. Returns answer plus ranked results with URLs. Use time_range for recency, include_domains to restrict sources.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Search query" },
                        "search_depth": { "type": "string", "enum": ["basic", "advanced"], "default": "basic" },
                        "max_results": { "type": "integer", "minimum": 1, "maximum": 10, "default": 5 },
                        "time_range": { "type": "string", "enum": ["day", "week", "month", "year"], "description": "Restrict results to a recency window" },
                        "topic": { "type": "string", "enum": ["general", "news", "finance"], "default": "general" },
                        "include_domains": { "type": "array", "items": { "type": "string" }, "description": "Restrict to these domains (max 5)" },
                        "exclude_domains": { "type": "array", "items": { "type": "string" }, "description": "Exclude these domains (max 5)" }
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
                        "urls": { "type": "array", "items": { "type": "string" }, "description": "URLs to extract (max 5)" },
                        "query": { "type": "string", "description": "Focus extraction on this query" },
                        "extract_depth": { "type": "string", "enum": ["basic", "advanced"], "default": "basic" }
                    },
                    "required": ["urls"]
                }),
            )),
            "tavily_crawl" => defs.push(ToolDefinition::new(
                "tavily_crawl",
                "Crawl a site starting at one URL following Tavily instructions. Returns extracted page content (max 5 pages). Prefer tavily_map first to discover URLs.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "Starting URL to crawl" },
                        "instructions": { "type": "string", "description": "What content to extract from each page" },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 5, "default": 3 }
                    },
                    "required": ["url"]
                }),
            )),
            "tavily_map" => defs.push(ToolDefinition::new(
                "tavily_map",
                "Map a site to its URL graph via Tavily. Cheap discovery step before crawl/extract.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "Site root URL to map" },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 50, "default": 20 }
                    },
                    "required": ["url"]
                }),
            )),
            "firecrawl_search" => defs.push(ToolDefinition::new(
                "firecrawl_search",
                "LLM-ready web search via Firecrawl with markdown snippets.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Search query" },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 10, "default": 5 },
                        "tbs": { "type": "string", "description": "Time-based filter, e.g. 'qdr:d' (past day), 'qdr:w' (past week)" }
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
                        "url": { "type": "string", "description": "URL to scrape" },
                        "only_main_content": { "type": "boolean", "default": true, "description": "Strip nav/ads, keep the article body" }
                    },
                    "required": ["url"]
                }),
            )),
            "firecrawl_map" => defs.push(ToolDefinition::new(
                "firecrawl_map",
                "Map a site to its URL list via Firecrawl. Cheap discovery step before scrape/crawl.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "Site root URL to map" },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 30 }
                    },
                    "required": ["url"]
                }),
            )),
            "firecrawl_crawl" => defs.push(ToolDefinition::new(
                "firecrawl_crawl",
                "Crawl a site via Firecrawl (start URL + discovery limit). Polls until done or the provider timeout; returns page markdowns. Costs more credits than map/scrape.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "Starting URL to crawl" },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 10, "default": 5 }
                    },
                    "required": ["url"]
                }),
            )),
            _ => {}
        }
    }
    defs
}

struct WebCreds {
    base_url: String,
    api_key: String,
    timeout: Duration,
}

fn web_creds(settings: &AppSettings, id: &str) -> Result<WebCreds, String> {
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
    Ok(WebCreds {
        base_url: provider.base_url.clone(),
        api_key,
        timeout: crate::web_client::clamp_timeout_secs(provider.timeout_seconds as u64),
    })
}

fn non_empty_string(args: &serde_json::Value, key: &str, tool: &str) -> Result<String, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{tool} requires a non-empty '{key}'"))
}

fn string_list(args: &serde_json::Value, key: &str, max: usize) -> Vec<String> {
    args.get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .take(max)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
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
            let creds = web_creds(settings, "tavily")?;
            let query = non_empty_string(args, "query", "tavily_search")?;
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
            let time_range = args
                .get("time_range")
                .and_then(|v| v.as_str())
                .filter(|s| ["day", "week", "month", "year"].contains(s))
                .map(str::to_string);
            let topic = args
                .get("topic")
                .and_then(|v| v.as_str())
                .filter(|s| ["general", "news", "finance"].contains(s))
                .map(str::to_string);
            let res = crate::web_client::tavily_search_with_options(
                &creds.base_url,
                &creds.api_key,
                &crate::web_client::TavilySearchOptions {
                    query: query.clone(),
                    search_depth: Some(depth.to_string()),
                    max_results: Some(max),
                    include_answer: Some(true),
                    time_range,
                    topic,
                    include_domains: {
                        let l = string_list(args, "include_domains", 5);
                        if l.is_empty() {
                            None
                        } else {
                            Some(l)
                        }
                    },
                    exclude_domains: {
                        let l = string_list(args, "exclude_domains", 5);
                        if l.is_empty() {
                            None
                        } else {
                            Some(l)
                        }
                    },
                    ..Default::default()
                },
                creds.timeout,
            )
            .await?;
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
            let creds = web_creds(settings, "tavily")?;
            let urls = string_list(args, "urls", 5);
            if urls.is_empty() {
                return Err("tavily_extract requires a non-empty 'urls' array".to_string());
            }
            let depth = args
                .get("extract_depth")
                .and_then(|v| v.as_str())
                .filter(|s| *s == "advanced")
                .unwrap_or("basic");
            let res = crate::web_client::tavily_extract_with_options(
                &creds.base_url,
                &creds.api_key,
                &crate::web_client::TavilyExtractOptions {
                    urls: urls.clone(),
                    query: args
                        .get("query")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string),
                    extract_depth: Some(depth.to_string()),
                    ..Default::default()
                },
                creds.timeout,
            )
            .await?;
            let mut content = String::new();
            for item in &res.results {
                content.push_str(&format!(
                    "## {}\n{}\n{}\n\n",
                    item.url, item.url, item.raw_content
                ));
            }
            if !res.failed_results.is_empty() {
                content.push_str(&format!(
                    "_({} URL(s) failed to extract)_\n",
                    res.failed_results.len()
                ));
            }
            Ok(ToolExecution {
                summary: format!(
                    "tavily_extract {} urls → {} pages",
                    urls.len(),
                    res.results.len()
                ),
                content: content.chars().take(8000).collect(),
            })
        }
        "tavily_crawl" => {
            let creds = web_creds(settings, "tavily")?;
            let url = non_empty_string(args, "url", "tavily_crawl")?;
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(3)
                .clamp(1, 5) as u32;
            let instructions = args
                .get("instructions")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let res = crate::web_client::tavily_crawl(
                &creds.base_url,
                &creds.api_key,
                &url,
                instructions.as_deref(),
                limit,
                creds.timeout,
            )
            .await?;
            let mut content = String::new();
            for item in res.results.iter().take(limit as usize) {
                content.push_str(&format!(
                    "## {}\n{}\n{}\n\n",
                    item.url, item.url, item.raw_content
                ));
            }
            Ok(ToolExecution {
                summary: format!("tavily_crawl {url} → {} pages", res.results.len()),
                content: content.chars().take(8000).collect(),
            })
        }
        "tavily_map" => {
            let creds = web_creds(settings, "tavily")?;
            let url = non_empty_string(args, "url", "tavily_map")?;
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(20)
                .clamp(1, 50) as u32;
            let res = crate::web_client::tavily_map(
                &creds.base_url,
                &creds.api_key,
                &url,
                limit,
                creds.timeout,
            )
            .await?;
            let mut content = String::new();
            for u in res.results.iter().take(limit as usize) {
                content.push_str(&format!("- {u}\n"));
            }
            Ok(ToolExecution {
                summary: format!("tavily_map {url} → {} urls", res.results.len()),
                content: content.chars().take(6000).collect(),
            })
        }
        "firecrawl_search" => {
            let creds = web_creds(settings, "firecrawl")?;
            let query = non_empty_string(args, "query", "firecrawl_search")?;
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(5)
                .clamp(1, 10) as u32;
            let items = crate::web_client::firecrawl_search_with_options(
                &creds.base_url,
                &creds.api_key,
                &crate::web_client::FirecrawlSearchOptions {
                    query: query.clone(),
                    limit: Some(limit),
                    tbs: args
                        .get("tbs")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string),
                    ..Default::default()
                },
                creds.timeout,
            )
            .await?;
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
            let creds = web_creds(settings, "firecrawl")?;
            let url = non_empty_string(args, "url", "firecrawl_scrape")?;
            let only_main = args
                .get("only_main_content")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let data = crate::web_client::firecrawl_scrape_with_options(
                &creds.base_url,
                &creds.api_key,
                &crate::web_client::FirecrawlScrapeOptionsRequest {
                    url: url.clone(),
                    only_main_content: Some(only_main),
                    ..Default::default()
                },
                creds.timeout,
            )
            .await?;
            Ok(ToolExecution {
                summary: format!("firecrawl_scrape {url}"),
                content: data
                    .markdown
                    .unwrap_or_default()
                    .chars()
                    .take(8000)
                    .collect(),
            })
        }
        "firecrawl_map" => {
            let creds = web_creds(settings, "firecrawl")?;
            let url = non_empty_string(args, "url", "firecrawl_map")?;
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(30)
                .clamp(1, 100) as u32;
            let links = crate::web_client::firecrawl_map(
                &creds.base_url,
                &creds.api_key,
                &url,
                limit,
                creds.timeout,
            )
            .await?;
            let mut content = String::new();
            for u in links.iter().take(limit as usize) {
                content.push_str(&format!("- {u}\n"));
            }
            Ok(ToolExecution {
                summary: format!("firecrawl_map {url} → {} urls", links.len()),
                content: content.chars().take(6000).collect(),
            })
        }
        "firecrawl_crawl" => {
            let creds = web_creds(settings, "firecrawl")?;
            let url = non_empty_string(args, "url", "firecrawl_crawl")?;
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(5)
                .clamp(1, 10) as u32;
            let pages = crate::web_client::firecrawl_crawl(
                &creds.base_url,
                &creds.api_key,
                &crate::web_client::FirecrawlCrawlOptions {
                    url: url.clone(),
                    limit: Some(limit),
                    scrape_options: Some(crate::web_client::FirecrawlScrapeOptions {
                        formats: Some(vec!["markdown".to_string()]),
                        only_main_content: Some(true),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                creds.timeout,
            )
            .await?;
            let mut content = String::new();
            for page in pages.iter().take(limit as usize) {
                let body = page.markdown.as_deref().unwrap_or("");
                content.push_str(&format!("## {}\n{}\n{}\n\n", page.url, page.url, body));
            }
            Ok(ToolExecution {
                summary: format!("firecrawl_crawl {url} → {} pages", pages.len()),
                content: content.chars().take(8000).collect(),
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

    #[test]
    fn new_crawl_map_tools_have_definitions() {
        let defs = definitions(
            &[
                "tavily_crawl",
                "tavily_map",
                "firecrawl_map",
                "firecrawl_crawl",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>(),
        );
        assert_eq!(defs.len(), 4);
    }
}
