//! Direct URL and Hugging Face model downloader.
//!
//! Downloads GGUF, GGML, and ONNX models directly into the local models directory
//! with streaming progress reporting and cancellation support.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use log::{info, warn};
use tokio::io::AsyncWriteExt;

use crate::context::{
    AppContext, AppEvent, ModelDownloadFinishedEvent, ModelDownloadProgressEvent,
};
use std::sync::LazyLock;

static CANCEL_DOWNLOAD: LazyLock<std::sync::Mutex<Option<Arc<AtomicBool>>>> =
    LazyLock::new(|| std::sync::Mutex::new(None));

/// Parse and normalize user input into a direct download URL and suggested filename.
pub fn normalize_model_url(input: &str) -> Result<(String, String), String> {
    let raw = input.trim();
    if raw.is_empty() {
        return Err("Please enter a valid URL or Hugging Face repository link.".to_string());
    }

    // 1. Shorthand Hugging Face format: "org/repo/filename.ext" or "org/repo:filename.ext"
    let is_http = raw.starts_with("http://") || raw.starts_with("https://");
    if !is_http {
        let parts: Vec<&str> = raw.split('/').collect();
        if parts.len() == 3 {
            let org = parts[0];
            let repo = parts[1];
            let file = parts[2];
            let url = format!(
                "https://huggingface.co/{}/{}/resolve/main/{}?download=true",
                org, repo, file
            );
            return Ok((url, file.to_string()));
        } else {
            return Err(
                "Invalid format. Provide a full URL (https://huggingface.co/...) or 'owner/repo/model.gguf'."
                    .to_string(),
            );
        }
    }

    let parsed_url = reqwest::Url::parse(raw).map_err(|e| format!("Invalid URL: {e}"))?;

    // 2. Hugging Face web URLs: convert '/blob/' to '/resolve/' and ensure '?download=true'
    if let Some(host) = parsed_url.host_str() {
        if host.contains("huggingface.co") || host.contains("hf.co") {
            let path = parsed_url.path();
            let resolved_path = if path.contains("/blob/") {
                path.replace("/blob/", "/resolve/")
            } else {
                path.to_string()
            };

            let filename = resolved_path
                .split('/')
                .next_back()
                .unwrap_or("model.gguf")
                .to_string();

            let clean_filename = filename.split('?').next().unwrap_or(&filename).to_string();

            let mut final_url = format!("https://huggingface.co{}", resolved_path);
            if !final_url.contains("download=true") {
                if final_url.contains('?') {
                    final_url.push_str("&download=true");
                } else {
                    final_url.push_str("?download=true");
                }
            }
            return Ok((final_url, clean_filename));
        }
    }

    // 3. Direct HTTP/HTTPS link to any file
    let path = parsed_url.path();
    let filename = path
        .split('/')
        .next_back()
        .filter(|s| !s.is_empty())
        .unwrap_or("model.gguf")
        .to_string();

    let clean_filename = filename.split('?').next().unwrap_or(&filename).to_string();

    Ok((raw.to_string(), clean_filename))
}

/// Download a model from a URL or Hugging Face spec to the models directory.
pub async fn download_model(ctx: &AppContext, url_or_spec: &str) -> Result<PathBuf, String> {
    let (url, filename) = normalize_model_url(url_or_spec)?;

    let models_dir = ctx.model.models_dir();
    std::fs::create_dir_all(models_dir)
        .map_err(|e| format!("Failed to create models folder: {e}"))?;

    let target_file = models_dir.join(&filename);
    let part_file = models_dir.join(format!("{}.part", filename));

    info!(
        "Starting model download from '{}' to '{:?}'",
        url, target_file
    );

    let cancel_flag = Arc::new(AtomicBool::new(false));
    {
        let mut guard = CANCEL_DOWNLOAD.lock().unwrap();
        *guard = Some(Arc::clone(&cancel_flag));
    }

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("Failed to initialize HTTP client: {e}"))?;

    let response = client
        .get(&url)
        .header("User-Agent", "Otush/1.0 (Linux)")
        .send()
        .await
        .map_err(|e| format!("Network request failed: {e}"))?;

    if !response.status().is_success() {
        let err_msg = format!("HTTP error: {}", response.status());
        ctx.bus.send(AppEvent::ModelDownloadFinished(
            ModelDownloadFinishedEvent {
                filename: filename.clone(),
                success: false,
                error: Some(err_msg.clone()),
            },
        ));
        return Err(err_msg);
    }

    let total_bytes = response.content_length();
    let mut stream = response.bytes_stream();

    let mut out_file = tokio::fs::File::create(&part_file)
        .await
        .map_err(|e| format!("Failed to create local file '{:?}': {e}", part_file))?;

    let mut downloaded_bytes: u64 = 0;
    let start_time = Instant::now();
    let mut last_ui_update = Instant::now();

    while let Some(chunk_result) = stream.next().await {
        if cancel_flag.load(Ordering::SeqCst) {
            let _ = tokio::fs::remove_file(&part_file).await;
            let err_msg = "Download cancelled by user".to_string();
            ctx.bus.send(AppEvent::ModelDownloadFinished(
                ModelDownloadFinishedEvent {
                    filename: filename.clone(),
                    success: false,
                    error: Some(err_msg.clone()),
                },
            ));
            return Err(err_msg);
        }

        let chunk = chunk_result.map_err(|e| format!("Download stream error: {e}"))?;
        out_file
            .write_all(&chunk)
            .await
            .map_err(|e| format!("Write to disk error: {e}"))?;

        downloaded_bytes += chunk.len() as u64;

        if last_ui_update.elapsed() >= Duration::from_millis(150) {
            let elapsed_secs = start_time.elapsed().as_secs_f64();
            let speed_mb_s = if elapsed_secs > 0.0 {
                (downloaded_bytes as f64 / (1024.0 * 1024.0)) / elapsed_secs
            } else {
                0.0
            };

            let percentage = if let Some(total) = total_bytes {
                if total > 0 {
                    (downloaded_bytes as f64 / total as f64) * 100.0
                } else {
                    0.0
                }
            } else {
                0.0
            };

            ctx.bus.send(AppEvent::ModelDownloadProgress(
                ModelDownloadProgressEvent {
                    url: url.clone(),
                    filename: filename.clone(),
                    downloaded_bytes,
                    total_bytes,
                    percentage,
                    speed_mb_s,
                },
            ));
            last_ui_update = Instant::now();
        }
    }

    out_file
        .flush()
        .await
        .map_err(|e| format!("Failed to flush file to disk: {e}"))?;
    drop(out_file);

    // Atomic move from part file to target file
    std::fs::rename(&part_file, &target_file)
        .map_err(|e| format!("Failed to finalize model file: {e}"))?;

    info!(
        "Model successfully downloaded: '{:?}' ({} bytes)",
        target_file, downloaded_bytes
    );

    // Clean cancellation flag
    {
        let mut guard = CANCEL_DOWNLOAD.lock().unwrap();
        *guard = None;
    }

    // Rescan local models and refresh UI
    if let Err(e) = ctx.model.rescan_local_models() {
        warn!("Failed to rescan models after download: {e}");
    }

    ctx.bus.send(AppEvent::ModelDownloadFinished(
        ModelDownloadFinishedEvent {
            filename: filename.clone(),
            success: true,
            error: None,
        },
    ));
    ctx.bus.send(AppEvent::ModelsUpdated);

    Ok(target_file)
}

/// Cancel any active model download.
pub fn cancel_download() {
    let mut guard = CANCEL_DOWNLOAD.lock().unwrap();
    if let Some(flag) = guard.take() {
        flag.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_huggingface_blob_url() {
        let (url, filename) = normalize_model_url(
            "https://huggingface.co/ggerganov/whisper.cpp/blob/main/ggml-small.bin",
        )
        .unwrap();
        assert_eq!(filename, "ggml-small.bin");
        assert_eq!(
            url,
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin?download=true"
        );
    }

    #[test]
    fn test_normalize_huggingface_shorthand() {
        let (url, filename) =
            normalize_model_url("ggerganov/whisper.cpp/ggml-base.en.bin").unwrap();
        assert_eq!(filename, "ggml-base.en.bin");
        assert_eq!(
            url,
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin?download=true"
        );
    }

    #[test]
    fn test_normalize_direct_url() {
        let (url, filename) =
            normalize_model_url("https://example.com/models/whisper-large.gguf").unwrap();
        assert_eq!(filename, "whisper-large.gguf");
        assert_eq!(url, "https://example.com/models/whisper-large.gguf");
    }
}
