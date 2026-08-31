//! Local-only Model Manager for Otush.
//!
//! Models are stored locally in the application models directory
//! (`~/.local/share/com.clusterat.otush/models` or `./data/models` in portable mode).
//!
//! The manager auto-discovers:
//! - Whisper GGUF models (`.gguf`) with automatic header capability probing.
//! - Legacy GGML models (`.bin`).
//! - ONNX models (`.onnx` or directories).

pub mod download;

use super::model_capabilities::{CapabilityProber, GgufHeaderProber};
use crate::context::{AppEvent, AppPaths, EventBus};
use crate::settings::{read_settings_from, write_settings_to};
use anyhow::Result;
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EngineType {
    /// Any GGML/GGUF model loaded through transcribe-cpp (Whisper, Parakeet, Voxtral, etc.)
    TranscribeCpp,
    Parakeet,
    Moonshine,
    MoonshineStreaming,
    SenseVoice,
    GigaAM,
    Canary,
    Cohere,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ModelSource {
    Local,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub filename: String,
    pub source: ModelSource,
    pub size_mb: u64,
    pub is_downloaded: bool,
    pub is_downloading: bool,
    pub partial_size: u64,
    pub is_directory: bool,
    pub engine_type: EngineType,
    pub accuracy_score: f32,
    pub speed_score: f32,
    pub supports_translation: bool,
    pub is_recommended: bool,
    pub supported_languages: Vec<String>,
    pub supports_language_selection: bool,
    pub is_custom: bool,
    pub supports_streaming: bool,
    pub supports_language_detection: bool,
}

const CHINESE_LANGUAGE_CODE: &str = "zh";

fn recognition_language(language: &str) -> &str {
    match language {
        "zh-Hans" | "zh-Hant" => CHINESE_LANGUAGE_CODE,
        other => other,
    }
}

fn base_language(language: &str) -> &str {
    match language.split_once('-') {
        Some((base, _)) => base,
        None => language,
    }
}

fn canonicalize_supported_languages(languages: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut canonical = Vec::with_capacity(languages.len());

    for language in languages {
        let language = recognition_language(&language).to_string();
        if seen.insert(language.clone()) {
            canonical.push(language);
        }
    }

    canonical
}

/// Resolve the user's persisted language intent into the language a given model supports.
pub fn effective_language(
    intent: &str,
    supported_languages: &[String],
    supports_language_detection: bool,
) -> String {
    if supported_languages.is_empty() {
        return intent.to_string();
    }

    if intent != "auto" {
        if let Some(code) = supported_languages
            .iter()
            .find(|language| base_language(language) == base_language(intent))
        {
            if intent == "zh-Hans" || intent == "zh-Hant" {
                return intent.to_string();
            }
            return code.clone();
        }
    }

    if supports_language_detection {
        return "auto".to_string();
    }

    if let Some(en) = supported_languages
        .iter()
        .find(|language| base_language(language) == "en")
    {
        return en.clone();
    }
    recognition_language(&supported_languages[0]).to_string()
}

pub struct ModelManager {
    bus: EventBus,
    paths: AppPaths,
    models_dir: PathBuf,
    available_models: Mutex<HashMap<String, ModelInfo>>,
    is_rescanning: Arc<AtomicBool>,
}

impl ModelManager {
    pub fn new(paths: &AppPaths, bus: EventBus) -> Result<Self> {
        let models_dir = paths.models_dir();
        if let Err(e) = fs::create_dir_all(&models_dir) {
            warn!("Failed to create models directory {:?}: {}", models_dir, e);
        }

        let mut available_models = HashMap::new();

        // 1. Discover models in ~/.local/share/com.clusterat.otush/models
        if let Err(e) = Self::discover_models_in(&models_dir, &mut available_models) {
            warn!("Failed to discover models in {:?}: {}", models_dir, e);
        }

        // 2. Also check resources/models for any bundled/packaged models
        let resource_models = paths.resource_dir.join("models");
        if resource_models.exists() && resource_models != models_dir {
            let _ = Self::discover_models_in(&resource_models, &mut available_models);
        }

        let manager = Self {
            bus,
            paths: paths.clone(),
            models_dir,
            available_models: Mutex::new(available_models),
            is_rescanning: Arc::new(AtomicBool::new(false)),
        };

        manager.auto_select_model_if_needed()?;

        Ok(manager)
    }

    pub fn models_dir(&self) -> &Path {
        &self.models_dir
    }

    pub fn get_available_models(&self) -> Vec<ModelInfo> {
        let mut list: Vec<ModelInfo> = {
            let models = self.available_models.lock().unwrap();
            models.values().cloned().collect()
        };
        list.sort_by(|a, b| {
            (!a.is_recommended)
                .cmp(&(!b.is_recommended))
                .then(b.accuracy_score.total_cmp(&a.accuracy_score))
                .then(b.speed_score.total_cmp(&a.speed_score))
                .then_with(|| a.name.cmp(&b.name))
        });
        list
    }

    pub fn get_model_info(&self, model_id: &str) -> Option<ModelInfo> {
        let models = self.available_models.lock().unwrap();
        models.get(model_id).cloned()
    }

    pub fn get_model_path(&self, model_id: &str) -> Result<PathBuf> {
        let model_info = self
            .get_model_info(model_id)
            .ok_or_else(|| anyhow::anyhow!("Model not found: {}", model_id))?;

        let primary_path = self.models_dir.join(&model_info.filename);
        if primary_path.exists() {
            return Ok(primary_path);
        }

        let resource_path = self
            .paths
            .resource_dir
            .join("models")
            .join(&model_info.filename);
        if resource_path.exists() {
            return Ok(resource_path);
        }

        Err(anyhow::anyhow!(
            "Model file not found on disk: {:?} or {:?}",
            primary_path,
            resource_path
        ))
    }

    pub fn rescan_local_models(&self) -> Result<()> {
        if self.is_rescanning.swap(true, Ordering::SeqCst) {
            debug!("Model rescan already in progress; skipping");
            return Ok(());
        }

        let mut discovered = HashMap::new();
        if let Err(e) = Self::discover_models_in(&self.models_dir, &mut discovered) {
            warn!("Rescan: failed to discover models: {}", e);
        }

        let resource_models = self.paths.resource_dir.join("models");
        if resource_models.exists() && resource_models != self.models_dir {
            let _ = Self::discover_models_in(&resource_models, &mut discovered);
        }

        {
            let mut live = self.available_models.lock().unwrap();
            *live = discovered;
        }

        self.is_rescanning.store(false, Ordering::SeqCst);
        self.auto_select_model_if_needed()?;
        self.bus.send(AppEvent::ModelsUpdated);
        Ok(())
    }

    pub fn delete_model(&self, model_id: &str) -> Result<()> {
        let model_info = self
            .get_model_info(model_id)
            .ok_or_else(|| anyhow::anyhow!("Model not found: {}", model_id))?;

        let path = self.models_dir.join(&model_info.filename);
        if path.exists() {
            if model_info.is_directory {
                fs::remove_dir_all(&path)?;
            } else {
                fs::remove_file(&path)?;
            }
            info!("Deleted model from disk: {:?}", path);
        }

        {
            let mut models = self.available_models.lock().unwrap();
            models.remove(model_id);
        }

        self.bus.send(AppEvent::ModelDeleted(model_id.to_string()));
        self.bus.send(AppEvent::ModelsUpdated);
        Ok(())
    }

    pub fn auto_select_model_if_needed(&self) -> Result<()> {
        let settings_path = self.paths.settings_store_path();
        let mut settings = read_settings_from(&settings_path);

        let models = self.available_models.lock().unwrap();
        let current_valid =
            !settings.selected_model.is_empty() && models.contains_key(&settings.selected_model);

        if !current_valid {
            if let Some(first_model) = models.values().next() {
                info!(
                    "Auto-selecting available local model '{}' ({})",
                    first_model.name, first_model.id
                );
                settings.selected_model = first_model.id.clone();
                write_settings_to(&settings_path, &settings);
            }
        }
        Ok(())
    }

    pub fn set_runtime_capabilities(
        &self,
        model_id: &str,
        supports_streaming: bool,
        supports_translation: bool,
        supports_language_detection: bool,
        languages: Vec<String>,
    ) {
        let mut models = self.available_models.lock().unwrap();
        if let Some(info) = models.get_mut(model_id) {
            let canonical = canonicalize_supported_languages(languages);
            info.supports_language_selection = canonical.len() > 1;
            info.supported_languages = canonical;
            info.supports_streaming = supports_streaming;
            info.supports_translation = supports_translation;
            info.supports_language_detection = supports_language_detection;
        }
    }

    /// Discover models inside a specific directory.
    pub fn discover_models_in(
        dir: &Path,
        available_models: &mut HashMap<String, ModelInfo>,
    ) -> Result<()> {
        if !dir.exists() || !dir.is_dir() {
            return Ok(());
        }

        let entries = fs::read_dir(dir)?;
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };

            let lower_name = file_name.to_lowercase();
            if file_name.starts_with('.')
                || file_name.ends_with(".partial")
                || lower_name.contains("vad")
                || lower_name.contains("sentence-transformers")
            {
                continue;
            }

            if path.is_file() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    match ext.to_lowercase().as_str() {
                        "gguf" => {
                            if let Some(info) =
                                Self::probe_gguf_file(&path, &file_name, &file_name, false)
                            {
                                available_models.insert(info.id.clone(), info);
                            }
                        }
                        "bin" => {
                            let info =
                                Self::create_bin_model_info(&path, &file_name, &file_name, false);
                            available_models.insert(info.id.clone(), info);
                        }
                        "onnx" => {
                            let info =
                                Self::create_onnx_model_info(&path, &file_name, &file_name, false);
                            available_models.insert(info.id.clone(), info);
                        }
                        _ => {}
                    }
                }
            } else if path.is_dir() {
                // Check if directory contains a recognizable ONNX model
                if path.join("model.onnx").exists()
                    || path.join("encoder-model.onnx").exists()
                    || path.join("config.json").exists()
                {
                    let info = Self::create_dir_model_info(&path, &file_name);
                    available_models.insert(info.id.clone(), info);
                } else {
                    // Check for nested .gguf or .bin files (e.g. HuggingFace snapshots or subfolders)
                    let nested_ggufs = find_files_with_extension(&path, "gguf", 4);
                    if !nested_ggufs.is_empty() {
                        for gguf_path in nested_ggufs {
                            if let Ok(rel_path) = gguf_path.strip_prefix(dir) {
                                let rel_str = rel_path.to_string_lossy().to_string();
                                let id = file_name.clone();
                                if let Some(info) =
                                    Self::probe_gguf_file(&gguf_path, &id, &rel_str, true)
                                {
                                    available_models.insert(info.id.clone(), info);
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn probe_gguf_file(
        path: &Path,
        id: &str,
        relative_filename: &str,
        is_directory: bool,
    ) -> Option<ModelInfo> {
        let metadata = fs::metadata(path).ok()?;
        let size_mb = metadata.len() / (1024 * 1024);

        let probe = GgufHeaderProber.probe_file(path);
        let clean_id = id.strip_suffix(".gguf").unwrap_or(id).to_string();

        let raw_name = probe.display_name.as_deref().unwrap_or(clean_id.as_str());

        let name = humanize_model_name(raw_name);
        let languages = canonicalize_supported_languages(probe.languages.unwrap_or_default());
        let arch = probe.architecture.unwrap_or_else(|| "whisper".to_string());
        let is_whisper = arch == "whisper";

        Some(ModelInfo {
            id: clean_id,
            name,
            description: format!("Local {} model ({} MB)", arch.to_uppercase(), size_mb),
            filename: relative_filename.to_string(),
            source: ModelSource::Local,
            size_mb,
            is_downloaded: true,
            is_downloading: false,
            partial_size: 0,
            is_directory,
            engine_type: EngineType::TranscribeCpp,
            accuracy_score: 0.85,
            speed_score: 0.85,
            supports_translation: probe.supports_translation.unwrap_or(is_whisper),
            is_recommended: false,
            supports_language_selection: languages.len() > 1,
            supported_languages: languages,
            is_custom: true,
            supports_streaming: probe.supports_streaming.unwrap_or(true),
            supports_language_detection: probe.supports_language_detect.unwrap_or(true),
        })
    }

    fn create_bin_model_info(
        path: &Path,
        id: &str,
        relative_filename: &str,
        is_directory: bool,
    ) -> ModelInfo {
        let size_mb = fs::metadata(path)
            .map(|m| m.len() / (1024 * 1024))
            .unwrap_or(0);
        let clean_id = id.strip_suffix(".bin").unwrap_or(id).to_string();
        let name = humanize_model_name(&clean_id);

        ModelInfo {
            id: clean_id,
            name,
            description: format!("GGML Whisper model ({} MB)", size_mb),
            filename: relative_filename.to_string(),
            source: ModelSource::Local,
            size_mb,
            is_downloaded: true,
            is_downloading: false,
            partial_size: 0,
            is_directory,
            engine_type: EngineType::TranscribeCpp,
            accuracy_score: 0.85,
            speed_score: 0.85,
            supports_translation: true,
            is_recommended: false,
            supported_languages: vec![],
            supports_language_selection: true,
            is_custom: true,
            supports_streaming: true,
            supports_language_detection: true,
        }
    }

    fn create_onnx_model_info(
        path: &Path,
        id: &str,
        relative_filename: &str,
        is_directory: bool,
    ) -> ModelInfo {
        let size_mb = fs::metadata(path)
            .map(|m| m.len() / (1024 * 1024))
            .unwrap_or(0);
        let clean_id = id.strip_suffix(".onnx").unwrap_or(id).to_string();
        let name = humanize_model_name(&clean_id);

        ModelInfo {
            id: clean_id,
            name,
            description: format!("ONNX model ({} MB)", size_mb),
            filename: relative_filename.to_string(),
            source: ModelSource::Local,
            size_mb,
            is_downloaded: true,
            is_downloading: false,
            partial_size: 0,
            is_directory,
            engine_type: EngineType::Parakeet,
            accuracy_score: 0.85,
            speed_score: 0.85,
            supports_translation: false,
            is_recommended: false,
            supported_languages: vec!["en".to_string()],
            supports_language_selection: false,
            is_custom: true,
            supports_streaming: false,
            supports_language_detection: false,
        }
    }

    fn create_dir_model_info(path: &Path, dirname: &str) -> ModelInfo {
        let size_mb = fs::read_dir(path)
            .map(|entries| {
                entries
                    .flatten()
                    .filter_map(|e| e.metadata().ok())
                    .map(|m| m.len())
                    .sum::<u64>()
                    / (1024 * 1024)
            })
            .unwrap_or(0);

        let name = humanize_model_name(dirname);

        ModelInfo {
            id: dirname.to_string(),
            name,
            description: format!("Local model directory ({} MB)", size_mb),
            filename: dirname.to_string(),
            source: ModelSource::Local,
            size_mb,
            is_downloaded: true,
            is_downloading: false,
            partial_size: 0,
            is_directory: true,
            engine_type: EngineType::SenseVoice,
            accuracy_score: 0.85,
            speed_score: 0.85,
            supports_translation: false,
            is_recommended: false,
            supported_languages: vec!["auto".to_string()],
            supports_language_selection: false,
            is_custom: true,
            supports_streaming: false,
            supports_language_detection: true,
        }
    }
}

fn find_files_with_extension(dir: &Path, ext: &str, max_depth: usize) -> Vec<PathBuf> {
    let mut results = Vec::new();
    fn walk(
        current: &Path,
        ext: &str,
        current_depth: usize,
        max_depth: usize,
        results: &mut Vec<PathBuf>,
    ) {
        if current_depth > max_depth {
            return;
        }
        if let Ok(entries) = fs::read_dir(current) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file() {
                    if p.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
                    {
                        results.push(p);
                    }
                } else if p.is_dir() {
                    walk(&p, ext, current_depth + 1, max_depth, results);
                }
            }
        }
    }
    walk(dir, ext, 1, max_depth, &mut results);
    results
}

fn humanize_model_name(raw: &str) -> String {
    let cleaned = raw
        .replace(['-', '_'], " ")
        .replace(".gguf", "")
        .replace(".bin", "")
        .replace(".onnx", "");
    let words: Vec<String> = cleaned
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect();

    if words.is_empty() {
        raw.to_string()
    } else {
        words.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humanize_model_name_works() {
        assert_eq!(
            humanize_model_name("whisper-small-q4_k_m"),
            "Whisper Small Q4 K M"
        );
        assert_eq!(humanize_model_name("ggml-base.en.bin"), "Ggml Base.en");
    }

    #[test]
    fn discover_models_in_empty_dir() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut models = HashMap::new();
        ModelManager::discover_models_in(temp_dir.path(), &mut models).unwrap();
        assert!(models.is_empty());
    }

    #[test]
    fn discover_models_in_nested_dir() {
        let temp_dir = tempfile::tempdir().unwrap();
        let nested = temp_dir.path().join("my-model-folder/snapshots/rev1");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("model.gguf"), b"dummy").unwrap();

        let mut models = HashMap::new();
        ModelManager::discover_models_in(temp_dir.path(), &mut models).unwrap();
        assert_eq!(models.len(), 1);
        let info = models.get("my-model-folder").unwrap();
        assert_eq!(info.filename, "my-model-folder/snapshots/rev1/model.gguf");
        assert!(info.is_directory);
    }

    #[test]
    fn vad_models_are_ignored_in_discovery() {
        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(temp_dir.path().join("silero_vad_v4.onnx"), b"dummy").unwrap();
        fs::write(temp_dir.path().join("vad_model.bin"), b"dummy").unwrap();

        let mut models = HashMap::new();
        ModelManager::discover_models_in(temp_dir.path(), &mut models).unwrap();
        assert!(models.is_empty());
    }

    #[test]
    fn effective_language_falls_back_gracefully() {
        let langs = vec!["en".to_string(), "pt".to_string(), "es".to_string()];
        assert_eq!(effective_language("en-US", &langs, true), "en");
        assert_eq!(effective_language("pt-BR", &langs, true), "pt");
        assert_eq!(effective_language("auto", &langs, true), "auto");
        assert_eq!(effective_language("auto", &langs, false), "en");
    }
}
