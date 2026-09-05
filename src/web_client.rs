//! Web & Document Intelligence Client for Tavily and Firecrawl APIs.
//!
//! Provides native integration with:
//! - Tavily: Real-time search, deep research, web page content extraction
//! - Firecrawl: LLM-ready web search, markdown scraping, document parsing (PDF, DOCX)

use log::info;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{Duration, Instant};

// ============================================================================
// Tavily Models & Functions
// ============================================================================

#[derive(Debug, Serialize, Clone)]
pub struct TavilySearchRequest {
    pub api_key: String,
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_depth: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_answer: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TavilySearchResultItem {
    pub title: String,
    pub url: String,
    pub content: String,
    #[serde(default)]
    pub score: f64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TavilySearchResponse {
    pub query: Option<String>,
    pub answer: Option<String>,
    #[serde(default)]
    pub results: Vec<TavilySearchResultItem>,
}

#[derive(Debug, Serialize, Clone)]
pub struct TavilyExtractRequest {
    pub api_key: String,
    pub urls: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TavilyExtractItem {
    pub url: String,
    pub raw_content: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TavilyExtractResponse {
    #[serde(default)]
    pub results: Vec<TavilyExtractItem>,
    #[serde(default)]
    pub failed_results: Vec<Value>,
}

/// Execute a real-time web search via Tavily.
pub async fn tavily_search(
    base_url: &str,
    api_key: &str,
    query: &str,
    search_depth: &str,
    max_results: u32,
) -> Result<TavilySearchResponse, String> {
    if api_key.trim().is_empty() {
        return Err(
            "Tavily API key is missing. Please configure it in Settings -> Providers.".to_string(),
        );
    }

    let base = if base_url.trim().is_empty() {
        "https://api.tavily.com"
    } else {
        base_url.trim_end_matches('/')
    };
    let url = format!("{}/search", base);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let payload = TavilySearchRequest {
        api_key: api_key.to_string(),
        query: query.to_string(),
        search_depth: Some(search_depth.to_string()),
        include_answer: Some(true),
        max_results: Some(max_results.clamp(1, 20)),
    };

    info!(
        "Executing Tavily search for query: '{}' (depth: {})",
        query, search_depth
    );

    let response = client
        .post(&url)
        .header(CONTENT_TYPE, "application/json")
        .header(USER_AGENT, "Otush/1.0 (+https://github.com/ATClus/otush)")
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

/// Perform deep research investigation via Tavily advanced search.
pub async fn tavily_deep_research(
    base_url: &str,
    api_key: &str,
    query: &str,
) -> Result<TavilySearchResponse, String> {
    // Advanced search depth queries multiple sources and produces a high-level answer
    tavily_search(base_url, api_key, query, "advanced", 10).await
}

/// Extract clean content from a list of web URLs via Tavily.
pub async fn tavily_extract(
    base_url: &str,
    api_key: &str,
    urls: Vec<String>,
) -> Result<Vec<TavilyExtractItem>, String> {
    if api_key.trim().is_empty() {
        return Err(
            "Tavily API key is missing. Please configure it in Settings -> Providers.".to_string(),
        );
    }

    let base = if base_url.trim().is_empty() {
        "https://api.tavily.com"
    } else {
        base_url.trim_end_matches('/')
    };
    let url = format!("{}/extract", base);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let payload = TavilyExtractRequest {
        api_key: api_key.to_string(),
        urls,
    };

    let response = client
        .post(&url)
        .header(CONTENT_TYPE, "application/json")
        .header(USER_AGENT, "Otush/1.0 (+https://github.com/ATClus/otush)")
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

    Ok(extract_res.results)
}

/// Test connection and measure latency to Tavily API.
pub async fn tavily_test_connection(
    base_url: &str,
    api_key: &str,
) -> Result<(String, u128), String> {
    let start = Instant::now();
    let res = tavily_search(base_url, api_key, "Otush test ping", "basic", 1).await?;
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

#[derive(Debug, Serialize, Clone)]
pub struct FirecrawlSearchRequest {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(rename = "scrapeOptions", skip_serializing_if = "Option::is_none")]
    pub scrape_options: Option<FirecrawlScrapeOptions>,
}

#[derive(Debug, Serialize, Clone)]
pub struct FirecrawlScrapeOptions {
    pub formats: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
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

/// Perform web search and markdown retrieval via Firecrawl API (supports both v2 and v1).
pub async fn firecrawl_search(
    base_url: &str,
    api_key: &str,
    query: &str,
    limit: u32,
) -> Result<Vec<FirecrawlSearchResultItem>, String> {
    if api_key.trim().is_empty() {
        return Err(
            "Firecrawl API key is missing. Please configure it in Settings -> Providers."
                .to_string(),
        );
    }

    let base = normalize_firecrawl_base_url(base_url);
    let url = format!("{}/search", base);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let payload = serde_json::json!({
        "query": query,
        "limit": limit.clamp(1, 20),
        "scrapeOptions": {
            "formats": ["markdown"]
        }
    });

    info!("Executing Firecrawl search for: '{}'", query);

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", api_key.trim()))
            .map_err(|e| format!("Invalid Firecrawl API key header: {e}"))?,
    );
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("Otush/1.0 (+https://github.com/ATClus/otush)"),
    );

    let response = client
        .post(&url)
        .headers(headers)
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

    let success = val.get("success").and_then(|v| v.as_bool()).unwrap_or(true);
    if !success {
        let err = val
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("Firecrawl search indicated failure");
        return Err(err.to_string());
    }

    let mut results = Vec::new();
    // 1. Firecrawl v2 format: data.web is an array
    if let Some(arr) = val.pointer("/data/web").and_then(|v| v.as_array()) {
        for it in arr {
            if let Ok(item) = serde_json::from_value::<FirecrawlSearchResultItem>(it.clone()) {
                results.push(item);
            }
        }
    }
    // 2. Firecrawl v1 format: data is an array
    else if let Some(arr) = val.get("data").and_then(|v| v.as_array()) {
        for it in arr {
            if let Ok(item) = serde_json::from_value::<FirecrawlSearchResultItem>(it.clone()) {
                results.push(item);
            }
        }
    }

    Ok(results)
}

/// Scrape a specific URL and return LLM-ready clean Markdown via Firecrawl.
pub async fn firecrawl_scrape(
    base_url: &str,
    api_key: &str,
    target_url: &str,
) -> Result<String, String> {
    if api_key.trim().is_empty() {
        return Err(
            "Firecrawl API key is missing. Please configure it in Settings -> Providers."
                .to_string(),
        );
    }

    let base = normalize_firecrawl_base_url(base_url);
    let url = format!("{}/scrape", base);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let payload = serde_json::json!({
        "url": target_url,
        "formats": ["markdown"]
    });

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", api_key.trim()))
            .map_err(|e| format!("Invalid Firecrawl API key header: {e}"))?,
    );
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("Otush/1.0 (+https://github.com/ATClus/otush)"),
    );

    let response = client
        .post(&url)
        .headers(headers)
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

    let success = parsed_json
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    if !success {
        let err = parsed_json
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("Firecrawl scrape indicated failure");
        return Err(err.to_string());
    }

    let md = parsed_json
        .pointer("/data/markdown")
        .and_then(|v| v.as_str())
        .or_else(|| parsed_json.pointer("/markdown").and_then(|v| v.as_str()))
        .unwrap_or_default();

    Ok(md.to_string())
}

/// Parse a document (PDF, Word, etc.) into structured Markdown using Firecrawl v2.
pub async fn firecrawl_parse_document(
    base_url: &str,
    api_key: &str,
    filename: &str,
    file_bytes: Vec<u8>,
) -> Result<String, String> {
    if api_key.trim().is_empty() {
        return Err(
            "Firecrawl API key is missing. Please configure it in Settings -> Providers."
                .to_string(),
        );
    }

    let base = normalize_firecrawl_base_url(base_url);
    let url = format!("{}/parse", base);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

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

    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", api_key.trim()))
            .map_err(|e| format!("Invalid Firecrawl API key header: {e}"))?,
    );
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("Otush/1.0 (+https://github.com/ATClus/otush)"),
    );

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
    if api_key.trim().is_empty() {
        return Err(
            "Firecrawl API key is missing. Please configure it in Settings -> Providers."
                .to_string(),
        );
    }

    let base = normalize_firecrawl_base_url(base_url);
    let url = format!("{}/scrape", base);

    let start = Instant::now();

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let payload = serde_json::json!({
        "url": "https://firecrawl.dev",
        "formats": ["markdown"]
    });

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", api_key.trim()))
            .map_err(|e| format!("Invalid Firecrawl API key header: {e}"))?,
    );
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("Otush/1.0 (+https://github.com/ATClus/otush)"),
    );

    let response = client
        .post(&url)
        .headers(headers)
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

    let is_success = parsed
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    if !is_success {
        let err = parsed
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("Firecrawl returned failure status");
        return Err(err.to_string());
    }

    Ok((
        "Connected to Firecrawl API (v2 Scrape)".to_string(),
        elapsed,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tavily_search_request_serialization() {
        let req = TavilySearchRequest {
            api_key: "tvly-test".to_string(),
            query: "rust gtk4".to_string(),
            search_depth: Some("advanced".to_string()),
            include_answer: Some(true),
            max_results: Some(5),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("tvly-test"));
        assert!(json.contains("rust gtk4"));
        assert!(json.contains("advanced"));
    }

    #[test]
    fn test_firecrawl_search_request_serialization() {
        let req = FirecrawlSearchRequest {
            query: "linux desktop".to_string(),
            limit: Some(3),
            scrape_options: Some(FirecrawlScrapeOptions {
                formats: vec!["markdown".to_string()],
            }),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("linux desktop"));
        assert!(json.contains("scrapeOptions"));
        assert!(json.contains("markdown"));
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
        assert_eq!(
            normalize_firecrawl_base_url("http://localhost:3002"),
            "http://localhost:3002"
        );
    }
}
