//! Cloud Text-to-Speech (TTS) client.
//!
//! Batch (REST) synthesis for the Reader and Chat speak modes:
//! - Deepgram (`/speak` with an Aura-2 voice model)
//! - OpenAI (`/audio/speech`, OpenAI-compatible)
//! - Google Cloud Text-to-Speech (`text:synthesize`, base64 `audioContent`)
//!
//! Providers are tried in settings priority order with automatic fallback,
//! mirroring [`crate::stt_client::transcribe_with_fallback`]. Long texts are
//! split with [`chunk_text_for_tts`] before synthesis. Playback lives in
//! `commands::tts`; this module only fetches audio bytes.

use crate::settings::{AppSettings, TtsFormat, TtsProvider};
use base64::Engine;
use log::{debug, info, warn};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use serde_json::Value;
use std::time::{Duration, Instant};

/// Per-provider hard cap for a single synthesis request, in characters.
/// The reader chunker stays below these limits.
pub fn max_chars_for_provider(provider_id: &str) -> usize {
    match provider_id {
        "deepgram" => 2000,
        "openai" => 4000,
        "google" => 5000,
        _ => 2000,
    }
}

/// One synthesis request. `voice`/`model` resolve from settings when empty.
#[derive(Debug, Clone)]
pub struct TtsRequest {
    pub text: String,
    pub voice: String,
    pub model: String,
    pub language: String,
    pub speaking_rate: f32,
    pub format: TtsFormat,
}

impl TtsRequest {
    pub fn new(text: String) -> Self {
        Self {
            text,
            voice: String::new(),
            model: String::new(),
            language: String::new(),
            speaking_rate: 1.0,
            format: TtsFormat::Mp3,
        }
    }
}

/// Synthesized audio bytes plus the encoding the provider returned.
#[derive(Debug, Clone)]
pub struct TtsAudio {
    pub bytes: Vec<u8>,
    pub format: TtsFormat,
}

impl TtsAudio {
    /// File extension matching the encoding (for temp files / debugging).
    pub fn extension(&self) -> &'static str {
        match self.format {
            TtsFormat::Mp3 => "mp3",
            TtsFormat::Wav => "wav",
        }
    }
}

/// Strip markdown/HTML markup down to speakable plain text.
///
/// Removes code fences, inline code, images/links (keeps link text),
/// headings, emphasis, blockquotes, list markers and HTML tags, then
/// collapses whitespace. Empty/markup-only input yields an empty string.
pub fn plain_text_for_tts(input: &str) -> String {
    let mut text = input.to_string();

    // Fenced code blocks -> keep inner text only.
    while let Some(start) = text.find("```") {
        let after = start + 3;
        if let Some(end) = text[after..].find("```") {
            let inner = text[after..after + end].to_string();
            // Drop an optional language tag on the first line.
            let inner = inner
                .split_once('\n')
                .map(|(_, rest)| rest)
                .unwrap_or(&inner);
            text.replace_range(start..after + end + 3, inner);
        } else {
            text.replace_range(start.., "");
            break;
        }
    }

    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    // Inside an HTML tag?
    let mut in_tag = false;
    while let Some(c) = chars.next() {
        if in_tag {
            if c == '>' {
                in_tag = false;
                out.push(' ');
            }
            continue;
        }
        match c {
            '<' => in_tag = true,
            // Inline code tick.
            '`' => {}
            // Markdown image: ![alt](url) -> alt.
            '!' if chars.peek() == Some(&'[') => {
                chars.next();
                let mut alt = String::new();
                for c in chars.by_ref() {
                    if c == ']' {
                        break;
                    }
                    alt.push(c);
                }
                // Skip optional (url).
                if chars.peek() == Some(&'(') {
                    for c in chars.by_ref() {
                        if c == ')' {
                            break;
                        }
                    }
                }
                out.push_str(&alt);
            }
            // Link: [text](url) -> text.
            '[' => {
                let mut label = String::new();
                let mut closed = false;
                for c in chars.by_ref() {
                    if c == ']' {
                        closed = true;
                        break;
                    }
                    label.push(c);
                }
                if closed && chars.peek() == Some(&'(') {
                    for c in chars.by_ref() {
                        if c == ')' {
                            break;
                        }
                    }
                    out.push_str(&label);
                } else {
                    out.push('[');
                    out.push_str(&label);
                    if closed {
                        out.push(']');
                    }
                }
            }
            // Emphasis / heading / quote / list markers become spaces.
            '*' | '_' | '#' | '>' | '|' | '~' => out.push(' '),
            // Horizontal rules / bullets on line starts.
            '-' | '+' => out.push(' '),
            _ => out.push(c),
        }
    }

    // HTML entities (common subset).
    let text = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");

    // Collapse whitespace runs.
    let mut collapsed = String::with_capacity(text.len());
    let mut last_space = true;
    for c in text.chars() {
        if c.is_whitespace() {
            if !last_space {
                collapsed.push(' ');
                last_space = true;
            }
        } else {
            collapsed.push(c);
            last_space = false;
        }
    }
    collapsed.trim().to_string()
}

/// Split text into chunks of at most `max_chars`, preferring sentence
/// boundaries (`.!?…` + CJK `。！？`). A single over-long sentence is
/// hard-split so no chunk ever exceeds the limit.
pub fn chunk_text_for_tts(text: &str, max_chars: usize) -> Vec<String> {
    let max_chars = max_chars.max(1);
    let plain = plain_text_for_tts(text);
    if plain.is_empty() {
        return Vec::new();
    }
    if plain.chars().count() <= max_chars {
        return vec![plain];
    }

    // Sentence split on boundary punctuation followed by whitespace or end.
    let mut sentences: Vec<String> = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = plain.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        current.push(c);
        let is_boundary = matches!(c, '.' | '!' | '?' | '…' | '。' | '！' | '？');
        if is_boundary {
            let next = chars.get(i + 1);
            if next.is_none() || next.is_some_and(|n| n.is_whitespace()) {
                let s = current.trim().to_string();
                if !s.is_empty() {
                    sentences.push(s);
                }
                current.clear();
            }
        }
        i += 1;
    }
    let tail = current.trim().to_string();
    if !tail.is_empty() {
        sentences.push(tail);
    }
    if sentences.is_empty() {
        sentences.push(plain);
    }

    // Greedily pack sentences; hard-split oversized ones.
    let mut chunks: Vec<String> = Vec::new();
    let mut buf = String::new();
    let mut buf_len = 0usize;
    let push_buf = |buf: &mut String, buf_len: &mut usize, chunks: &mut Vec<String>| {
        let s = buf.trim().to_string();
        if !s.is_empty() {
            chunks.push(s);
        }
        buf.clear();
        *buf_len = 0;
    };
    for sentence in sentences {
        let s_len = sentence.chars().count();
        if s_len > max_chars {
            if buf_len > 0 {
                push_buf(&mut buf, &mut buf_len, &mut chunks);
            }
            // Hard-split on char boundaries.
            let s_chars: Vec<char> = sentence.chars().collect();
            for part in s_chars.chunks(max_chars) {
                let part: String = part.iter().collect();
                let part = part.trim().to_string();
                if !part.is_empty() {
                    chunks.push(part);
                }
            }
            continue;
        }
        let sep = if buf_len == 0 { 0 } else { 1 };
        if buf_len + sep + s_len > max_chars {
            push_buf(&mut buf, &mut buf_len, &mut chunks);
        }
        if buf_len > 0 {
            buf.push(' ');
            buf_len += 1;
        }
        buf.push_str(&sentence);
        buf_len += s_len;
    }
    if buf_len > 0 {
        push_buf(&mut buf, &mut buf_len, &mut chunks);
    }
    chunks
}

/// Resolved per-request synthesis parameters for one provider.
struct ResolvedTts {
    voice: String,
    model: String,
    language: String,
    speaking_rate: f32,
    format: TtsFormat,
}

fn resolve_request(
    settings: &AppSettings,
    provider: &TtsProvider,
    req: &TtsRequest,
) -> ResolvedTts {
    let voice = if !req.voice.trim().is_empty() {
        req.voice.trim().to_string()
    } else {
        settings
            .tts_voices
            .get(&provider.id)
            .cloned()
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let voice = if voice.is_empty() {
        crate::settings::defaults::default_tts_voice_for_provider(&provider.id)
    } else {
        voice
    };
    let model = if !req.model.trim().is_empty() {
        req.model.trim().to_string()
    } else {
        settings
            .tts_models
            .get(&provider.id)
            .cloned()
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let model = if model.is_empty() {
        crate::settings::defaults::default_tts_model_for_provider(&provider.id)
    } else {
        model
    };
    let language = if !req.language.trim().is_empty() {
        req.language.trim().to_string()
    } else {
        settings.selected_language.clone()
    };
    let speaking_rate = if req.speaking_rate > 0.0 {
        req.speaking_rate.clamp(0.5, 2.0)
    } else {
        settings.effective_tts_speaking_rate()
    };
    ResolvedTts {
        voice,
        model,
        language,
        speaking_rate,
        format: req.format,
    }
}

fn base_client(provider: &TtsProvider) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(provider.timeout_seconds.max(10) as u64))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))
}

fn user_agent_headers(provider: &TtsProvider) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("Otush/1.0 (+https://github.com/ATClus/otush)"),
    );
    for (k, v) in &provider.custom_headers {
        if let (Ok(name), Ok(val)) = (
            reqwest::header::HeaderName::from_bytes(k.as_bytes()),
            HeaderValue::from_str(v),
        ) {
            headers.insert(name, val);
        }
    }
    headers
}

/// Whether a Deepgram voice id selects the Flux family (`flux-*`,
/// case-insensitive). Flux models only work on the `/v2/speak` endpoint;
/// everything else (Aura `aura-*`) uses `/v1/speak`.
pub fn is_deepgram_flux_voice(voice: &str) -> bool {
    voice.trim().len() >= 5 && voice.trim()[..5].eq_ignore_ascii_case("flux-")
}

/// Build the Deepgram `/speak` URL for a voice/format pair.
///
/// Flux voices go to `/v2/speak` (no `encoding`/`container` params — v2
/// returns mp3 for REST); Aura voices use `/v1/speak`, where `container` is
/// only sent for raw PCM (`linear16` → `wav`) since `mp3` rejects it (400).
pub fn deepgram_speak_url(base_url: &str, voice: &str, format: TtsFormat) -> String {
    // Strip any version suffix (`/v1`, `/v2`, trailing slashes) so the
    // version is always chosen by the voice family, never by settings drift.
    let base = base_url
        .trim_end_matches('/')
        .strip_suffix("/v2")
        .or_else(|| base_url.trim_end_matches('/').strip_suffix("/v1"))
        .unwrap_or_else(|| base_url.trim_end_matches('/'));
    let voice = voice.trim();
    if is_deepgram_flux_voice(voice) {
        return format!("{base}/v2/speak?model={voice}");
    }
    match format {
        TtsFormat::Wav => {
            format!("{base}/v1/speak?model={voice}&encoding=linear16&container=wav")
        }
        TtsFormat::Mp3 => format!("{base}/v1/speak?model={voice}&encoding=mp3"),
    }
}

/// Synthesize via Deepgram `/speak` (returns raw audio bytes).
async fn synthesize_deepgram(
    provider: &TtsProvider,
    api_key: &str,
    resolved: &ResolvedTts,
    text: &str,
) -> Result<TtsAudio, String> {
    let url = deepgram_speak_url(&provider.base_url, &resolved.voice, resolved.format);
    let client = base_client(provider)?;

    let mut headers = user_agent_headers(provider);
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Token {api_key}"))
            .map_err(|e| format!("Invalid API key: {e}"))?,
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    // Trace the full TTS flow (never log the API key itself: Deepgram
    // authenticates via the Authorization header, and the URL carries no
    // secret — only host/version/model/encoding params).
    let text_chars = text.chars().count();
    let text_preview: String = text.chars().take(160).collect();
    let url_path = url.split('?').next().unwrap_or(&url);
    debug!(
        "Deepgram TTS request: url={url_path} query=[{}] voice={} chars={} preview={:?}",
        url.split('?').nth(1).unwrap_or(""),
        resolved.voice,
        text_chars,
        text_preview,
    );
    let started = Instant::now();
    let body = serde_json::json!({ "text": text });
    let response = client
        .post(&url)
        .headers(headers)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Deepgram TTS request error: {e}"))?;

    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("?")
        .to_string();
    debug!(
        "Deepgram TTS response: status={} content_type={} elapsed_ms={}",
        status,
        content_type,
        started.elapsed().as_millis(),
    );
    if !status.is_success() {
        let err_text = response.text().await.unwrap_or_default();
        warn!(
            "Deepgram TTS failed: status={} voice={} chars={} body={:?}",
            status, resolved.voice, text_chars, err_text,
        );
        return Err(format!("Deepgram TTS error ({status}): {err_text}"));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read Deepgram TTS audio: {e}"))?;
    debug!(
        "Deepgram TTS audio: bytes={} voice={} chars={}",
        bytes.len(),
        resolved.voice,
        text_chars,
    );
    if bytes.is_empty() {
        return Err("Deepgram TTS returned empty audio".to_string());
    }
    Ok(TtsAudio {
        bytes: bytes.to_vec(),
        // v2 (Flux) REST always returns mp3 for batch synthesis.
        format: if is_deepgram_flux_voice(&resolved.voice) {
            TtsFormat::Mp3
        } else {
            resolved.format
        },
    })
}

/// Build the OpenAI `/audio/speech` request body.
pub fn openai_speech_body(model: &str, voice: &str, text: &str, format: TtsFormat) -> Value {
    let response_format = match format {
        TtsFormat::Wav => "wav",
        TtsFormat::Mp3 => "mp3",
    };
    serde_json::json!({
        "model": model,
        "voice": voice,
        "input": text,
        "response_format": response_format,
    })
}

/// Synthesize via OpenAI `/audio/speech` (returns raw audio bytes).
async fn synthesize_openai(
    provider: &TtsProvider,
    api_key: &str,
    resolved: &ResolvedTts,
    text: &str,
) -> Result<TtsAudio, String> {
    let base = provider.base_url.trim_end_matches('/');
    let url = format!("{base}/audio/speech");
    let client = base_client(provider)?;

    let mut headers = user_agent_headers(provider);
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {api_key}"))
            .map_err(|e| format!("Invalid API key: {e}"))?,
    );

    let model = if resolved.model.trim().is_empty() {
        "tts-1"
    } else {
        resolved.model.trim()
    };
    let body = openai_speech_body(model, &resolved.voice, text, resolved.format);
    let response = client
        .post(&url)
        .headers(headers)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("OpenAI TTS request error: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        let err_text = response.text().await.unwrap_or_default();
        return Err(format!("OpenAI TTS error ({status}): {err_text}"));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read OpenAI TTS audio: {e}"))?;
    if bytes.is_empty() {
        return Err("OpenAI TTS returned empty audio".to_string());
    }
    Ok(TtsAudio {
        bytes: bytes.to_vec(),
        format: resolved.format,
    })
}

/// Build the Google `text:synthesize` request body.
pub fn google_synthesize_body(
    text: &str,
    voice_name: &str,
    language: &str,
    speaking_rate: f32,
    format: TtsFormat,
) -> Value {
    let language_code = language_code_for_google(language, voice_name);
    let audio_encoding = match format {
        TtsFormat::Wav => "LINEAR16",
        TtsFormat::Mp3 => "MP3",
    };
    serde_json::json!({
        "input": { "text": text },
        "voice": { "languageCode": language_code, "name": voice_name },
        "audioConfig": { "audioEncoding": audio_encoding, "speakingRate": speaking_rate },
    })
}

/// Derive a BCP-47 language code for Google from the app language/voice.
/// An explicit voice like `pt-BR-Standard-A` wins; otherwise the app
/// `selected_language` (`pt-BR`, `en`, `auto`) maps to a default voice locale.
pub fn language_code_for_google(language: &str, voice_name: &str) -> String {
    // A full voice name already embeds its locale (xx-YY-...).
    let parts: Vec<&str> = voice_name.split('-').collect();
    if parts.len() >= 2 && parts[0].len() == 2 && parts[1].len() == 2 {
        return format!("{}-{}", parts[0], parts[1]);
    }
    let lang = language.trim();
    if lang.is_empty() || lang.eq_ignore_ascii_case("auto") {
        return "en-US".to_string();
    }
    if lang.contains('-') {
        return lang.to_string();
    }
    match lang.to_lowercase().as_str() {
        "pt" => "pt-BR".to_string(),
        "en" => "en-US".to_string(),
        "es" => "es-ES".to_string(),
        "fr" => "fr-FR".to_string(),
        "de" => "de-DE".to_string(),
        "it" => "it-IT".to_string(),
        other => other.to_string(),
    }
}

/// Synthesize via Google Cloud `text:synthesize` (base64 `audioContent`).
async fn synthesize_google(
    provider: &TtsProvider,
    api_key: &str,
    resolved: &ResolvedTts,
    text: &str,
) -> Result<TtsAudio, String> {
    let base = provider.base_url.trim_end_matches('/');
    // Simple Cloud API keys travel as `?key=`; OAuth-style tokens use Bearer.
    let looks_like_oauth = api_key.starts_with("ya29.") || api_key.len() > 120;
    let url = if looks_like_oauth {
        format!("{base}/text:synthesize")
    } else {
        format!("{base}/text:synthesize?key={api_key}")
    };
    let client = base_client(provider)?;

    let mut headers = user_agent_headers(provider);
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if looks_like_oauth {
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {api_key}"))
                .map_err(|e| format!("Invalid API key: {e}"))?,
        );
    }

    let body = google_synthesize_body(
        text,
        &resolved.voice,
        &resolved.language,
        resolved.speaking_rate,
        resolved.format,
    );
    let response = client
        .post(&url)
        .headers(headers)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Google TTS request error: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        let err_text = response.text().await.unwrap_or_default();
        return Err(format!("Google TTS error ({status}): {err_text}"));
    }
    let parsed: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Google TTS response: {e}"))?;
    let audio_b64 = parsed
        .get("audioContent")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if audio_b64.is_empty() {
        return Err("Google TTS response missing audioContent".to_string());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(audio_b64)
        .map_err(|e| format!("Failed to decode Google TTS audio: {e}"))?;
    if bytes.is_empty() {
        return Err("Google TTS returned empty audio".to_string());
    }
    Ok(TtsAudio {
        bytes,
        format: resolved.format,
    })
}

/// Synthesize one text with a single provider (no fallback).
async fn synthesize_with_provider(
    provider: &TtsProvider,
    settings: &AppSettings,
    req: &TtsRequest,
) -> Result<TtsAudio, String> {
    let api_key = settings.tts_api_key(&provider.id);
    if api_key.trim().is_empty() {
        return Err(format!(
            "Skipping provider '{}': missing API key",
            provider.label
        ));
    }
    let resolved = resolve_request(settings, provider, req);
    let text = plain_text_for_tts(&req.text);
    if text.is_empty() {
        return Err("Nothing to read: the text is empty".to_string());
    }
    let limit = max_chars_for_provider(&provider.id);
    if text.chars().count() > limit {
        return Err(format!(
            "Text too long for {} ({} chars, limit {limit}); split it first",
            provider.label,
            text.chars().count()
        ));
    }

    debug!(
        "Attempting TTS via provider '{}' (voice: {})",
        provider.label, resolved.voice
    );
    match provider.id.as_str() {
        "deepgram" => synthesize_deepgram(provider, &api_key, &resolved, &text).await,
        "google" => synthesize_google(provider, &api_key, &resolved, &text).await,
        _ => synthesize_openai(provider, &api_key, &resolved, &text).await,
    }
}

/// Synthesize using providers in priority order with automatic fallback.
///
/// The active provider (`tts_active_provider_id`) is tried first when
/// enabled; every other enabled provider follows in list order.
pub async fn synthesize_with_fallback(
    settings: &AppSettings,
    req: &TtsRequest,
) -> Result<TtsAudio, String> {
    let mut ordered: Vec<&TtsProvider> = Vec::new();
    if let Some(active) = settings.active_tts_provider() {
        ordered.push(active);
    }
    for provider in &settings.tts_providers {
        if provider.enabled && !ordered.iter().any(|p| p.id == provider.id) {
            ordered.push(provider);
        }
    }

    if ordered.is_empty() {
        return Err(
            "No TTS providers are enabled. Enable one in Settings -> Providers.".to_string(),
        );
    }

    let mut last_error = String::new();
    for provider in ordered {
        debug!(
            "TTS attempt: provider='{}' voice='{}' chars={}",
            provider.label,
            settings
                .tts_voices
                .get(&provider.id)
                .map(String::as_str)
                .unwrap_or(""),
            req.text.chars().count(),
        );
        match synthesize_with_provider(provider, settings, req).await {
            Ok(audio) => {
                info!(
                    "TTS succeeded via provider '{}' ({} bytes)",
                    provider.label,
                    audio.bytes.len()
                );
                return Ok(audio);
            }
            Err(err) => {
                if err.starts_with("Skipping provider") {
                    debug!("{err}");
                    continue;
                }
                warn!(
                    "TTS failed on provider '{}': {}. Trying next provider...",
                    provider.label, err
                );
                last_error = err;
            }
        }
    }

    Err(if last_error.is_empty() {
        "All TTS providers failed or lacked API keys.".to_string()
    } else {
        format!("All TTS providers failed. Last error: {last_error}")
    })
}

/// Test a TTS provider with a short sentence (no audio playback).
/// Returns `(audio_bytes, elapsed_ms)`.
pub async fn test_tts_provider(
    provider: &TtsProvider,
    settings: &AppSettings,
    api_key: String,
    voice: &str,
    model: &str,
) -> Result<(usize, u128), String> {
    let start = Instant::now();
    let effective_key = if api_key.trim().is_empty() {
        settings.tts_api_key(&provider.id)
    } else {
        api_key
    };
    if effective_key.trim().is_empty() {
        return Err(format!("Missing API key for '{}'", provider.label));
    }
    let mut scoped = settings.clone();
    scoped
        .tts_api_keys
        .insert(provider.id.clone(), effective_key);
    let req = TtsRequest {
        text: "Connected successfully.".to_string(),
        voice: voice.to_string(),
        model: model.to_string(),
        language: settings.selected_language.clone(),
        speaking_rate: 1.0,
        format: TtsFormat::Mp3,
    };
    let audio = synthesize_with_provider(provider, &scoped, &req).await?;
    Ok((audio.bytes.len(), start.elapsed().as_millis()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::defaults::get_default_settings;

    #[test]
    fn deepgram_speak_url_encodes_voice_and_format() {
        let wav = deepgram_speak_url(
            "https://api.deepgram.com/v1/",
            "aura-2-thalia-en",
            TtsFormat::Wav,
        );
        assert_eq!(
            wav,
            "https://api.deepgram.com/v1/speak?model=aura-2-thalia-en&encoding=linear16&container=wav"
        );
        let mp3 = deepgram_speak_url(
            "https://api.deepgram.com/v1",
            "aura-2-thalia-en",
            TtsFormat::Mp3,
        );
        assert_eq!(
            mp3,
            "https://api.deepgram.com/v1/speak?model=aura-2-thalia-en&encoding=mp3"
        );
    }

    #[test]
    fn deepgram_flux_voices_route_to_v2_without_media_params() {
        assert!(is_deepgram_flux_voice("flux-alexis-en"));
        assert!(is_deepgram_flux_voice(" FLUX-Thalia-En "));
        assert!(!is_deepgram_flux_voice("aura-2-thalia-en"));
        // v2 batch REST returns mp3: no encoding/container params.
        for format in [TtsFormat::Mp3, TtsFormat::Wav] {
            assert_eq!(
                deepgram_speak_url("https://api.deepgram.com/v1", "flux-alexis-en", format),
                "https://api.deepgram.com/v2/speak?model=flux-alexis-en"
            );
        }
        // A stale /v2 base in settings never leaks an Aura request onto v2.
        assert_eq!(
            deepgram_speak_url(
                "https://api.deepgram.com/v2",
                "aura-2-thalia-en",
                TtsFormat::Mp3
            ),
            "https://api.deepgram.com/v1/speak?model=aura-2-thalia-en&encoding=mp3"
        );
    }

    #[test]
    fn openai_speech_body_shape() {
        let body = openai_speech_body("tts-1", "alloy", "hello", TtsFormat::Mp3);
        assert_eq!(body["model"], "tts-1");
        assert_eq!(body["voice"], "alloy");
        assert_eq!(body["input"], "hello");
        assert_eq!(body["response_format"], "mp3");
        let wav = openai_speech_body("tts-1", "alloy", "hello", TtsFormat::Wav);
        assert_eq!(wav["response_format"], "wav");
    }

    #[test]
    fn google_body_uses_voice_locale_and_rate() {
        let body = google_synthesize_body("oi", "pt-BR-Standard-A", "auto", 1.25, TtsFormat::Mp3);
        assert_eq!(body["input"]["text"], "oi");
        assert_eq!(body["voice"]["languageCode"], "pt-BR");
        assert_eq!(body["voice"]["name"], "pt-BR-Standard-A");
        assert_eq!(body["audioConfig"]["audioEncoding"], "MP3");
        assert_eq!(body["audioConfig"]["speakingRate"], 1.25);
    }

    #[test]
    fn google_language_code_prefers_voice_locale() {
        assert_eq!(language_code_for_google("en", "pt-BR-Standard-A"), "pt-BR");
        assert_eq!(language_code_for_google("auto", "fancy"), "en-US");
        assert_eq!(language_code_for_google("pt-BR", "fancy"), "pt-BR");
        assert_eq!(language_code_for_google("pt", "fancy"), "pt-BR");
        assert_eq!(language_code_for_google("de", "fancy"), "de-DE");
    }

    #[test]
    fn plain_text_strips_markdown_and_html() {
        let md = "# Title\n\nHello **world**! See [docs](https://x) and `code`.\n\n```rust\nlet x = 1;\n```\n<p>para</p> &amp; more";
        let plain = plain_text_for_tts(md);
        assert!(plain.contains("Title"));
        assert!(plain.contains("Hello world"));
        assert!(plain.contains("docs"));
        assert!(!plain.contains("**"));
        assert!(!plain.contains("```"));
        assert!(!plain.contains("<p>"));
        assert!(plain.contains("& more"));
    }

    #[test]
    fn plain_text_of_markup_only_is_empty() {
        assert_eq!(plain_text_for_tts("**##** <br/> `code`"), "code");
        assert!(plain_text_for_tts("   ").is_empty());
    }

    #[test]
    fn chunker_keeps_short_text_whole() {
        let chunks = chunk_text_for_tts("Hello world.", 2000);
        assert_eq!(chunks, vec!["Hello world."]);
    }

    #[test]
    fn chunker_splits_on_sentences_within_limit() {
        let text = "First sentence here. Second sentence here! Third one? Yes.";
        let chunks = chunk_text_for_tts(text, 40);
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            assert!(chunk.chars().count() <= 40, "chunk too long: {chunk}");
        }
        let joined = chunks.join(" ");
        assert!(joined.contains("First sentence here."));
        assert!(joined.contains("Third one?"));
    }

    #[test]
    fn chunker_hard_splits_overlong_sentence() {
        let long = "a".repeat(300);
        let chunks = chunk_text_for_tts(&long, 100);
        assert_eq!(chunks.len(), 3);
        for chunk in &chunks {
            assert!(chunk.chars().count() <= 100);
        }
    }

    #[test]
    fn chunker_handles_cjk_boundaries() {
        let text = "你好世界。这是第二句！第三句？好的。";
        let chunks = chunk_text_for_tts(text, 12);
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            assert!(chunk.chars().count() <= 12, "chunk too long: {chunk}");
        }
    }

    #[test]
    fn chunker_empty_is_empty() {
        assert!(chunk_text_for_tts("   ", 100).is_empty());
    }

    #[test]
    fn tts_api_key_falls_back_to_stt_key() {
        let mut settings = get_default_settings();
        settings
            .transcription_api_keys
            .insert("deepgram".to_string(), "stt-key".to_string());
        assert_eq!(settings.tts_api_key("deepgram"), "stt-key");
        settings
            .tts_api_keys
            .insert("deepgram".to_string(), "tts-key".to_string());
        assert_eq!(settings.tts_api_key("deepgram"), "tts-key");
    }

    #[test]
    fn active_tts_provider_prefers_active_id() {
        let settings = get_default_settings();
        assert_eq!(
            settings.active_tts_provider().map(|p| p.id.as_str()),
            Some("deepgram")
        );
    }

    #[test]
    fn default_tts_catalog_has_three_providers() {
        let providers = crate::settings::defaults::default_tts_providers();
        assert_eq!(providers.len(), 3);
        assert_eq!(providers[0].id, "deepgram");
        assert_eq!(providers[1].id, "openai");
        assert_eq!(providers[2].id, "google");
        assert!(providers.iter().all(|p| p.enabled));
    }

    #[test]
    fn speaking_rate_clamps_to_provider_range() {
        let mut settings = get_default_settings();
        settings.tts_speaking_rate = 9.0;
        assert_eq!(settings.effective_tts_speaking_rate(), 2.0);
        settings.tts_speaking_rate = 0.0;
        assert_eq!(settings.effective_tts_speaking_rate(), 0.5);
    }
}
