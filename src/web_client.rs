//! Web & Document Intelligence Client for Tavily and Firecrawl APIs.
//!
//! Provides native integration with:
//! - Tavily: Real-time search, deep research, web page content extraction,
//!   crawl, and map (`https://api.tavily.com`)
//! - Firecrawl: LLM-ready web search, markdown scraping, document parsing
//!   (PDF, DOCX), site mapping, and crawling (`https://api.firecrawl.dev/v2`)
//!
//! Auth (current per official SDKs):
//! - Tavily: `Authorization: Bearer <key>` header. The legacy `api_key` body
//!   field is still sent alongside for backward compatibility with older
//!   deployments, but the header is authoritative.
//! - Firecrawl: `Authorization: Bearer <key>` header.
//!
//! Timeouts: every public function takes an explicit `timeout` (`Duration`)
//! so the configured `WebProvider.timeout_seconds` is honored end-to-end.
//! Thin `*_with_default_timeout` wrappers preserve the old call shape for
//! direct UI callers during migration.

use log::info;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{Duration, Instant};

const USER_AGENT_VALUE: &str = "Otush/1.0 (+https://github.com/ATClus/otush)";
const TAVILY_DEFAULT_BASE: &str = "https://api.tavily.com";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// Clamp a provider timeout (seconds) into a sane 5s..=300s window.
pub fn clamp_timeout_secs(timeout_secs: u64) -> Duration {
    Duration::from_secs(timeout_secs.clamp(5, 300))
}

fn http_client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))
}

fn tavily_base(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        TAVILY_DEFAULT_BASE.to_string()
    } else {
        trimmed.to_string()
    }
}

fn tavily_headers(api_key: &str) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", api_key.trim()))
            .map_err(|e| format!("Invalid Tavily API key header: {e}"))?,
    );
    headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE));
    Ok(headers)
}

fn firecrawl_headers(api_key: &str) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", api_key.trim()))
            .map_err(|e| format!("Invalid Firecrawl API key header: {e}"))?,
    );
    headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE));
    Ok(headers)
}

fn check_api_key(api_key: &str, provider: &str) -> Result<(), String> {
    if api_key.trim().is_empty() {
        return Err(format!(
            "{provider} API key is missing. Please configure it in Settings -> Providers."
        ));
    }
    Ok(())
}

// ============================================================================
// Tavily Models & Functions
// ============================================================================

/// Options for `POST /search`, mirroring the official Tavily SDK surface.
/// Only `query` is required; every other field is omitted when `None` so
/// server-side defaults apply.
#[derive(Debug, Serialize, Clone, Default)]
pub struct TavilySearchOptions {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_depth: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_range: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub days: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_domains: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_domains: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_answer: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_images: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_raw_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct TavilySearchResultItem {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    pub published_date: Option<String>,
    #[serde(default)]
    pub favicon: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct TavilySearchResponse {
    pub query: Option<String>,
    pub answer: Option<String>,
    #[serde(default)]
    pub results: Vec<TavilySearchResultItem>,
    #[serde(default)]
    pub images: Vec<Value>,
}

#[derive(Debug, Serialize, Clone, Default)]
pub struct TavilyExtractOptions {
    pub urls: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extract_depth: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_images: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunks_per_source: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct TavilyExtractItem {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub raw_content: String,
    #[serde(default)]
    pub images: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct TavilyExtractResponse {
    #[serde(default)]
    pub results: Vec<TavilyExtractItem>,
    #[serde(default)]
    pub failed_results: Vec<Value>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct TavilyCrawlResponse {
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub results: Vec<TavilyExtractItem>,
    #[serde(default)]
    pub failed_results: Vec<Value>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct TavilyMapResponse {
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub results: Vec<String>,
}

/// Execute a real-time web search via Tavily (`POST /search`).
///
/// Sends both the `Authorization: Bearer` header (current per the official
/// SDK) and the legacy `api_key` body field for backward compatibility.
pub async fn tavily_search_with_options(
    base_url: &str,
    api_key: &str,
    options: &TavilySearchOptions,
    timeout: Duration,
) -> Result<TavilySearchResponse, String> {
    check_api_key(api_key, "Tavily")?;
    if options.query.trim().is_empty() {
        return Err("Tavily search requires a non-empty query".to_string());
    }
    let base = tavily_base(base_url);
    let url = format!("{}/search", base);
    let client = http_client(timeout)?;

    let mut payload = serde_json::to_value(options)
        .map_err(|e| format!("Failed to serialize Tavily search options: {e}"))?;
    // Legacy body auth kept alongside the Bearer header.
    payload["api_key"] = Value::String(api_key.to_string());
    if let Some(max) = payload.get("max_results").and_then(|v| v.as_u64()) {
        payload["max_results"] = Value::from(max.clamp(1, 20));
    }

    info!(
        "Executing Tavily search for query: '{}' (depth: {})",
        options.query,
        options.search_depth.as_deref().unwrap_or("default")
    );

    let response = client
        .post(&url)
        .headers(tavily_headers(api_key)?)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Tavily search network request failed: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        let err_text = response.text().await.unwrap_or_default();
        return Err(format!("Tavily API error ({status}): {err_text}"));
    }

    let search_res: TavilySearchResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Tavily search response JSON: {e}"))?;

    Ok(search_res)
}

/// Legacy search signature (basic/advanced + max results), now backed by
/// [`tavily_search_with_options`] with `include_answer: true`.
pub async fn tavily_search(
    base_url: &str,
    api_key: &str,
    query: &str,
    search_depth: &str,
    max_results: u32,
) -> Result<TavilySearchResponse, String> {
    tavily_search_with_options(
        base_url,
        api_key,
        &TavilySearchOptions {
            query: query.to_string(),
            search_depth: Some(search_depth.to_string()),
            max_results: Some(max_results.clamp(1, 20)),
            include_answer: Some(true),
            ..Default::default()
        },
        DEFAULT_TIMEOUT,
    )
    .await
}

/// Perform deep research investigation via Tavily advanced search.
///
/// NOTE: this is a single `advanced`-depth search pass, not the async
/// Tavily Research API (`POST /research`). Kept under this name because
/// the Search/Research UIs call it; prefer `tavily_search_with_options`
/// with `search_depth: "advanced"` for new code.
pub async fn tavily_deep_research(
    base_url: &str,
    api_key: &str,
    query: &str,
) -> Result<TavilySearchResponse, String> {
    // Advanced search depth queries multiple sources and produces a high-level answer
    tavily_search(base_url, api_key, query, "advanced", 10).await
}

/// Extract clean content from a list of web URLs via Tavily (`POST /extract`).
pub async fn tavily_extract_with_options(
    base_url: &str,
    api_key: &str,
    options: &TavilyExtractOptions,
    timeout: Duration,
) -> Result<TavilyExtractResponse, String> {
    check_api_key(api_key, "Tavily")?;
    if options.urls.is_empty() {
        return Err("Tavily extract requires at least one URL".to_string());
    }
    let base = tavily_base(base_url);
    let url = format!("{}/extract", base);
    let client = http_client(timeout)?;

    let mut payload = serde_json::to_value(options)
        .map_err(|e| format!("Failed to serialize Tavily extract options: {e}"))?;
    payload["api_key"] = Value::String(api_key.to_string());

    let response = client
        .post(&url)
        .headers(tavily_headers(api_key)?)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Tavily extract network request failed: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        let err_text = response.text().await.unwrap_or_default();
        return Err(format!("Tavily extract error ({status}): {err_text}"));
    }

    let extract_res: TavilyExtractResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Tavily extract response JSON: {e}"))?;

    Ok(extract_res)
}

/// Legacy extract signature returning just the pages.
pub async fn tavily_extract(
    base_url: &str,
    api_key: &str,
    urls: Vec<String>,
) -> Result<Vec<TavilyExtractItem>, String> {
    let res = tavily_extract_with_options(
        base_url,
        api_key,
        &TavilyExtractOptions {
            urls,
            ..Default::default()
        },
        DEFAULT_TIMEOUT,
    )
    .await?;
    if res.results.is_empty() && !res.failed_results.is_empty() {
        return Err(format!(
            "Tavily extract failed for all {} URL(s)",
            res.failed_results.len()
        ));
    }
    Ok(res.results)
}

/// Crawl a site starting at `url` with natural-language `instructions`
/// (`POST /crawl`). Returns extracted pages; partial failures surface in
/// `failed_results` without failing the whole call.
pub async fn tavily_crawl(
    base_url: &str,
    api_key: &str,
    url: &str,
    instructions: Option<&str>,
    limit: u32,
    timeout: Duration,
) -> Result<TavilyCrawlResponse, String> {
    check_api_key(api_key, "Tavily")?;
    if url.trim().is_empty() {
        return Err("Tavily crawl requires a non-empty URL".to_string());
    }
    let base = tavily_base(base_url);
    let endpoint = format!("{}/crawl", base);
    let client = http_client(timeout)?;

    let payload = serde_json::json!({
        "api_key": api_key,
        "url": url.trim(),
        "instructions": instructions.unwrap_or("Extract the main content of each page"),
        "limit": limit.clamp(1, 20),
        "extract_depth": "basic",
    });

    info!("Executing Tavily crawl for: '{url}' (limit: {limit})");

    let response = client
        .post(&endpoint)
        .headers(tavily_headers(api_key)?)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Tavily crawl network request failed: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        let err_text = response.text().await.unwrap_or_default();
        return Err(format!("Tavily crawl error ({status}): {err_text}"));
    }

    response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Tavily crawl response JSON: {e}"))
}

/// Map a site to its URL graph (`POST /map`). Returns discovered URLs.
pub async fn tavily_map(
    base_url: &str,
    api_key: &str,
    url: &str,
    limit: u32,
    timeout: Duration,
) -> Result<TavilyMapResponse, String> {
    check_api_key(api_key, "Tavily")?;
    if url.trim().is_empty() {
        return Err("Tavily map requires a non-empty URL".to_string());
    }
    let base = tavily_base(base_url);
    let endpoint = format!("{}/map", base);
    let client = http_client(timeout)?;

    let payload = serde_json::json!({
        "api_key": api_key,
        "url": url.trim(),
        "limit": limit.clamp(1, 100),
    });

    info!("Executing Tavily map for: '{url}'");

    let response = client
        .post(&endpoint)
        .headers(tavily_headers(api_key)?)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Tavily map network request failed: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        let err_text = response.text().await.unwrap_or_default();
        return Err(format!("Tavily map error ({status}): {err_text}"));
    }

    // The API returns `{ results: [...] }` where entries may be plain URL
    // strings or `{ url }` objects; normalize both.
    let val: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Tavily map response JSON: {e}"))?;
    let mut results = Vec::new();
    if let Some(arr) = val.get("results").and_then(|v| v.as_array()) {
        for it in arr {
            if let Some(s) = it.as_str() {
                results.push(s.to_string());
            } else if let Some(s) = it.get("url").and_then(|v| v.as_str()) {
                results.push(s.to_string());
            }
        }
    }
    Ok(TavilyMapResponse {
        base_url: val
            .get("base_url")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        results,
    })
}

/// Test connection and measure latency to Tavily API.
pub async fn tavily_test_connection(
    base_url: &str,
    api_key: &str,
) -> Result<(String, u128), String> {
    tavily_test_connection_with_timeout(base_url, api_key, DEFAULT_TIMEOUT).await
}

pub async fn tavily_test_connection_with_timeout(
    base_url: &str,
    api_key: &str,
    timeout: Duration,
) -> Result<(String, u128), String> {
    let start = Instant::now();
    let res = tavily_search_with_options(
        base_url,
        api_key,
        &TavilySearchOptions {
            query: "Otush test ping".to_string(),
            search_depth: Some("basic".to_string()),
            max_results: Some(1),
            ..Default::default()
        },
        timeout,
    )
    .await?;
    let elapsed = start.elapsed().as_millis();
    let sample_title = res
        .results
        .first()
        .map(|r| r.title.clone())
        .unwrap_or_else(|| "Connected successfully".to_string());
    Ok((sample_title, elapsed))
}

// ============================================================================
// Firecrawl Models & Functions (v2 & v1 compatible)
// ============================================================================

/// Normalizes the Firecrawl base URL to ensure v2 compatibility.
pub fn normalize_firecrawl_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty()
        || trimmed == "https://api.firecrawl.dev"
        || trimmed == "https://api.firecrawl.dev/v1"
    {
        "https://api.firecrawl.dev/v2".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Scrape options for Firecrawl search/scrape, mirroring the v2 SDK
/// (`scrapeOptions`, lowercase `s`). Only `Some` fields hit the wire.
#[derive(Debug, Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct FirecrawlScrapeOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formats: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub only_main_content: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait_for: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_age: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mobile: Option<bool>,
}

/// Kept for the serialization unit test + backward compat; prefer
/// [`FirecrawlScrapeOptions`] for new code.
#[derive(Debug, Serialize, Clone)]
pub struct FirecrawlSearchRequest {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(rename = "scrapeOptions", skip_serializing_if = "Option::is_none")]
    pub scrape_options: Option<FirecrawlScrapeOptions>,
}

/// Options for `POST /search` (v2): `sources` defaults to web search with
/// markdown snippets; `categories`, `tbs`, and `limit` narrow results.
#[derive(Debug, Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct FirecrawlSearchOptions {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub categories: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tbs: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignore_invalid_urls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scrape_options: Option<FirecrawlScrapeOptions>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct FirecrawlSearchResultItem {
    pub url: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub markdown: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct FirecrawlSearchResponse {
    pub success: bool,
    #[serde(default)]
    pub data: Vec<FirecrawlSearchResultItem>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct FirecrawlScrapeRequest {
    pub url: String,
    pub formats: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct FirecrawlScrapeData {
    pub markdown: Option<String>,
    pub metadata: Option<Value>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct FirecrawlScrapeResponse {
    pub success: bool,
    pub data: Option<FirecrawlScrapeData>,
    pub error: Option<String>,
}

/// Options for `POST /scrape` (v2). `formats` defaults to `["markdown"]`.
#[derive(Debug, Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct FirecrawlScrapeOptionsRequest {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formats: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub only_main_content: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait_for: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_age: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mobile: Option<bool>,
}

/// One scraped page from a crawl job.
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct FirecrawlCrawlPage {
    #[serde(default)]
    pub url: String,
    pub markdown: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
}

/// Options for `POST /crawl` (v2): discovery limit + scrape behavior.
#[derive(Debug, Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct FirecrawlCrawlOptions {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_discovery_depth: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_paths: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_paths: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scrape_options: Option<FirecrawlScrapeOptions>,
}

fn firecrawl_check_success(val: &Value, what: &str) -> Result<(), String> {
    let success = val.get("success").and_then(|v| v.as_bool()).unwrap_or(true);
    if !success {
        let err = val.get("error").and_then(|v| v.as_str()).unwrap_or(what);
        return Err(err.to_string());
    }
    Ok(())
}

fn firecrawl_items_from_search_value(val: &Value) -> Vec<FirecrawlSearchResultItem> {
    let mut results = Vec::new();
    // Firecrawl v2: `data` is an object (`{ web: [...], news?, images? }`).
    if let Some(data) = val.get("data") {
        if let Some(obj) = data.as_object() {
            for key in ["web", "news", "images"] {
                if let Some(arr) = obj.get(key).and_then(|v| v.as_array()) {
                    for it in arr {
                        if let Ok(item) =
                            serde_json::from_value::<FirecrawlSearchResultItem>(it.clone())
                        {
                            results.push(item);
                        }
                    }
                }
            }
            if !results.is_empty() {
                return results;
            }
        }
        // Firecrawl v1: `data` is a bare array.
        if let Some(arr) = data.as_array() {
            for it in arr {
                if let Ok(item) = serde_json::from_value::<FirecrawlSearchResultItem>(it.clone()) {
                    results.push(item);
                }
            }
        }
    }
    results
}

/// Perform web search and markdown retrieval via Firecrawl (`POST /search`).
pub async fn firecrawl_search_with_options(
    base_url: &str,
    api_key: &str,
    options: &FirecrawlSearchOptions,
    timeout: Duration,
) -> Result<Vec<FirecrawlSearchResultItem>, String> {
    check_api_key(api_key, "Firecrawl")?;
    if options.query.trim().is_empty() {
        return Err("Firecrawl search requires a non-empty query".to_string());
    }

    let base = normalize_firecrawl_base_url(base_url);
    let url = format!("{}/search", base);
    let client = http_client(timeout)?;

    let mut payload = serde_json::to_value(options)
        .map_err(|e| format!("Failed to serialize Firecrawl search options: {e}"))?;
    if let Some(limit) = payload.get("limit").and_then(|v| v.as_u64()) {
        payload["limit"] = Value::from(limit.clamp(1, 20));
    }
    // Default: web search with markdown snippets (matches previous behavior
    // of `scrapeOptions: { formats: ["markdown"] }`).
    if payload.get("sources").is_none() {
        payload["sources"] = serde_json::json!([{"type": "web"}]);
    }
    if payload.get("scrapeOptions").is_none() {
        payload["scrapeOptions"] = serde_json::json!({ "formats": ["markdown"] });
    }

    info!("Executing Firecrawl search for: '{}'", options.query);

    let response = client
        .post(&url)
        .headers(firecrawl_headers(api_key)?)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Firecrawl search network request failed: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        let err_text = response.text().await.unwrap_or_default();
        return Err(format!("Firecrawl API error ({status}): {err_text}"));
    }

    let val: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Firecrawl search response JSON: {e}"))?;

    firecrawl_check_success(&val, "Firecrawl search indicated failure")?;
    Ok(firecrawl_items_from_search_value(&val))
}

/// Legacy search signature (query + limit) with default web/markdown options.
pub async fn firecrawl_search(
    base_url: &str,
    api_key: &str,
    query: &str,
    limit: u32,
) -> Result<Vec<FirecrawlSearchResultItem>, String> {
    firecrawl_search_with_options(
        base_url,
        api_key,
        &FirecrawlSearchOptions {
            query: query.to_string(),
            limit: Some(limit.clamp(1, 20)),
            ..Default::default()
        },
        DEFAULT_TIMEOUT,
    )
    .await
}

/// Scrape a URL into markdown + metadata via Firecrawl (`POST /scrape`).
pub async fn firecrawl_scrape_with_options(
    base_url: &str,
    api_key: &str,
    options: &FirecrawlScrapeOptionsRequest,
    timeout: Duration,
) -> Result<FirecrawlScrapeData, String> {
    check_api_key(api_key, "Firecrawl")?;
    if options.url.trim().is_empty() {
        return Err("Firecrawl scrape requires a non-empty URL".to_string());
    }

    let base = normalize_firecrawl_base_url(base_url);
    let url = format!("{}/scrape", base);
    let client = http_client(timeout)?;

    let mut payload = serde_json::to_value(options)
        .map_err(|e| format!("Failed to serialize Firecrawl scrape options: {e}"))?;
    if payload.get("formats").is_none() {
        payload["formats"] = serde_json::json!(["markdown"]);
    }

    let response = client
        .post(&url)
        .headers(firecrawl_headers(api_key)?)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Firecrawl scrape network request failed: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        let err_text = response.text().await.unwrap_or_default();
        return Err(format!("Firecrawl scrape error ({status}): {err_text}"));
    }

    let parsed_json: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Firecrawl scrape response JSON: {e}"))?;

    firecrawl_check_success(&parsed_json, "Firecrawl scrape indicated failure")?;

    let data: FirecrawlScrapeData =
        serde_json::from_value(parsed_json.get("data").cloned().unwrap_or(Value::Null))
            .map_err(|e| format!("Failed to parse Firecrawl scrape data: {e}"))?;
    // Some deployments return a bare `{ markdown }` envelope instead.
    if data.markdown.is_none() {
        if let Some(md) = parsed_json.pointer("/markdown").and_then(|v| v.as_str()) {
            return Ok(FirecrawlScrapeData {
                markdown: Some(md.to_string()),
                metadata: data.metadata,
            });
        }
    }
    Ok(data)
}

/// Scrape a specific URL and return LLM-ready clean Markdown via Firecrawl.
pub async fn firecrawl_scrape(
    base_url: &str,
    api_key: &str,
    target_url: &str,
) -> Result<String, String> {
    let data = firecrawl_scrape_with_options(
        base_url,
        api_key,
        &FirecrawlScrapeOptionsRequest {
            url: target_url.to_string(),
            ..Default::default()
        },
        DEFAULT_TIMEOUT,
    )
    .await?;
    Ok(data.markdown.unwrap_or_default())
}

/// Map a site to its URL graph via Firecrawl (`POST /map`).
pub async fn firecrawl_map(
    base_url: &str,
    api_key: &str,
    url: &str,
    limit: u32,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    check_api_key(api_key, "Firecrawl")?;
    if url.trim().is_empty() {
        return Err("Firecrawl map requires a non-empty URL".to_string());
    }

    let base = normalize_firecrawl_base_url(base_url);
    let endpoint = format!("{}/map", base);
    let client = http_client(timeout)?;

    let payload = serde_json::json!({ "url": url.trim(), "limit": limit.clamp(1, 500) });

    info!("Executing Firecrawl map for: '{url}'");

    let response = client
        .post(&endpoint)
        .headers(firecrawl_headers(api_key)?)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Firecrawl map network request failed: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        let err_text = response.text().await.unwrap_or_default();
        return Err(format!("Firecrawl map error ({status}): {err_text}"));
    }

    let val: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Firecrawl map response JSON: {e}"))?;
    firecrawl_check_success(&val, "Firecrawl map indicated failure")?;

    let mut links = Vec::new();
    if let Some(arr) = val.get("links").and_then(|v| v.as_array()) {
        for it in arr {
            if let Some(s) = it.as_str() {
                links.push(s.to_string());
            } else if let Some(s) = it.get("url").and_then(|v| v.as_str()) {
                links.push(s.to_string());
            }
        }
    }
    Ok(links)
}

/// Crawl a site via Firecrawl (`POST /crawl` + `GET /crawl/{id}` polling).
///
/// Starts the job then polls every 2s until `completed`/`failed` or the
/// `timeout` deadline, returning whatever pages finished in time. A
/// deadline expiry is surfaced as an `Err` naming the pages recovered so
/// callers can decide whether partial output is usable.
pub async fn firecrawl_crawl(
    base_url: &str,
    api_key: &str,
    options: &FirecrawlCrawlOptions,
    timeout: Duration,
) -> Result<Vec<FirecrawlCrawlPage>, String> {
    check_api_key(api_key, "Firecrawl")?;
    if options.url.trim().is_empty() {
        return Err("Firecrawl crawl requires a non-empty URL".to_string());
    }
    let base = normalize_firecrawl_base_url(base_url);
    let client = http_client(timeout)?;
    let headers = firecrawl_headers(api_key)?;
    let deadline = Instant::now() + timeout;

    let start_payload = serde_json::to_value(options)
        .map_err(|e| format!("Failed to serialize Firecrawl crawl options: {e}"))?;

    info!(
        "Starting Firecrawl crawl for: '{}' (limit: {})",
        options.url,
        options.limit.unwrap_or(10)
    );

    let start_res = client
        .post(format!("{}/crawl", base))
        .headers(headers.clone())
        .json(&start_payload)
        .send()
        .await
        .map_err(|e| format!("Firecrawl crawl start failed: {e}"))?;
    let status = start_res.status();
    if !status.is_success() {
        let err_text = start_res.text().await.unwrap_or_default();
        return Err(format!("Firecrawl crawl error ({status}): {err_text}"));
    }
    let start_val: Value = start_res
        .json()
        .await
        .map_err(|e| format!("Failed to parse Firecrawl crawl response JSON: {e}"))?;
    firecrawl_check_success(&start_val, "Firecrawl crawl indicated failure")?;
    let job_id = start_val
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Firecrawl crawl returned no job id".to_string())?;

    loop {
        if Instant::now() >= deadline {
            return Err(format!(
                "Firecrawl crawl timed out after {}s (job {job_id}). Raise the provider timeout in Settings → Providers for larger crawls.",
                timeout.as_secs()
            ));
        }
        tokio::time::sleep(Duration::from_secs(2)).await;

        let poll = client
            .get(format!("{}/crawl/{}", base, job_id))
            .headers(headers.clone())
            .send()
            .await
            .map_err(|e| format!("Firecrawl crawl poll failed: {e}"))?;
        let status = poll.status();
        if !status.is_success() {
            let err_text = poll.text().await.unwrap_or_default();
            return Err(format!("Firecrawl crawl poll error ({status}): {err_text}"));
        }
        let val: Value = poll
            .json()
            .await
            .map_err(|e| format!("Failed to parse Firecrawl crawl status JSON: {e}"))?;
        firecrawl_check_success(&val, "Firecrawl crawl job failed")?;
        let job_status = val
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("scraping");

        let mut pages = Vec::new();
        if let Some(arr) = val.get("data").and_then(|v| v.as_array()) {
            for it in arr {
                let url = it
                    .pointer("/metadata/sourceURL")
                    .and_then(|v| v.as_str())
                    .or_else(|| it.get("url").and_then(|v| v.as_str()))
                    .unwrap_or_default()
                    .to_string();
                pages.push(FirecrawlCrawlPage {
                    url,
                    markdown: it
                        .get("markdown")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    title: it
                        .pointer("/metadata/title")
                        .and_then(|v| v.as_str())
                        .or_else(|| it.get("title").and_then(|v| v.as_str()))
                        .map(str::to_string),
                    description: it
                        .pointer("/metadata/description")
                        .and_then(|v| v.as_str())
                        .or_else(|| it.get("description").and_then(|v| v.as_str()))
                        .map(str::to_string),
                });
            }
        }

        match job_status {
            "completed" => return Ok(pages),
            "failed" | "cancelled" => {
                return Err(format!(
                    "Firecrawl crawl job {job_id} ended with status '{job_status}' after {} page(s)",
                    pages.len()
                ));
            }
            _ => {
                // Still scraping: keep polling. Surface progress at debug.
                log::debug!(
                    "Firecrawl crawl {job_id} status '{job_status}' ({} pages so far)",
                    pages.len()
                );
            }
        }
    }
}

/// Parse a document (PDF, Word, etc.) into structured Markdown using Firecrawl v2.
pub async fn firecrawl_parse_document(
    base_url: &str,
    api_key: &str,
    filename: &str,
    file_bytes: Vec<u8>,
) -> Result<String, String> {
    firecrawl_parse_document_with_timeout(
        base_url,
        api_key,
        filename,
        file_bytes,
        Duration::from_secs(120),
    )
    .await
}

pub async fn firecrawl_parse_document_with_timeout(
    base_url: &str,
    api_key: &str,
    filename: &str,
    file_bytes: Vec<u8>,
    timeout: Duration,
) -> Result<String, String> {
    check_api_key(api_key, "Firecrawl")?;

    let base = normalize_firecrawl_base_url(base_url);
    let url = format!("{}/parse", base);

    let client = http_client(timeout)?;

    let mime = if filename.ends_with(".pdf") {
        "application/pdf"
    } else if filename.ends_with(".docx") {
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
    } else if filename.ends_with(".png") {
        "image/png"
    } else if filename.ends_with(".jpg") || filename.ends_with(".jpeg") {
        "image/jpeg"
    } else {
        "application/octet-stream"
    };

    let part = reqwest::multipart::Part::bytes(file_bytes)
        .file_name(filename.to_string())
        .mime_str(mime)
        .map_err(|e| format!("Failed to build multipart payload: {e}"))?;

    let form = reqwest::multipart::Form::new().part("file", part);

    // Multipart sets its own Content-Type (boundary); only auth + UA here.
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", api_key.trim()))
            .map_err(|e| format!("Invalid Firecrawl API key header: {e}"))?,
    );
    headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE));

    let response = client
        .post(&url)
        .headers(headers)
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("Firecrawl parse document request failed: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        let err_text = response.text().await.unwrap_or_default();
        return Err(format!(
            "Firecrawl document parse error ({status}): {err_text}"
        ));
    }

    let parsed_json: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Firecrawl response JSON: {e}"))?;

    // Extract markdown or parsed text
    let markdown = parsed_json
        .pointer("/data/markdown")
        .and_then(|v| v.as_str())
        .or_else(|| parsed_json.pointer("/markdown").and_then(|v| v.as_str()))
        .or_else(|| parsed_json.pointer("/data/text").and_then(|v| v.as_str()))
        .unwrap_or_default();

    if markdown.trim().is_empty() {
        Ok(parsed_json.to_string())
    } else {
        Ok(markdown.to_string())
    }
}

/// Test connection and measure latency to the Firecrawl API using the official
/// probe endpoint (<https://docs.firecrawl.dev/introduction>).
pub async fn firecrawl_test_connection(
    base_url: &str,
    api_key: &str,
) -> Result<(String, u128), String> {
    firecrawl_test_connection_with_timeout(base_url, api_key, Duration::from_secs(30)).await
}

pub async fn firecrawl_test_connection_with_timeout(
    base_url: &str,
    api_key: &str,
    timeout: Duration,
) -> Result<(String, u128), String> {
    check_api_key(api_key, "Firecrawl")?;

    let base = normalize_firecrawl_base_url(base_url);
    let url = format!("{}/scrape", base);

    let start = Instant::now();

    let client = http_client(timeout)?;

    let payload = serde_json::json!({
        "url": "https://firecrawl.dev",
        "formats": ["markdown"]
    });

    let response = client
        .post(&url)
        .headers(firecrawl_headers(api_key)?)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Firecrawl connection test failed: {e}"))?;

    let elapsed = start.elapsed().as_millis();
    let status = response.status();
    if !status.is_success() {
        let err_text = response.text().await.unwrap_or_default();
        return Err(format!("Firecrawl API error ({status}): {err_text}"));
    }

    let parsed: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Firecrawl response JSON: {e}"))?;

    firecrawl_check_success(&parsed, "Firecrawl returned failure status")?;

    Ok((
        "Connected to Firecrawl API (v2 Scrape)".to_string(),
        elapsed,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tavily_search_options_serialization() {
        let opts = TavilySearchOptions {
            query: "rust gtk4".to_string(),
            search_depth: Some("advanced".to_string()),
            topic: Some("general".to_string()),
            time_range: Some("week".to_string()),
            max_results: Some(5),
            include_domains: Some(vec!["example.com".to_string()]),
            include_answer: Some(true),
            ..Default::default()
        };
        let json = serde_json::to_value(&opts).unwrap();
        assert_eq!(json["query"], "rust gtk4");
        assert_eq!(json["search_depth"], "advanced");
        assert_eq!(json["time_range"], "week");
        assert_eq!(json["include_domains"][0], "example.com");
        // Nones are omitted so server defaults apply.
        assert!(json.get("country").is_none());
        assert!(json.get("days").is_none());
    }

    #[test]
    fn test_tavily_extract_options_serialization() {
        let opts = TavilyExtractOptions {
            urls: vec!["https://example.com".to_string()],
            query: Some("summary".to_string()),
            extract_depth: Some("advanced".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_value(&opts).unwrap();
        assert_eq!(json["urls"][0], "https://example.com");
        assert_eq!(json["extract_depth"], "advanced");
        assert!(json.get("include_images").is_none());
    }

    #[test]
    fn test_tavily_response_parses_new_fields() {
        let raw = serde_json::json!({
            "query": "q",
            "answer": "a",
            "results": [{
                "title": "t", "url": "u", "content": "c", "score": 0.9,
                "published_date": "2026-01-01", "favicon": "https://x/favicon.ico"
            }],
            "images": []
        });
        let res: TavilySearchResponse = serde_json::from_value(raw).unwrap();
        assert_eq!(res.results[0].published_date.as_deref(), Some("2026-01-01"));
        // Missing new fields default instead of failing.
        let bare = serde_json::json!({
            "results": [{ "title": "t", "url": "u", "content": "c" }]
        });
        let res2: TavilySearchResponse = serde_json::from_value(bare).unwrap();
        assert!(res2.results[0].published_date.is_none());
    }

    #[test]
    fn test_firecrawl_search_request_serialization() {
        let req = FirecrawlSearchRequest {
            query: "linux desktop".to_string(),
            limit: Some(3),
            scrape_options: Some(FirecrawlScrapeOptions {
                formats: Some(vec!["markdown".to_string()]),
                ..Default::default()
            }),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("linux desktop"));
        assert!(json.contains("scrapeOptions"));
        assert!(json.contains("markdown"));
    }

    #[test]
    fn test_firecrawl_search_options_wire_shape() {
        // v2 uses lowercase `scrapeOptions` with optional sources/categories.
        let opts = FirecrawlSearchOptions {
            query: "q".to_string(),
            sources: Some(vec!["web".to_string()]),
            limit: Some(5),
            scrape_options: Some(FirecrawlScrapeOptions {
                formats: Some(vec!["markdown".to_string()]),
                only_main_content: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        };
        let json = serde_json::to_value(&opts).unwrap();
        assert_eq!(json["sources"][0], "web");
        assert_eq!(json["scrapeOptions"]["onlyMainContent"], true);
        assert!(json.get("tbs").is_none());
    }

    #[test]
    fn test_firecrawl_search_parses_v2_object_and_v1_array() {
        let v2 = serde_json::json!({
            "success": true,
            "data": { "web": [{ "url": "https://a", "title": "A" }] }
        });
        assert_eq!(firecrawl_items_from_search_value(&v2).len(), 1);
        let v1 = serde_json::json!({
            "success": true,
            "data": [{ "url": "https://b", "title": "B" }]
        });
        assert_eq!(firecrawl_items_from_search_value(&v1).len(), 1);
    }

    #[test]
    fn test_firecrawl_check_success_rejects_envelope() {
        let bad = serde_json::json!({ "success": false, "error": "bad key" });
        assert!(firecrawl_check_success(&bad, "fallback").is_err());
        let ok = serde_json::json!({ "success": true });
        assert!(firecrawl_check_success(&ok, "fallback").is_ok());
    }

    #[test]
    fn test_normalize_firecrawl_base_url() {
        assert_eq!(
            normalize_firecrawl_base_url(""),
            "https://api.firecrawl.dev/v2"
        );
        assert_eq!(
            normalize_firecrawl_base_url("https://api.firecrawl.dev"),
            "https://api.firecrawl.dev/v2"
        );
        assert_eq!(
            normalize_firecrawl_base_url("https://api.firecrawl.dev/"),
            "https://api.firecrawl.dev/v2"
        );
        assert_eq!(
            normalize_firecrawl_base_url("https://api.firecrawl.dev/v1"),
            "https://api.firecrawl.dev/v2"
        );
        assert_eq!(
            normalize_firecrawl_base_url("https://api.firecrawl.dev/v2"),
            "https://api.firecrawl.dev/v2"
        );
        // Self-hosted instances keep their origin (trailing slash trimmed).
        assert_eq!(
            normalize_firecrawl_base_url("http://localhost:3002/"),
            "http://localhost:3002"
        );
        assert_eq!(
            normalize_firecrawl_base_url("http://localhost:3002"),
            "http://localhost:3002"
        );
    }

    #[test]
    fn test_clamp_timeout_secs() {
        assert_eq!(clamp_timeout_secs(0), Duration::from_secs(5));
        assert_eq!(clamp_timeout_secs(60), Duration::from_secs(60));
        assert_eq!(clamp_timeout_secs(9999), Duration::from_secs(300));
    }
}
