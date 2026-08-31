//! External Cloud Speech-to-Text (STT) client.
//!
//! Provides integration with external transcription APIs:
//! - Deepgram (Nova-3 / Nova-2 via `/listen`)
//! - OpenAI (Whisper-1 via `/audio/transcriptions`)
//! - Google AI Studio (Gemini 2.0 Flash / 1.5 Flash multimodal audio)
//! - Groq (Whisper-large-v3 / Whisper-large-v3-turbo via `/audio/transcriptions`)
//! - Custom OpenAI-compatible STT endpoints

use crate::settings::{AppSettings, TranscriptionProvider};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use log::{debug, info, warn};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

/// Encode 16kHz mono 16-bit PCM audio samples into in-memory WAV byte stream.
pub fn samples_to_wav_bytes(samples: &[f32]) -> Vec<u8> {
    let mut buffer = Vec::with_capacity(44 + samples.len() * 2);
    let sample_rate: u32 = 16000;
    let num_channels: u16 = 1;
    let bits_per_sample: u16 = 16;
    let byte_rate: u32 = sample_rate * (num_channels as u32) * (bits_per_sample as u32 / 8);
    let block_align: u16 = num_channels * (bits_per_sample / 8);
    let subchunk2_size: u32 = (samples.len() * 2) as u32;
    let chunk_size: u32 = 36 + subchunk2_size;

    // RIFF Header
    buffer.extend_from_slice(b"RIFF");
    buffer.extend_from_slice(&chunk_size.to_le_bytes());
    buffer.extend_from_slice(b"WAVE");

    // fmt Subchunk
    buffer.extend_from_slice(b"fmt ");
    buffer.extend_from_slice(&16_u32.to_le_bytes()); // Subchunk1Size (16 for PCM)
    buffer.extend_from_slice(&1_u16.to_le_bytes()); // AudioFormat (1 for PCM)
    buffer.extend_from_slice(&num_channels.to_le_bytes());
    buffer.extend_from_slice(&sample_rate.to_le_bytes());
    buffer.extend_from_slice(&byte_rate.to_le_bytes());
    buffer.extend_from_slice(&block_align.to_le_bytes());
    buffer.extend_from_slice(&bits_per_sample.to_le_bytes());

    // data Subchunk
    buffer.extend_from_slice(b"data");
    buffer.extend_from_slice(&subchunk2_size.to_le_bytes());
    for &sample in samples {
        let sample_i16 = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        buffer.extend_from_slice(&sample_i16.to_le_bytes());
    }

    buffer
}

/// Generate a synthetic 0.5s audio signal for connection testing.
pub fn generate_test_audio_samples() -> Vec<f32> {
    let sample_count = 8000; // 0.5s at 16kHz
    let mut samples = Vec::with_capacity(sample_count);
    for i in 0..sample_count {
        let t = (i as f32) / 16000.0;
        let val = (t * 440.0 * 2.0 * std::f32::consts::PI).sin() * 0.5;
        samples.push(val);
    }
    samples
}

/// Build Deepgram query parameters for both REST and WebSocket endpoints.
pub fn build_deepgram_query_params(
    provider: &TranscriptionProvider,
    model: &str,
    language: &str,
    is_streaming: bool,
) -> Vec<(String, String)> {
    let config = provider.deepgram.clone().unwrap_or_default();
    let mut query_params: Vec<(String, String)> = Vec::new();
    query_params.push(("model".to_string(), model.to_string()));

    if is_streaming {
        query_params.push(("encoding".to_string(), "linear16".to_string()));
        query_params.push(("sample_rate".to_string(), "16000".to_string()));
        query_params.push(("channels".to_string(), "1".to_string()));
        query_params.push(("interim_results".to_string(), "true".to_string()));
    }

    let dg_lang = config.language.as_deref().unwrap_or("").trim();
    let effective_lang = if !dg_lang.is_empty() {
        dg_lang
    } else {
        language.trim()
    };

    if effective_lang.is_empty() || effective_lang.eq_ignore_ascii_case("auto") {
        query_params.push(("detect_language".to_string(), "true".to_string()));
    } else {
        query_params.push(("language".to_string(), effective_lang.to_string()));
    }

    if config.smart_format {
        query_params.push(("smart_format".to_string(), "true".to_string()));
    }
    if config.punctuate {
        query_params.push(("punctuate".to_string(), "true".to_string()));
    }
    if config.numerals {
        query_params.push(("numerals".to_string(), "true".to_string()));
    }
    if config.paragraphs {
        query_params.push(("paragraphs".to_string(), "true".to_string()));
    }
    if config.diarize {
        query_params.push(("diarize".to_string(), "true".to_string()));
    }
    if config.filler_words {
        query_params.push(("filler_words".to_string(), "true".to_string()));
    }
    if config.profanity_filter {
        query_params.push(("profanity_filter".to_string(), "true".to_string()));
    }
    for kw in &config.keywords {
        let kw_trimmed = kw.trim();
        if !kw_trimmed.is_empty() {
            query_params.push(("keywords".to_string(), kw_trimmed.to_string()));
        }
    }
    for (k, v) in &config.extra_query_params {
        let k_trim = k.trim();
        if !k_trim.is_empty() {
            query_params.push((k_trim.to_string(), v.trim().to_string()));
        }
    }

    query_params
}

/// Run live WebSocket streaming with Deepgram.
pub(crate) async fn run_deepgram_websocket_stream(
    provider: &TranscriptionProvider,
    api_key: &str,
    model: &str,
    language: &str,
    rx: std::sync::mpsc::Receiver<crate::managers::transcription::StreamCmd>,
    bus: crate::context::EventBus,
    stream_active: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let base_url = provider.base_url.trim_end_matches('/');
    let ws_base = if base_url.starts_with("https://") {
        base_url.replacen("https://", "wss://", 1)
    } else if base_url.starts_with("http://") {
        base_url.replacen("http://", "ws://", 1)
    } else {
        format!("wss://{}", base_url.trim_start_matches('/'))
    };

    let ws_path = if ws_base.ends_with("/listen") {
        ws_base
    } else {
        format!("{}/listen", ws_base)
    };

    let query_params = build_deepgram_query_params(provider, model, language, true);
    let mut url = match reqwest::Url::parse(&ws_path) {
        Ok(u) => u,
        Err(e) => {
            warn!("Invalid Deepgram WebSocket URL: {e}");
            drain_and_fail(rx);
            return;
        }
    };
    {
        let mut pairs = url.query_pairs_mut();
        for (k, v) in &query_params {
            pairs.append_pair(k, v);
        }
    }

    let mut request = match url.as_str().into_client_request() {
        Ok(r) => r,
        Err(e) => {
            warn!("Failed to create Deepgram WebSocket request: {e}");
            drain_and_fail(rx);
            return;
        }
    };

    let auth_val = match HeaderValue::from_str(&format!("Token {}", api_key)) {
        Ok(v) => v,
        Err(e) => {
            warn!("Invalid Deepgram API key header value: {e}");
            drain_and_fail(rx);
            return;
        }
    };
    request.headers_mut().insert(AUTHORIZATION, auth_val);
    request.headers_mut().insert(
        USER_AGENT,
        HeaderValue::from_static("Otush/1.0 (+https://github.com/ATClus/otush)"),
    );

    for (k, v) in &provider.custom_headers {
        if let (Ok(name), Ok(val)) = (
            reqwest::header::HeaderName::from_bytes(k.as_bytes()),
            HeaderValue::from_str(v),
        ) {
            request.headers_mut().insert(name, val);
        }
    }

    let ws_stream = match tokio_tungstenite::connect_async(request).await {
        Ok((stream, _)) => stream,
        Err(e) => {
            warn!(
                "Deepgram WebSocket live stream connection failed: {e}. Will fall back to batch."
            );
            drain_and_fail(rx);
            return;
        }
    };

    stream_active.store(true, std::sync::atomic::Ordering::Release);
    info!(
        "Deepgram live WebSocket streaming connected (model: {})",
        model
    );

    let (mut ws_write, mut ws_read) = ws_stream.split();

    let committed_text = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let tentative_text = std::sync::Arc::new(std::sync::Mutex::new(String::new()));

    let committed_for_reader = Arc::clone(&committed_text);
    let tentative_for_reader = Arc::clone(&tentative_text);
    let bus_clone = bus.clone();

    // Reader task: receive real-time JSON transcripts from Deepgram WebSocket
    let reader_task = tokio::spawn(async move {
        while let Some(msg_res) = ws_read.next().await {
            match msg_res {
                Ok(Message::Text(text)) => {
                    if let Ok(val) = serde_json::from_str::<Value>(&text) {
                        let msg_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        if msg_type == "Results" {
                            let is_final = val
                                .get("is_final")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            let transcript = val
                                .pointer("/channel/alternatives/0/transcript")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .trim();

                            let mut comm = committed_for_reader.lock().unwrap();
                            let mut tent = tentative_for_reader.lock().unwrap();

                            if is_final {
                                if !transcript.is_empty() {
                                    if !comm.is_empty() && !comm.ends_with(' ') {
                                        comm.push(' ');
                                    }
                                    comm.push_str(transcript);
                                }
                                tent.clear();
                            } else {
                                *tent = transcript.to_string();
                            }

                            bus_clone.send(crate::context::AppEvent::StreamText(
                                crate::managers::transcription::StreamTextEvent {
                                    committed: comm.clone(),
                                    tentative: tent.clone(),
                                },
                            ));
                        }
                    }
                }
                Ok(Message::Close(_)) => {
                    debug!("Deepgram WebSocket closed by server");
                    break;
                }
                Err(e) => {
                    warn!("Deepgram WebSocket read error: {e}");
                    break;
                }
                _ => {}
            }
        }
    });

    // Main worker loop: read audio frames and commands from rx channel
    while let Ok(cmd) = rx.recv() {
        match cmd {
            crate::managers::transcription::StreamCmd::Feed(pcm) => {
                if !pcm.is_empty() {
                    let mut pcm_bytes = Vec::with_capacity(pcm.len() * 2);
                    for &sample in &pcm {
                        let sample_i16 = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                        pcm_bytes.extend_from_slice(&sample_i16.to_le_bytes());
                    }
                    if let Err(e) = ws_write.send(Message::Binary(pcm_bytes.into())).await {
                        warn!("Deepgram WebSocket write feed error: {e}");
                        break;
                    }
                }
            }
            crate::managers::transcription::StreamCmd::Finalize(reply) => {
                // Send zero-byte binary frame or CloseStream to tell Deepgram audio is finished
                let _ = ws_write.send(Message::Binary(Vec::new().into())).await;
                let _ = ws_write
                    .send(Message::Text(r#"{"type":"CloseStream"}"#.into()))
                    .await;

                // Wait up to 600ms for reader to drain any remaining final message
                let _ = tokio::time::timeout(Duration::from_millis(600), reader_task).await;

                let comm = committed_text.lock().unwrap().clone();
                let tent = tentative_text.lock().unwrap().clone();
                let mut full_text = comm;
                if !tent.is_empty() {
                    if !full_text.is_empty() && !full_text.ends_with(' ') {
                        full_text.push(' ');
                    }
                    full_text.push_str(&tent);
                }

                let final_trimmed = full_text.trim().to_string();
                if !final_trimmed.is_empty() {
                    let _ = reply.send(Some(crate::managers::transcription::FinalizedStreamText {
                        text: final_trimmed,
                        output_language: crate::audio_toolkit::OutputLanguageEvidence::Unknown,
                        supported_languages: Vec::new(),
                    }));
                } else {
                    let _ = reply.send(None);
                }
                let _ = ws_write.close().await;
                return;
            }
            crate::managers::transcription::StreamCmd::Cancel => {
                reader_task.abort();
                let _ = ws_write.close().await;
                return;
            }
        }
    }
}

fn drain_and_fail(rx: std::sync::mpsc::Receiver<crate::managers::transcription::StreamCmd>) {
    while let Ok(cmd) = rx.recv() {
        match cmd {
            crate::managers::transcription::StreamCmd::Finalize(reply) => {
                let _ = reply.send(None);
                return;
            }
            crate::managers::transcription::StreamCmd::Cancel => {
                return;
            }
            crate::managers::transcription::StreamCmd::Feed(_) => {}
        }
    }
}

/// Transcribe audio using Deepgram REST API (`/v1/listen`).
async fn transcribe_deepgram(
    provider: &TranscriptionProvider,
    api_key: &str,
    model: &str,
    language: &str,
    wav_bytes: Vec<u8>,
) -> Result<String, String> {
    let base_url = provider.base_url.trim_end_matches('/');
    let url = format!("{}/listen", base_url);

    let config = provider.deepgram.clone().unwrap_or_default();
    let query_params = build_deepgram_query_params(provider, model, language, false);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(provider.timeout_seconds as u64))
        .build()
        .map_err(|e| format!("Failed to create client: {}", e))?;

    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Token {}", api_key))
            .map_err(|e| format!("Invalid API key: {}", e))?,
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("audio/wav"));
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

    let response = client
        .post(&url)
        .headers(headers)
        .query(&query_params)
        .body(wav_bytes)
        .send()
        .await
        .map_err(|e| format!("Deepgram request error: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        let err_text = response.text().await.unwrap_or_default();
        return Err(format!("Deepgram error ({}): {}", status, err_text));
    }

    let parsed: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Deepgram response: {}", e))?;

    let paragraph_transcript = if config.paragraphs {
        parsed
            .pointer("/results/channels/0/alternatives/0/paragraphs/transcript")
            .and_then(|t| t.as_str())
    } else {
        None
    };

    let transcript = paragraph_transcript
        .or_else(|| {
            parsed
                .pointer("/results/channels/0/alternatives/0/transcript")
                .and_then(|t| t.as_str())
        })
        .unwrap_or_default();

    Ok(transcript.trim().to_string())
}

/// Transcribe audio using an OpenAI-compatible audio transcriptions endpoint (OpenAI, Groq, Custom).
async fn transcribe_openai_compatible(
    provider: &TranscriptionProvider,
    api_key: &str,
    model: &str,
    language: &str,
    wav_bytes: Vec<u8>,
) -> Result<String, String> {
    let base_url = provider.base_url.trim_end_matches('/');
    let url = format!("{}/audio/transcriptions", base_url);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(provider.timeout_seconds as u64))
        .build()
        .map_err(|e| format!("Failed to create client: {}", e))?;

    let mut headers = HeaderMap::new();
    if !api_key.is_empty() {
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", api_key))
                .map_err(|e| format!("Invalid API key: {}", e))?,
        );
    }
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

    let part = reqwest::multipart::Part::bytes(wav_bytes)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| format!("Failed to build multipart payload: {}", e))?;

    let mut form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model", model.to_string())
        .text("response_format", "json".to_string());

    let lang_trimmed = language.trim();
    if !lang_trimmed.is_empty() && !lang_trimmed.eq_ignore_ascii_case("auto") {
        let lang_code = lang_trimmed.split('-').next().unwrap_or(lang_trimmed);
        form = form.text("language", lang_code.to_string());
    }

    let response = client
        .post(&url)
        .headers(headers)
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("OpenAI-compatible STT request error: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        let err_text = response.text().await.unwrap_or_default();
        return Err(format!("STT error ({}): {}", status, err_text));
    }

    let parsed: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse STT response: {}", e))?;

    let text = parsed
        .get("text")
        .and_then(|t| t.as_str())
        .unwrap_or_default();

    Ok(text.trim().to_string())
}

/// Transcribe audio using Google Gemini multimodal audio API (AI Studio).
async fn transcribe_google_gemini(
    provider: &TranscriptionProvider,
    api_key: &str,
    model: &str,
    language: &str,
    wav_bytes: Vec<u8>,
) -> Result<String, String> {
    let base_url = provider.base_url.trim_end_matches('/');
    let url = format!(
        "{}/models/{}:generateContent?key={}",
        base_url, model, api_key
    );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(provider.timeout_seconds as u64))
        .build()
        .map_err(|e| format!("Failed to create client: {}", e))?;

    let b64 = base64::engine::general_purpose::STANDARD.encode(&wav_bytes);

    let prompt_text = if !language.trim().is_empty()
        && !language.trim().eq_ignore_ascii_case("auto")
    {
        format!("Transcribe the spoken audio verbatim in {}. Output only the exact transcribed speech text without any timestamps, explanation, or commentary.", language.trim())
    } else {
        "Transcribe the spoken audio verbatim. Output only the exact transcribed speech text without any timestamps, explanation, or commentary.".to_string()
    };

    let body = serde_json::json!({
        "contents": [{
            "parts": [
                {
                    "inline_data": {
                        "mime_type": "audio/wav",
                        "data": b64
                    }
                },
                {
                    "text": prompt_text
                }
            ]
        }],
        "generationConfig": {
            "temperature": 0.0
        }
    });

    let response = client
        .post(&url)
        .header(CONTENT_TYPE, "application/json")
        .header(USER_AGENT, "Otush/1.0 (+https://github.com/ATClus/otush)")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Gemini audio request error: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        let err_text = response.text().await.unwrap_or_default();
        return Err(format!("Google Gemini error ({}): {}", status, err_text));
    }

    let parsed: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Gemini response: {}", e))?;

    let text = parsed
        .pointer("/candidates/0/content/parts/0/text")
        .and_then(|t| t.as_str())
        .unwrap_or_default();

    Ok(text.trim().to_string())
}

/// Transcribe audio samples using the cloud providers in priority order with automatic fallback.
pub async fn transcribe_with_fallback(
    settings: &AppSettings,
    samples: &[f32],
) -> Result<String, String> {
    let wav_bytes = samples_to_wav_bytes(samples);

    let enabled_providers: Vec<&TranscriptionProvider> = settings
        .transcription_providers
        .iter()
        .filter(|p| p.enabled)
        .collect();

    if enabled_providers.is_empty() {
        return Err(
            "No cloud transcription providers are enabled. Please enable at least one provider in Settings -> Providers."
                .to_string(),
        );
    }

    let language = &settings.selected_language;
    let mut last_error = String::new();

    for provider in enabled_providers {
        let api_key = settings
            .transcription_api_keys
            .get(&provider.id)
            .cloned()
            .unwrap_or_default();

        if api_key.trim().is_empty() && provider.id != "custom" {
            debug!("Skipping provider '{}': missing API key", provider.label);
            continue;
        }

        let model = settings
            .transcription_models
            .get(&provider.id)
            .cloned()
            .unwrap_or_else(|| provider.model.clone());

        debug!(
            "Attempting cloud transcription via provider '{}' (model: {})",
            provider.label, model
        );

        let result = match provider.id.as_str() {
            "deepgram" => {
                transcribe_deepgram(provider, &api_key, &model, language, wav_bytes.clone()).await
            }
            "gemini" => {
                transcribe_google_gemini(provider, &api_key, &model, language, wav_bytes.clone())
                    .await
            }
            _ => {
                transcribe_openai_compatible(
                    provider,
                    &api_key,
                    &model,
                    language,
                    wav_bytes.clone(),
                )
                .await
            }
        };

        match result {
            Ok(transcript) => {
                info!(
                    "Cloud transcription succeeded via provider '{}' (model: {})",
                    provider.label, model
                );
                return Ok(transcript);
            }
            Err(err) => {
                warn!(
                    "Cloud transcription failed on provider '{}': {}. Trying next provider in fallback chain...",
                    provider.label, err
                );
                last_error = err;
            }
        }
    }

    Err(if last_error.is_empty() {
        "All cloud transcription providers failed or lacked API keys.".to_string()
    } else {
        format!(
            "All cloud transcription providers failed. Last error: {}",
            last_error
        )
    })
}

/// Test connection and transcription latency for a cloud provider.
pub async fn test_transcription_provider(
    provider: &TranscriptionProvider,
    api_key: String,
    model: &str,
    language: &str,
) -> Result<(String, u128), String> {
    let start = Instant::now();
    let samples = generate_test_audio_samples();
    let wav_bytes = samples_to_wav_bytes(&samples);

    let result = match provider.id.as_str() {
        "deepgram" => transcribe_deepgram(provider, &api_key, model, language, wav_bytes).await,
        "gemini" => transcribe_google_gemini(provider, &api_key, model, language, wav_bytes).await,
        _ => transcribe_openai_compatible(provider, &api_key, model, language, wav_bytes).await,
    };

    let elapsed = start.elapsed().as_millis();
    match result {
        Ok(text) => Ok((text, elapsed)),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_samples_to_wav_bytes_creates_valid_header() {
        let samples = vec![0.0_f32; 1600]; // 0.1s
        let wav = samples_to_wav_bytes(&samples);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(wav.len(), 44 + 3200);
    }

    #[test]
    fn test_generate_test_audio_samples() {
        let samples = generate_test_audio_samples();
        assert_eq!(samples.len(), 8000);
        for &s in &samples {
            assert!((-1.0..=1.0).contains(&s));
        }
    }

    #[test]
    fn test_default_transcription_providers_structure() {
        let providers = crate::settings::default_transcription_providers();
        assert_eq!(providers.len(), 5);
        assert_eq!(providers[0].id, "deepgram");
        assert_eq!(providers[1].id, "groq");
        assert_eq!(providers[2].id, "openai");
        assert_eq!(providers[3].id, "gemini");
        assert_eq!(providers[4].id, "custom");

        let dg = &providers[0];
        assert!(dg.deepgram.is_some());
        let dg_cfg = dg.deepgram.as_ref().unwrap();
        assert!(dg_cfg.smart_format);
        assert!(dg_cfg.punctuate);
        assert!(dg_cfg.numerals);
        assert!(!dg_cfg.paragraphs);
        assert!(!dg_cfg.diarize);
        assert!(!dg_cfg.filler_words);
        assert!(!dg_cfg.profanity_filter);
    }

    #[test]
    fn test_deepgram_config_default_and_backward_compatibility() {
        let json_data = r#"{
            "id": "deepgram",
            "label": "Deepgram",
            "base_url": "https://api.deepgram.com/v1",
            "model": "nova-3",
            "enabled": true,
            "allow_base_url_edit": false,
            "timeout_seconds": 15,
            "custom_headers": {}
        }"#;

        let provider: TranscriptionProvider = serde_json::from_str(json_data).unwrap();
        assert_eq!(provider.id, "deepgram");
        assert!(provider.deepgram.is_none());
        let effective_config = provider.deepgram.unwrap_or_default();
        assert!(effective_config.smart_format);
        assert!(effective_config.punctuate);
        assert!(effective_config.numerals);
        assert_eq!(effective_config.language, None);
    }

    #[test]
    fn test_deepgram_config_with_language_override() {
        let json_data = r#"{
            "id": "deepgram",
            "label": "Deepgram",
            "base_url": "https://api.deepgram.com/v1",
            "model": "nova-3",
            "enabled": true,
            "allow_base_url_edit": false,
            "timeout_seconds": 15,
            "custom_headers": {},
            "deepgram": {
                "language": "pt-BR",
                "smart_format": true,
                "punctuate": true,
                "numerals": true
            }
        }"#;

        let provider: TranscriptionProvider = serde_json::from_str(json_data).unwrap();
        assert_eq!(provider.id, "deepgram");
        assert!(provider.deepgram.is_some());
        let dg_cfg = provider.deepgram.unwrap();
        assert_eq!(dg_cfg.language.as_deref(), Some("pt-BR"));
        assert!(dg_cfg.numerals);
    }
}
