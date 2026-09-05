//! Multi-provider LLM chat-completion client (post-processing, notes, research).
//!
//! Speaks the OpenAI-compatible `/chat/completions` dialect to every
//! post-processing provider (local Ollama plus cloud Anthropic, OpenAI, Groq,
//! Mistral, Cerebras), with per-provider headers, reasoning-effort mapping,
//! JSON-schema structured output, and vision/OCR. All requests run on the
//! shared Tokio runtime via [`crate::runtime`]; API keys are read from
//! settings at call time and never logged (see [`crate::utils::redact_text`]).

use crate::settings::{PostProcessProvider, ReasoningEffort};
use log::{debug, info};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, REFERER, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::error::Error as StdError;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatMessage {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ToolCallOut>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

impl ChatMessage {
    /// Plain user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// Plain system message.
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// Assistant turn that requested tool calls.
    pub fn assistant_with_tools(content: String, tool_calls: Vec<ToolCallOut>) -> Self {
        Self {
            role: "assistant".to_string(),
            content,
            tool_calls: Some(tool_calls),
            tool_call_id: None,
        }
    }

    /// Result of one tool execution, fed back as `role: "tool"`.
    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".to_string(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

/// One tool call requested by the assistant (OpenAI-compat `tool_calls` item).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ToolCallOut {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: ToolFunctionCall,
}

/// The `function` payload of a tool call: name + JSON-encoded arguments.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ToolFunctionCall {
    pub name: String,
    pub arguments: String,
}

/// One function tool exposed to the model (OpenAI-compat `tools` item).
#[derive(Debug, Serialize, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    /// JSON Schema object for the arguments.
    pub parameters: Value,
}

impl ToolDefinition {
    pub fn new(name: impl Into<String>, description: impl Into<String>, parameters: Value) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
        }
    }
}

/// Assistant reply: final text, tool call requests, or both.
#[derive(Debug, Clone, Default)]
pub struct AssistantReply {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCallOut>,
}

#[derive(Debug, Serialize)]
struct JsonSchema {
    name: String,
    strict: bool,
    schema: Value,
}

#[derive(Debug, Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    format_type: String,
    json_schema: JsonSchema,
}

#[derive(Debug, Serialize, Clone, Default, PartialEq)]
struct ReasoningConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exclude: Option<bool>,
}

/// Request fields used for reasoning / thinking configuration across providers.
#[derive(Debug, Serialize, Clone, Default, PartialEq)]
struct ReasoningParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<ReasoningConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<Value>,
}

impl ReasoningParams {
    fn is_empty(&self) -> bool {
        self.reasoning_effort.is_none() && self.reasoning.is_none() && self.thinking.is_none()
    }
}

/// Pick the reasoning request fields according to provider and user configuration.
fn build_reasoning_params(
    provider: &PostProcessProvider,
    disable_reasoning: bool,
) -> ReasoningParams {
    let base_url = provider.base_url.to_lowercase();
    let effort = provider.reasoning.effort;

    if disable_reasoning || effort == ReasoningEffort::None {
        if base_url.contains("api.deepseek.com") || provider.id == "deepseek" {
            ReasoningParams {
                thinking: Some(serde_json::json!({ "type": "disabled" })),
                ..Default::default()
            }
        } else if provider.id == "openrouter" {
            ReasoningParams {
                reasoning: Some(ReasoningConfig {
                    effort: Some("none".to_string()),
                    exclude: Some(true),
                }),
                ..Default::default()
            }
        } else if provider.id == "meta" || base_url.contains("api.meta.ai") {
            // Meta Muse Spark does NOT support "none" — omit reasoning_effort
            ReasoningParams::default()
        } else if provider.id == "anthropic" || base_url.contains("anthropic.com") {
            // Anthropic extended thinking is disabled simply by omitting thinking parameter
            ReasoningParams::default()
        } else if provider.id == "gemini" || base_url.contains("generativelanguage.googleapis.com")
        {
            // Google Gemini openai-compatible endpoint does not accept reasoning_effort: "none"
            ReasoningParams::default()
        } else {
            ReasoningParams {
                reasoning_effort: Some("none".to_string()),
                ..Default::default()
            }
        }
    } else {
        let effort_str = match effort {
            ReasoningEffort::Minimal => "minimal",
            ReasoningEffort::Low => "low",
            ReasoningEffort::Medium => "medium",
            ReasoningEffort::High => "high",
            ReasoningEffort::XHigh => "xhigh",
            ReasoningEffort::None => "none",
        };

        if provider.id == "anthropic" || base_url.contains("anthropic.com") {
            let budget = provider.reasoning.budget_tokens.unwrap_or(match effort {
                ReasoningEffort::Minimal | ReasoningEffort::Low => 1024,
                ReasoningEffort::Medium => 2048,
                ReasoningEffort::High => 4096,
                ReasoningEffort::XHigh => 8192,
                ReasoningEffort::None => 0,
            });
            ReasoningParams {
                thinking: Some(serde_json::json!({
                    "type": "enabled",
                    "budget_tokens": budget
                })),
                ..Default::default()
            }
        } else if provider.id == "openrouter" {
            ReasoningParams {
                reasoning: Some(ReasoningConfig {
                    effort: Some(effort_str.to_string()),
                    exclude: Some(false),
                }),
                ..Default::default()
            }
        } else if provider.id == "deepseek" || base_url.contains("api.deepseek.com") {
            ReasoningParams {
                thinking: Some(serde_json::json!({ "type": "enabled" })),
                ..Default::default()
            }
        } else if provider.id == "meta" || base_url.contains("api.meta.ai") {
            // Official Meta Model API (Muse Spark) supports "minimal", "low", "medium", "high", "xhigh"
            ReasoningParams {
                reasoning_effort: Some(effort_str.to_string()),
                ..Default::default()
            }
        } else {
            // OpenAI o1/o3/o4 and standard OpenAI-compatible endpoints support "low", "medium", "high"
            let openai_effort = match effort {
                ReasoningEffort::Minimal | ReasoningEffort::Low => "low",
                ReasoningEffort::Medium => "medium",
                ReasoningEffort::High | ReasoningEffort::XHigh => "high",
                ReasoningEffort::None => "none",
            };
            ReasoningParams {
                reasoning_effort: Some(openai_effort.to_string()),
                ..Default::default()
            }
        }
    }
}

/// Endpoints (base_url|model) that rejected the reasoning-disable fields with a
/// 4xx. Remembered for the lifetime of the process.
fn reasoning_rejections() -> &'static Mutex<HashSet<String>> {
    static REJECTED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    REJECTED.get_or_init(|| Mutex::new(HashSet::new()))
}

fn endpoint_key(provider: &PostProcessProvider, model: &str) -> String {
    format!("{}|{}", provider.base_url.trim_end_matches('/'), model)
}

fn is_known_rejected(key: &str) -> bool {
    reasoning_rejections()
        .lock()
        .map(|set| set.contains(key))
        .unwrap_or(false)
}

fn remember_rejection(key: String) {
    if let Ok(mut set) = reasoning_rejections().lock() {
        set.insert(key);
    }
}

#[derive(Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ToolRequestItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
    #[serde(flatten)]
    reasoning: ReasoningParams,
}

/// OpenAI-compat `{"type": "function", "function": {...}}` wrapper.
#[derive(Debug, Serialize)]
struct ToolRequestItem {
    #[serde(rename = "type")]
    item_type: String,
    function: ToolRequestFunction,
}

/// OpenAI-compat function descriptor inside a `tools` item.
#[derive(Debug, Serialize)]
struct ToolRequestFunction {
    name: String,
    description: String,
    parameters: Value,
}

#[derive(Debug, Deserialize)]
struct CompletionTokensDetails {
    #[serde(default)]
    reasoning_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct UsageResponse {
    #[serde(default)]
    completion_tokens_details: Option<CompletionTokensDetails>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
    #[serde(default)]
    usage: Option<UsageResponse>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessageResponse,
}

#[derive(Debug, Deserialize)]
struct ChatMessageResponse {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ToolCallOut>>,
}

/// Build headers for API requests based on provider type and custom headers
fn build_headers(provider: &PostProcessProvider, api_key: &str) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();

    // Common headers
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        REFERER,
        HeaderValue::from_static("https://github.com/ATClus/otush"),
    );
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("Otush/1.0 (+https://github.com/ATClus/otush)"),
    );
    headers.insert("X-Title", HeaderValue::from_static("Otush"));

    // User-configured custom headers
    for (key, val) in &provider.custom_headers {
        if let (Ok(header_name), Ok(header_val)) = (
            reqwest::header::HeaderName::from_bytes(key.as_bytes()),
            HeaderValue::from_str(val),
        ) {
            headers.insert(header_name, header_val);
        }
    }

    // Provider-specific auth headers
    if !api_key.is_empty() {
        if provider.id == "anthropic" || provider.base_url.contains("anthropic.com") {
            headers.insert(
                "x-api-key",
                HeaderValue::from_str(api_key)
                    .map_err(|e| format!("Invalid API key header value: {}", e))?,
            );
            headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
        } else if provider.id == "gemini" || provider.base_url.contains("googleapis.com") {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {}", api_key))
                    .map_err(|e| format!("Invalid authorization header value: {}", e))?,
            );
            headers.insert(
                "x-goog-api-key",
                HeaderValue::from_str(api_key)
                    .map_err(|e| format!("Invalid API key header value: {}", e))?,
            );
        } else {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {}", api_key))
                    .map_err(|e| format!("Invalid authorization header value: {}", e))?,
            );
        }
    }

    Ok(headers)
}

/// Create an HTTP client with provider-specific headers and timeout
fn create_client(provider: &PostProcessProvider, api_key: &str) -> Result<reqwest::Client, String> {
    let headers = build_headers(provider, api_key)?;
    // Enforce a sane timeout for LLM generation: minimum 60s, default 120s if zero/unconfigured
    let timeout_secs = if provider.timeout_seconds >= 30 {
        provider.timeout_seconds
    } else if provider.timeout_seconds > 0 {
        provider.timeout_seconds.max(60)
    } else {
        120
    };
    reqwest::Client::builder()
        .default_headers(headers)
        .connect_timeout(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(timeout_secs as u64))
        .build()
        .map_err(|e| report_reqwest_error("Failed to build HTTP client", &e))
}

/// Retrieve or create a cached HTTP client to reuse TCP/TLS connection pools and keep-alive.
fn cached_client(provider: &PostProcessProvider, api_key: &str) -> Result<reqwest::Client, String> {
    static CLIENT_CACHE: OnceLock<Mutex<HashMap<String, reqwest::Client>>> = OnceLock::new();
    let cache = CLIENT_CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    let timeout_secs = if provider.timeout_seconds >= 30 {
        provider.timeout_seconds
    } else if provider.timeout_seconds > 0 {
        provider.timeout_seconds.max(60)
    } else {
        120
    };

    let cache_key = format!(
        "{}:{}:{}:{}",
        provider.id, provider.base_url, timeout_secs, api_key
    );

    if let Ok(guard) = cache.lock() {
        if let Some(client) = guard.get(&cache_key) {
            return Ok(client.clone());
        }
    }

    let new_client = create_client(provider, api_key)?;
    if let Ok(mut guard) = cache.lock() {
        guard.insert(cache_key, new_client.clone());
    }
    Ok(new_client)
}

/// Format a bounded error source chain.
///
/// `reqwest::Error`'s Display implementation intentionally gives only a short
/// summary. Nested causes contain the useful transport details, such as a
/// certificate validation failure, an HTTP/2 error, or a connection reset.
/// Callers must skip source types whose Display text can quote payload data.
fn error_source_chain(error: &(dyn StdError + 'static)) -> Vec<String> {
    let mut causes = Vec::new();
    let mut source = error.source();

    // Defensive cap in case a third-party error exposes a cyclic source chain.
    for _ in 0..16 {
        let Some(cause) = source else {
            break;
        };
        causes.push(cause.to_string());
        source = cause.source();
    }

    causes
}

fn reqwest_error_kinds(error: &reqwest::Error) -> String {
    let mut kinds = Vec::new();

    if error.is_builder() {
        kinds.push("builder");
    }
    if error.is_connect() {
        kinds.push("connect");
    }
    if error.is_request() {
        kinds.push("request");
    }
    if error.is_redirect() {
        kinds.push("redirect");
    }
    if error.is_timeout() {
        kinds.push("timeout");
    }
    if error.is_status() {
        kinds.push("status");
    }
    if error.is_body() {
        kinds.push("body");
    }
    if error.is_decode() {
        kinds.push("decode");
    }
    if error.is_upgrade() {
        kinds.push("upgrade");
    }

    if kinds.is_empty() {
        "unknown".to_string()
    } else {
        kinds.join(", ")
    }
}

fn sanitized_url(url: &reqwest::Url) -> String {
    let mut url = url.clone();

    // Custom endpoints should not contain credentials or query-string tokens,
    // but omit them from diagnostics in case one does.
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);

    url.to_string()
}

fn sanitized_url_for_log(url: &str) -> String {
    reqwest::Url::parse(url)
        .map(|url| sanitized_url(&url))
        // Do not echo an invalid URL: the parse failure might have been caused
        // by sensitive data entered in the custom endpoint field.
        .unwrap_or_else(|_| "<invalid URL>".to_string())
}

fn report_reqwest_error(context: &str, error: &reqwest::Error) -> String {
    let kinds = reqwest_error_kinds(error);
    let url = error
        .url()
        .map(sanitized_url)
        .map(|url| format!(", url: {url}"))
        .unwrap_or_default();

    // serde_json's error text can quote values from a malformed response. That
    // response may contain transcription content, so retain the useful decode
    // classification but never put its nested source in logs or UI errors.
    let causes = if error.is_decode() {
        Vec::new()
    } else {
        error_source_chain(error)
    };
    let cause_details = if error.is_timeout() {
        ": request timed out waiting for response or receiving body (consider increasing provider timeout for large transcripts or reasoning models)".to_string()
    } else if !causes.is_empty() {
        format!(": caused by: {}", causes.join(" -> "))
    } else if error.url().is_none() {
        // Reqwest's short Display text is safe when it cannot append a raw URL.
        format!(": {error}")
    } else {
        // The sanitized URL is already included above. Avoid formatting the
        // original error because its Display implementation includes the raw URL.
        String::new()
    };

    let details = format!("{context} (kind: {kinds}{url}){cause_details}");
    debug!("{details}");
    details
}

/// Send a chat completion request to an OpenAI-compatible API
/// Returns Ok(Some(content)) on success, Ok(None) if response has no content,
/// or Err on actual errors (HTTP, parsing, etc.)
pub async fn send_chat_completion(
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    prompt: String,
    disable_reasoning: bool,
) -> Result<Option<String>, String> {
    send_chat_completion_with_schema(
        provider,
        api_key,
        model,
        prompt,
        None,
        None,
        disable_reasoning,
    )
    .await
}

/// Send a chat completion request with structured output support.
/// When json_schema is provided, uses structured outputs mode.
/// system_prompt is used as the system message when provided.
///
/// When disable_reasoning is set, the request carries the reasoning-disable
/// fields the endpoint is expected to understand. Not every OpenAI-compatible
/// endpoint accepts them (DeepSeek, Gemini's compat layer, and some OpenRouter
/// upstreams reject with 400), so a 400/422 answer to such a request triggers
/// one retry without the fields, and the rejection is remembered per
/// (base_url, model) so later requests skip the failing attempt entirely.
pub async fn send_chat_completion_with_schema(
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    user_content: String,
    system_prompt: Option<String>,
    json_schema: Option<Value>,
    disable_reasoning: bool,
) -> Result<Option<String>, String> {
    let base_url = provider.base_url.trim_end_matches('/');
    let url = format!("{}/chat/completions", base_url);

    debug!(
        "Sending chat completion request to: {}",
        sanitized_url_for_log(&url)
    );

    let client = cached_client(provider, &api_key)?;

    // Build messages vector
    let mut messages = Vec::new();

    // Add system prompt if provided
    if let Some(system) = system_prompt {
        messages.push(ChatMessage::system(system));
    }

    // Add user message
    messages.push(ChatMessage::user(user_content));

    // Build response_format if schema is provided
    let response_format = json_schema.map(|schema| ResponseFormat {
        format_type: "json_schema".to_string(),
        json_schema: JsonSchema {
            name: "transcription_output".to_string(),
            strict: true,
            schema,
        },
    });

    let key = endpoint_key(provider, model);
    let reasoning = if !is_known_rejected(&key) {
        build_reasoning_params(provider, disable_reasoning)
    } else {
        ReasoningParams::default()
    };

    let mut request_body = ChatCompletionRequest {
        model: model.to_string(),
        messages,
        stream: false,
        response_format,
        tools: None,
        tool_choice: None,
        reasoning,
    };

    info!(
        "Sending LLM chat completion to {} (model: '{}', structured: {}):\n--- System Prompt ---\n{:?}\n--- User Content ---\n{:?}",
        sanitized_url_for_log(&url),
        model,
        request_body.response_format.is_some(),
        request_body.messages.iter().find(|m| m.role == "system").map(|m| &m.content),
        request_body.messages.iter().find(|m| m.role == "user").map(|m| &m.content)
    );

    let mut response = client
        .post(&url)
        .json(&request_body)
        .send()
        .await
        .map_err(|e| report_reqwest_error("HTTP request failed", &e))?;
    let mut status = response.status();
    debug!(
        "Chat completion response received with status {} over {:?} from {}",
        status,
        response.version(),
        sanitized_url(response.url())
    );

    // A 400/422 on a request carrying reasoning fields might be the endpoint rejecting those fields — retry once without them.
    if !status.is_success()
        && matches!(status.as_u16(), 400 | 422)
        && !request_body.reasoning.is_empty()
    {
        let error_text = response.text().await.unwrap_or_else(|e| {
            report_reqwest_error("Failed to read reasoning rejection response", &e)
        });
        info!(
            "Endpoint rejected request with reasoning fields (status {}): {}. Retrying without reasoning fields",
            status, error_text
        );

        request_body.reasoning = ReasoningParams::default();
        response = client
            .post(&url)
            .json(&request_body)
            .send()
            .await
            .map_err(|e| report_reqwest_error("HTTP retry failed", &e))?;
        status = response.status();
        debug!(
            "Chat completion retry response received with status {} over {:?} from {}",
            status,
            response.version(),
            sanitized_url(response.url())
        );

        if status.is_success() {
            info!(
                "Retry without reasoning fields succeeded; '{}' (model '{}') will skip them from now on",
                sanitized_url_for_log(base_url), model
            );
            remember_rejection(key);
        }
    }

    if !status.is_success() {
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|e| report_reqwest_error("Failed to read API error response", &e));
        info!(
            "LLM API request failed with status {}: {}",
            status, error_text
        );
        return Err(format!(
            "API request failed with status {}: {}",
            status, error_text
        ));
    }

    let raw_response = response
        .text()
        .await
        .map_err(|e| report_reqwest_error("Failed to read API response body", &e))?;

    info!(
        "LLM HTTP response received from {} (status {}):\n{}",
        sanitized_url_for_log(&url),
        status,
        raw_response
    );

    let completion: ChatCompletionResponse = serde_json::from_str(&raw_response)
        .map_err(|e| format!("Failed to parse API response JSON: {e}"))?;

    let mut content = completion
        .choices
        .first()
        .and_then(|choice| choice.message.content.clone());

    let reasoning_tokens = completion
        .usage
        .as_ref()
        .and_then(|u| u.completion_tokens_details.as_ref())
        .and_then(|d| d.reasoning_tokens);

    if let Some(tokens) = reasoning_tokens {
        info!("LLM completion reasoning tokens: {}", tokens);
        if let Some(text) = content {
            if text.contains("Connected successfully") && tokens > 0 {
                content = Some(format!("{} ({} reasoning tokens)", text, tokens));
            } else {
                content = Some(text);
            }
        }
    }

    info!("LLM parsed content from first choice: {:?}", content);

    Ok(content)
}

/// Send a multi-turn chat request with optional function tools (agentic loop).
///
/// `tools` maps to the OpenAI-compat `tools` array with `tool_choice: "auto"`.
/// The reasoning-disable/retry semantics match
/// [`send_chat_completion_with_schema`]: a 400/422 on a request carrying
/// reasoning fields retries once without them.
pub async fn send_chat_messages(
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    messages: Vec<ChatMessage>,
    tools: Option<Vec<ToolDefinition>>,
    disable_reasoning: bool,
) -> Result<AssistantReply, String> {
    let base_url = provider.base_url.trim_end_matches('/');
    let url = format!("{}/chat/completions", base_url);

    let client = cached_client(provider, &api_key)?;

    let tool_items: Option<Vec<ToolRequestItem>> = tools.map(|defs| {
        defs.into_iter()
            .map(|def| ToolRequestItem {
                item_type: "function".to_string(),
                function: ToolRequestFunction {
                    name: def.name,
                    description: def.description,
                    parameters: def.parameters,
                },
            })
            .collect()
    });
    let tool_choice = if tool_items.is_some() {
        Some("auto".to_string())
    } else {
        None
    };

    let key = endpoint_key(provider, model);
    let reasoning = if !is_known_rejected(&key) {
        build_reasoning_params(provider, disable_reasoning)
    } else {
        ReasoningParams::default()
    };

    let mut request_body = ChatCompletionRequest {
        model: model.to_string(),
        messages,
        stream: false,
        response_format: None,
        tools: tool_items,
        tool_choice,
        reasoning,
    };

    let mut response = client
        .post(&url)
        .json(&request_body)
        .send()
        .await
        .map_err(|e| report_reqwest_error("HTTP request failed", &e))?;
    let mut status = response.status();

    if !status.is_success()
        && matches!(status.as_u16(), 400 | 422)
        && !request_body.reasoning.is_empty()
    {
        let error_text = response.text().await.unwrap_or_else(|e| {
            report_reqwest_error("Failed to read reasoning rejection response", &e)
        });
        info!(
            "Endpoint rejected tool request with reasoning fields (status {}): {}. Retrying without reasoning fields",
            status, error_text
        );

        request_body.reasoning = ReasoningParams::default();
        response = client
            .post(&url)
            .json(&request_body)
            .send()
            .await
            .map_err(|e| report_reqwest_error("HTTP retry failed", &e))?;
        status = response.status();

        if status.is_success() {
            remember_rejection(key);
        }
    }

    if !status.is_success() {
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|e| report_reqwest_error("Failed to read API error response", &e));
        return Err(format!(
            "API request failed with status {}: {}",
            status, error_text
        ));
    }

    let raw_response = response
        .text()
        .await
        .map_err(|e| report_reqwest_error("Failed to read API response body", &e))?;

    let completion: ChatCompletionResponse = serde_json::from_str(&raw_response)
        .map_err(|e| format!("Failed to parse API response JSON: {e}"))?;

    let Some(first) = completion.choices.first() else {
        return Err("API response contained no choices".to_string());
    };
    Ok(AssistantReply {
        content: first.message.content.clone(),
        tool_calls: first.message.tool_calls.clone().unwrap_or_default(),
    })
}

/// Fetch available models dynamically from an LLM provider API.
/// Returns a list of clean model IDs suitable for chat completion.
pub async fn fetch_models(
    provider: &PostProcessProvider,
    api_key: String,
) -> Result<Vec<String>, String> {
    let base_url = provider.base_url.trim_end_matches('/');
    let endpoint = provider.models_endpoint.as_deref().unwrap_or("/models");
    let url = format!("{}{}", base_url, endpoint);

    debug!("Fetching models from: {}", sanitized_url_for_log(&url));

    let client = cached_client(provider, &api_key)?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| report_reqwest_error("Failed to fetch models", &e))?;

    let status = response.status();
    debug!(
        "Model list response received with status {} over {:?} from {}",
        status,
        response.version(),
        sanitized_url(response.url())
    );
    if !status.is_success() {
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|e| report_reqwest_error("Failed to read model list error", &e));
        return Err(format!(
            "Model list request failed ({}): {}",
            status, error_text
        ));
    }

    let parsed: serde_json::Value = response
        .json()
        .await
        .map_err(|e| report_reqwest_error("Failed to parse model list response", &e))?;

    let mut models = Vec::new();

    // 1. OpenAI/Anthropic/Groq/Mistral/OpenRouter/Z.AI/Cerebras format: { data: [ { id: "..." }, ... ] }
    if let Some(data) = parsed.get("data").and_then(|d| d.as_array()) {
        for entry in data {
            if let Some(id) = entry.get("id").and_then(|i| i.as_str()) {
                models.push(id.to_string());
            } else if let Some(name) = entry.get("name").and_then(|n| n.as_str()) {
                models.push(name.to_string());
            }
        }
    }
    // 2. Google Gemini / Ollama format: { models: [ { name: "models/gemini-2.0-flash" or "name": "..." }, ... ] }
    else if let Some(models_list) = parsed.get("models").and_then(|m| m.as_array()) {
        for entry in models_list {
            if let Some(name) = entry.get("name").and_then(|n| n.as_str()) {
                let clean_name = name.strip_prefix("models/").unwrap_or(name);
                models.push(clean_name.to_string());
            } else if let Some(model) = entry.get("model").and_then(|m| m.as_str()) {
                models.push(model.to_string());
            }
        }
    }
    // 2b. Ollama tags format: { tags: [ { name: "..." }, ... ] }
    else if let Some(tags_list) = parsed.get("tags").and_then(|t| t.as_array()) {
        for entry in tags_list {
            if let Some(name) = entry.get("name").and_then(|n| n.as_str()) {
                models.push(name.to_string());
            }
        }
    }
    // 3. Direct array format: [ "model1", "model2", ... ]
    else if let Some(array) = parsed.as_array() {
        for entry in array {
            if let Some(model) = entry.as_str() {
                models.push(model.to_string());
            }
        }
    }

    // Filter out non-chat models (embeddings, tts, image generation)
    models.retain(|m| {
        let lower = m.to_lowercase();
        !lower.starts_with("text-embedding")
            && !lower.starts_with("dall-e")
            && !lower.starts_with("tts-")
            && !lower.starts_with("whisper-")
            && !lower.contains("embed")
    });

    models.sort();
    models.dedup();

    Ok(models)
}

/// Test connection to an LLM provider and measure latency in milliseconds.
pub async fn test_provider_connection(
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
) -> Result<(String, u128), String> {
    let start = std::time::Instant::now();
    let test_prompt = "Hello! Reply with 'Connected successfully' and nothing else.".to_string();

    let result = send_chat_completion(provider, api_key, model, test_prompt, false).await?;

    let elapsed = start.elapsed().as_millis();
    match result {
        Some(text) => Ok((text.trim().to_string(), elapsed)),
        None => Err("Provider returned an empty response".to_string()),
    }
}

/// Send an image to a multimodal vision LLM (OpenAI GPT-4o, Google Gemini, Claude 3.7, Pixtral) for OCR and text extraction.
pub async fn send_vision_ocr(
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    image_bytes: &[u8],
    mime_type: &str,
    prompt: Option<&str>,
) -> Result<String, String> {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(image_bytes);
    let data_url = format!("data:{};base64,{}", mime_type, b64);
    let base_url = provider.base_url.trim_end_matches('/');
    let url = format!("{}/chat/completions", base_url);

    let prompt_text = prompt.unwrap_or(
        "Extract all readable text, tables, and structured data from this document/image. Preserve formatting as clean Markdown without conversational preamble.",
    );

    let client = cached_client(provider, &api_key)?;

    let request_body = serde_json::json!({
        "model": model,
        "messages": [
            {
                "role": "user",
                "content": [
                    {
                        "type": "text",
                        "text": prompt_text
                    },
                    {
                        "type": "image_url",
                        "image_url": {
                            "url": data_url
                        }
                    }
                ]
            }
        ],
        "stream": false
    });

    let response = client
        .post(&url)
        .json(&request_body)
        .send()
        .await
        .map_err(|e| report_reqwest_error("Vision OCR request failed", &e))?;

    let status = response.status();
    if !status.is_success() {
        let err_text = response.text().await.unwrap_or_default();
        return Err(format!("Vision OCR API error ({status}): {err_text}"));
    }

    let parsed: serde_json::Value = response
        .json()
        .await
        .map_err(|e| report_reqwest_error("Failed to parse Vision OCR response", &e))?;

    let content = parsed
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "No text returned from Vision OCR".to_string())?;

    Ok(content.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[derive(Debug)]
    struct TestError {
        message: &'static str,
        source: Option<Box<TestError>>,
    }

    impl fmt::Display for TestError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(self.message)
        }
    }

    impl StdError for TestError {
        fn source(&self) -> Option<&(dyn StdError + 'static)> {
            self.source
                .as_deref()
                .map(|source| source as &(dyn StdError + 'static))
        }
    }

    fn provider(id: &str, base_url: &str) -> PostProcessProvider {
        PostProcessProvider {
            id: id.to_string(),
            label: id.to_string(),
            base_url: base_url.to_string(),
            allow_base_url_edit: true,
            models_endpoint: None,
            supports_structured_output: false,
            reasoning: Default::default(),
            enabled: true,
            custom_headers: std::collections::HashMap::new(),
            timeout_seconds: 10,
        }
    }

    fn request_json(reasoning: ReasoningParams) -> Value {
        let request = ChatCompletionRequest {
            model: "test-model".to_string(),
            messages: vec![ChatMessage::user("hi")],
            stream: false,
            response_format: None,
            tools: None,
            tool_choice: None,
            reasoning,
        };
        serde_json::to_value(&request).unwrap()
    }

    async fn serve_one_response(status: &str, body: &str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 2048];
            let _ = stream.read(&mut request).await.unwrap();
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        format!("http://{address}")
    }

    #[test]
    fn error_source_chain_includes_all_nested_causes() {
        let error = TestError {
            message: "request failed",
            source: Some(Box::new(TestError {
                message: "TLS handshake failed",
                source: Some(Box::new(TestError {
                    message: "unknown certificate authority",
                    source: None,
                })),
            })),
        };

        assert_eq!(
            error_source_chain(&error),
            vec!["TLS handshake failed", "unknown certificate authority"]
        );
    }

    #[test]
    fn log_url_sanitization_removes_credentials_and_tokens() {
        let url = "https://user:password@example.com/v1/models?api_key=secret#private";
        assert_eq!(sanitized_url_for_log(url), "https://example.com/v1/models");
    }

    #[test]
    fn invalid_log_urls_are_not_echoed() {
        assert_eq!(
            sanitized_url_for_log("not a URL containing secret"),
            "<invalid URL>"
        );
    }

    #[tokio::test]
    async fn decode_error_does_not_echo_response_values() {
        let base_url =
            serve_one_response("200 OK", r#"{"choices":"PRIVATE TRANSCRIPTION CONTENT"}"#).await;
        let error = reqwest::get(base_url)
            .await
            .unwrap()
            .json::<ChatCompletionResponse>()
            .await
            .unwrap_err();

        let details = report_reqwest_error("Failed to parse API response", &error);
        assert!(details.contains("kind: decode"));
        assert!(!details.contains("PRIVATE TRANSCRIPTION CONTENT"));
    }

    #[tokio::test]
    async fn raw_error_url_is_not_reintroduced_without_a_source() {
        let base_url = serve_one_response("400 Bad Request", "bad request").await;
        let error = reqwest::get(format!(
            "{base_url}/private?api_key=SECRET_QUERY_TOKEN#private"
        ))
        .await
        .unwrap()
        .error_for_status()
        .unwrap_err();

        let details = report_reqwest_error("Request failed", &error);
        assert!(details.contains(&format!("url: {base_url}/private")));
        assert!(!details.contains("SECRET_QUERY_TOKEN"));
        assert!(!details.contains("#private"));
    }

    #[test]
    fn requests_explicitly_disable_streaming() {
        let json = request_json(ReasoningParams::default());
        assert_eq!(json["stream"], false);
    }

    #[test]
    fn default_reasoning_params_serialize_to_no_fields() {
        let json = request_json(ReasoningParams::default());
        assert!(json.get("reasoning_effort").is_none());
        assert!(json.get("reasoning").is_none());
        assert!(json.get("thinking").is_none());
    }

    #[test]
    fn custom_provider_uses_top_level_reasoning_effort() {
        let params = build_reasoning_params(&provider("custom", "http://localhost:11434/v1"), true);
        let json = request_json(params);
        assert_eq!(json["reasoning_effort"], "none");
        assert!(json.get("reasoning").is_none());
        assert!(json.get("thinking").is_none());
    }

    #[test]
    fn openrouter_uses_nested_reasoning_object() {
        let params = build_reasoning_params(
            &provider("openrouter", "https://openrouter.ai/api/v1"),
            true,
        );
        let json = request_json(params);
        assert!(json.get("reasoning_effort").is_none());
        assert_eq!(json["reasoning"]["effort"], "none");
        assert_eq!(json["reasoning"]["exclude"], true);
        assert!(json.get("thinking").is_none());
    }

    #[test]
    fn deepseek_base_url_uses_thinking_disabled() {
        let params = build_reasoning_params(&provider("custom", "https://api.deepseek.com"), true);
        let json = request_json(params);
        assert!(json.get("reasoning_effort").is_none());
        assert!(json.get("reasoning").is_none());
        assert_eq!(json["thinking"]["type"], "disabled");
    }

    #[test]
    fn anthropic_reasoning_generates_budget_tokens() {
        let mut prov = provider("anthropic", "https://api.anthropic.com/v1");
        prov.reasoning.effort = ReasoningEffort::High;
        prov.reasoning.budget_tokens = Some(4096);
        let params = build_reasoning_params(&prov, false);
        let json = request_json(params);
        assert_eq!(json["thinking"]["type"], "enabled");
        assert_eq!(json["thinking"]["budget_tokens"], 4096);
    }

    #[test]
    fn reasoning_params_is_empty_tracks_all_fields() {
        assert!(ReasoningParams::default().is_empty());
        assert!(!ReasoningParams {
            reasoning_effort: Some("none".to_string()),
            ..Default::default()
        }
        .is_empty());
        assert!(!ReasoningParams {
            thinking: Some(serde_json::json!({ "type": "disabled" })),
            ..Default::default()
        }
        .is_empty());
    }

    #[test]
    fn rejection_memo_is_keyed_by_base_url_and_model() {
        let deepseek = provider("custom", "https://api.deepseek.com/");
        let key = endpoint_key(&deepseek, "deepseek-chat");
        assert_eq!(key, "https://api.deepseek.com|deepseek-chat");
        assert!(!is_known_rejected(&key));
        remember_rejection(key.clone());
        assert!(is_known_rejected(&key));
        // A different model on the same endpoint is tracked separately
        assert!(!is_known_rejected(&endpoint_key(&deepseek, "other-model")));
    }

    #[test]
    fn meta_provider_omits_reasoning_when_disabled_or_none() {
        let mut prov = provider("meta", "https://api.meta.ai/v1");
        prov.reasoning.effort = ReasoningEffort::None;
        let params = build_reasoning_params(&prov, true);
        let json = request_json(params);
        assert!(json.get("reasoning_effort").is_none());
        assert!(json.get("reasoning").is_none());
        assert!(json.get("thinking").is_none());
    }

    #[test]
    fn meta_provider_supports_all_muse_spark_reasoning_levels() {
        let mut prov = provider("meta", "https://api.meta.ai/v1");

        prov.reasoning.effort = ReasoningEffort::Minimal;
        let json = request_json(build_reasoning_params(&prov, false));
        assert_eq!(json["reasoning_effort"], "minimal");

        prov.reasoning.effort = ReasoningEffort::Low;
        let json = request_json(build_reasoning_params(&prov, false));
        assert_eq!(json["reasoning_effort"], "low");

        prov.reasoning.effort = ReasoningEffort::Medium;
        let json = request_json(build_reasoning_params(&prov, false));
        assert_eq!(json["reasoning_effort"], "medium");

        prov.reasoning.effort = ReasoningEffort::High;
        let json = request_json(build_reasoning_params(&prov, false));
        assert_eq!(json["reasoning_effort"], "high");

        prov.reasoning.effort = ReasoningEffort::XHigh;
        let json = request_json(build_reasoning_params(&prov, false));
        assert_eq!(json["reasoning_effort"], "xhigh");
    }
}
