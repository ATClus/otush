//! Settings schema: every persisted type plus `AppSettings` itself. (split from `settings.rs`; same keys, same behavior).

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
use std::fmt;

use super::defaults::*;

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

// Custom deserializer to handle both old numeric format (1-5) and new string format ("trace", "debug", etc.)
impl<'de> Deserialize<'de> for LogLevel {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct LogLevelVisitor;

        impl<'de> Visitor<'de> for LogLevelVisitor {
            type Value = LogLevel;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a string or integer representing log level")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<LogLevel, E> {
                match value.to_lowercase().as_str() {
                    "trace" => Ok(LogLevel::Trace),
                    "debug" => Ok(LogLevel::Debug),
                    "info" => Ok(LogLevel::Info),
                    "warn" => Ok(LogLevel::Warn),
                    "error" => Ok(LogLevel::Error),
                    _ => Err(E::unknown_variant(
                        value,
                        &["trace", "debug", "info", "warn", "error"],
                    )),
                }
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<LogLevel, E> {
                match value {
                    1 => Ok(LogLevel::Trace),
                    2 => Ok(LogLevel::Debug),
                    3 => Ok(LogLevel::Info),
                    4 => Ok(LogLevel::Warn),
                    5 => Ok(LogLevel::Error),
                    _ => Err(E::invalid_value(de::Unexpected::Unsigned(value), &"1-5")),
                }
            }
        }

        deserializer.deserialize_any(LogLevelVisitor)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ShortcutBinding {
    pub id: String,
    pub name: String,
    pub description: String,
    pub default_binding: String,
    pub current_binding: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    #[default]
    None,
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
}

impl ReasoningEffort {
    pub fn from_index(idx: u32) -> Self {
        match idx {
            1 => ReasoningEffort::Minimal,
            2 => ReasoningEffort::Low,
            3 => ReasoningEffort::Medium,
            4 => ReasoningEffort::High,
            5 => ReasoningEffort::XHigh,
            _ => ReasoningEffort::None,
        }
    }

    pub fn to_index(self) -> u32 {
        match self {
            ReasoningEffort::None => 0,
            ReasoningEffort::Minimal => 1,
            ReasoningEffort::Low => 2,
            ReasoningEffort::Medium => 3,
            ReasoningEffort::High => 4,
            ReasoningEffort::XHigh => 5,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderReasoningConfig {
    #[serde(default)]
    pub effort: ReasoningEffort,
    #[serde(default)]
    pub budget_tokens: Option<u32>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LLMPrompt {
    pub id: String,
    pub name: String,
    pub prompt: String,
    #[serde(default)]
    pub preferred_provider_id: Option<String>,
}

/// Names of the read-only web tools an agent may call. The single source of
/// truth; [`crate::agents::tools`] builds the LLM-facing schemas from it.
pub const AGENT_TOOL_NAMES: &[&str] = &[
    "tavily_search",
    "tavily_extract",
    "firecrawl_search",
    "firecrawl_scrape",
];

/// Default per-tool call budget inside one user turn (each tool may be
/// called this many times before the runner refuses with a retry hint).
pub(crate) fn default_agent_tool_budget() -> u32 {
    3
}

/// An AI chat agent: a named persona bound to an LLM provider, with an
/// optional model override, a tool allow-list, and RAG preferences.
///
/// `enabled_tools` empty means "all tools" (the Agents page always writes an
/// explicit list; empty only arises from hand-edited stores). Model
/// resolution order is `model_override` → `post_process_models[provider_id]`
/// → error; see `commands::agents::resolve_agent`.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AgentConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub provider_id: String,
    #[serde(default)]
    pub model_override: Option<String>,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub enabled_tools: Vec<String>,
    #[serde(default = "default_agent_max_steps")]
    pub max_tool_steps: u32,
    /// Max calls per tool inside one turn (1..=10), guarding against loops
    /// where the model repeats the same failing query.
    #[serde(default = "default_agent_tool_budget")]
    pub tool_budget_per_tool: u32,
    #[serde(default = "default_true")]
    pub rag_enabled: bool,
    #[serde(default = "default_agent_top_k")]
    pub rag_top_k: u32,
}

impl AgentConfig {
    /// Tool allow-list with the empty-means-all convention expanded.
    pub fn effective_tools(&self) -> Vec<String> {
        if self.enabled_tools.is_empty() {
            return AGENT_TOOL_NAMES.iter().map(|s| s.to_string()).collect();
        }
        let mut tools: Vec<String> = self
            .enabled_tools
            .iter()
            .filter(|t| AGENT_TOOL_NAMES.contains(&t.as_str()))
            .cloned()
            .collect();
        tools.sort();
        tools.dedup();
        tools
    }

    /// Agentic loop cap, clamped to 1..=12.
    pub fn effective_max_steps(&self) -> u32 {
        self.max_tool_steps.clamp(1, 12)
    }

    /// Per-tool call budget per turn, clamped to 1..=10.
    pub fn effective_tool_budget(&self) -> u32 {
        self.tool_budget_per_tool.clamp(1, 10)
    }

    /// RAG retrieval depth, clamped to 1..=10.
    pub fn effective_top_k(&self) -> u32 {
        self.rag_top_k.clamp(1, 10)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PostProcessProvider {
    pub id: String,
    pub label: String,
    pub base_url: String,
    #[serde(default)]
    pub allow_base_url_edit: bool,
    #[serde(default)]
    pub models_endpoint: Option<String>,
    #[serde(default)]
    pub supports_structured_output: bool,
    #[serde(default)]
    pub reasoning: ProviderReasoningConfig,
    #[serde(default = "default_provider_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub custom_headers: HashMap<String, String>,
    #[serde(default = "default_provider_timeout")]
    pub timeout_seconds: u32,
}

fn default_provider_enabled() -> bool {
    true
}

fn default_provider_timeout() -> u32 {
    120
}

pub(crate) fn default_agent_max_steps() -> u32 {
    6
}

pub(crate) fn default_agent_top_k() -> u32 {
    4
}

pub(crate) fn default_agent_chat_retention_days() -> u32 {
    90
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct DeepgramConfig {
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default = "default_true")]
    pub smart_format: bool,
    #[serde(default = "default_true")]
    pub punctuate: bool,
    #[serde(default = "default_true")]
    pub numerals: bool,
    #[serde(default)]
    pub paragraphs: bool,
    #[serde(default)]
    pub diarize: bool,
    #[serde(default)]
    pub filler_words: bool,
    #[serde(default)]
    pub profanity_filter: bool,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub extra_query_params: HashMap<String, String>,
}

impl Default for DeepgramConfig {
    fn default() -> Self {
        Self {
            language: None,
            smart_format: true,
            punctuate: true,
            numerals: true,
            paragraphs: false,
            diarize: false,
            filler_words: false,
            profanity_filter: false,
            keywords: Vec::new(),
            extra_query_params: HashMap::new(),
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TranscriptionProvider {
    pub id: String,
    pub label: String,
    pub base_url: String,
    pub model: String,
    #[serde(default = "default_provider_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub allow_base_url_edit: bool,
    #[serde(default = "default_stt_provider_timeout")]
    pub timeout_seconds: u32,
    #[serde(default)]
    pub custom_headers: HashMap<String, String>,
    #[serde(default)]
    pub deepgram: Option<DeepgramConfig>,
}

fn default_stt_provider_timeout() -> u32 {
    15
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct WebProvider {
    pub id: String,
    pub label: String,
    pub base_url: String,
    #[serde(default = "default_provider_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub allow_base_url_edit: bool,
    #[serde(default = "default_web_provider_timeout")]
    pub timeout_seconds: u32,
    #[serde(default)]
    pub custom_headers: HashMap<String, String>,
}

fn default_web_provider_timeout() -> u32 {
    60
}

pub fn default_web_providers() -> Vec<WebProvider> {
    vec![
        WebProvider {
            id: "tavily".to_string(),
            label: "Tavily".to_string(),
            base_url: "https://api.tavily.com".to_string(),
            enabled: true,
            allow_base_url_edit: false,
            timeout_seconds: 60,
            custom_headers: HashMap::new(),
        },
        WebProvider {
            id: "firecrawl".to_string(),
            label: "Firecrawl".to_string(),
            base_url: "https://api.firecrawl.dev/v2".to_string(),
            enabled: true,
            allow_base_url_edit: true,
            timeout_seconds: 60,
            custom_headers: HashMap::new(),
        },
    ]
}

pub fn default_web_api_keys() -> SecretMap {
    let mut map = HashMap::new();
    for provider in default_web_providers() {
        map.insert(provider.id, String::new());
    }
    SecretMap(map)
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OverlayPosition {
    Top,
    // `none` is retired: overlay visibility is owned by `OverlayStyle` now. The
    // alias keeps legacy stores (`"overlay_position": "none"`) deserializing
    // instead of failing the whole load; the one-time overlay migration reads the
    // raw stored string to recover the old "hidden" intent as `OverlayStyle::None`.
    #[serde(alias = "none")]
    Bottom,
}

/// Which recording overlay to display. `Minimal` and `Live` share one base
/// (the pill); `Live` grows into the panel that shows live transcription text.
/// `None` hides the overlay entirely. Decoupled from whether the model runs in
/// streaming mode (that is driven purely by model capability).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OverlayStyle {
    None,
    Minimal,
    Live,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ModelUnloadTimeout {
    Never,
    Immediately,
    Min2,
    #[default]
    Min5,
    Min10,
    Min15,
    Hour1,
    Sec15, // Debug mode only
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PasteMethod {
    CtrlV,
    #[default]
    Direct,
    None,
    ShiftInsert,
    CtrlShiftV,
    ExternalScript,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardHandling {
    #[default]
    DontModify,
    CopyToClipboard,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutoSubmitKey {
    #[default]
    Enter,
    CtrlEnter,
    CmdEnter,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AudioCaptureSource {
    #[default]
    MicrophoneOnly,
    SystemAudioOnly,
    Mixed,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecordingRetentionPeriod {
    Never,
    PreserveLimit,
    Days3,
    Weeks2,
    Months3,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum KeyboardImplementation {
    #[default]
    Portal,
    Evdev,
}

impl ModelUnloadTimeout {
    pub fn to_minutes(self) -> Option<u64> {
        match self {
            ModelUnloadTimeout::Never => None,
            ModelUnloadTimeout::Immediately => Some(0), // Special case for immediate unloading
            ModelUnloadTimeout::Min2 => Some(2),
            ModelUnloadTimeout::Min5 => Some(5),
            ModelUnloadTimeout::Min10 => Some(10),
            ModelUnloadTimeout::Min15 => Some(15),
            ModelUnloadTimeout::Hour1 => Some(60),
            ModelUnloadTimeout::Sec15 => Some(0), // Special case for debug - handled separately
        }
    }

    pub fn to_seconds(self) -> Option<u64> {
        match self {
            ModelUnloadTimeout::Never => None,
            ModelUnloadTimeout::Immediately => Some(0), // Special case for immediate unloading
            ModelUnloadTimeout::Sec15 => Some(15),
            _ => self.to_minutes().map(|m| m * 60),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SoundTheme {
    Marimba,
    Pop,
    Custom,
}

impl SoundTheme {
    fn as_str(&self) -> &'static str {
        match self {
            SoundTheme::Marimba => "marimba",
            SoundTheme::Pop => "pop",
            SoundTheme::Custom => "custom",
        }
    }

    pub fn to_start_path(self) -> String {
        format!("{}_start.wav", self.as_str())
    }

    pub fn to_stop_path(self) -> String {
        format!("{}_stop.wav", self.as_str())
    }
}

/// UI appearance mode. `System` follows the OS `prefers-color-scheme`; `Light`
/// and `Dark` force one of the two palettes Otush already ships.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    System,
    Light,
    Dark,
}

/// Tray icon style (see `AppSettings::tray_theme`).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TrayTheme {
    /// Light glyphs for dark top bars (GNOME Shell default).
    #[default]
    Dark,
    /// Dark glyphs for light top bars.
    Light,
    /// Full-color app icons.
    Colored,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TypingTool {
    #[default]
    Auto,
    Wtype,
    Kwtype,
    Dotool,
    Ydotool,
    Xdotool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TranscribeAcceleratorSetting {
    #[default]
    Auto,
    Cpu,
    Gpu,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum OrtAcceleratorSetting {
    #[default]
    Auto,
    Cpu,
    Cuda,
    #[serde(rename = "directml")]
    DirectMl,
    Rocm,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum VadBackend {
    #[default]
    Silero,
    Earshot,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SecretMap(pub(crate) HashMap<String, String>);

impl fmt::Debug for SecretMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let redacted: HashMap<&String, &str> = self
            .0
            .iter()
            .map(|(k, v)| (k, if v.is_empty() { "" } else { "[REDACTED]" }))
            .collect();
        redacted.fmt(f)
    }
}

impl std::ops::Deref for SecretMap {
    type Target = HashMap<String, String>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for SecretMap {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/* still handy for composing the initial JSON in the store ------------- */

/// The container-level `serde(default)` (backed by the `Default` impl below)
/// guarantees every field — including ones added in the future — falls back to
/// its `get_default_settings()` value when missing from a stored settings
/// object, so a partial store can never fail the whole load (#1619).
/// Field-level defaults below take precedence where present.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(default)]
pub struct AppSettings {
    /// Internal settings schema marker for one-time migrations. Fresh installs
    /// start at the current version; existing stores missing this key are
    /// treated as version 0 and migrated forward.
    #[serde(default = "default_settings_schema_version")]
    pub settings_schema_version: u32,
    /// Defaults to empty on partial stores; the load path merges in the
    /// default bindings for any missing keys before the settings are used.
    #[serde(default)]
    pub bindings: HashMap<String, ShortcutBinding>,
    #[serde(default = "default_push_to_talk")]
    pub push_to_talk: bool,
    #[serde(default)]
    pub audio_feedback: bool,
    #[serde(default = "default_audio_feedback_volume")]
    pub audio_feedback_volume: f32,
    #[serde(default = "default_sound_theme")]
    pub sound_theme: SoundTheme,
    #[serde(default = "default_start_hidden")]
    pub start_hidden: bool,
    #[serde(default = "default_autostart_enabled")]
    pub autostart_enabled: bool,
    #[serde(default = "default_update_checks_enabled")]
    pub update_checks_enabled: bool,
    #[serde(default = "default_show_whats_new_on_update")]
    pub show_whats_new_on_update: bool,
    /// The app version whose What's New the user has already seen. Fresh installs
    /// default to the current version (nothing is "new" to them). Existing users
    /// upgrading from before this key existed are blanked by the migration so they
    /// see the current release's notes — see `apply_settings_migrations`.
    #[serde(default = "default_whats_new_last_seen_version")]
    pub whats_new_last_seen_version: String,
    #[serde(default = "default_model")]
    pub selected_model: String,
    #[serde(default)]
    pub onboarding_completed: bool,
    #[serde(default = "default_always_on_microphone")]
    pub always_on_microphone: bool,
    #[serde(default)]
    pub selected_microphone: Option<String>,
    /// Which input channel to use on the selected microphone device.
    /// None means "average all channels" (original behavior).
    #[serde(default)]
    pub selected_channel: Option<u16>,
    #[serde(default)]
    pub clamshell_microphone: Option<String>,
    #[serde(default)]
    pub selected_output_device: Option<String>,
    #[serde(default = "default_audio_capture_source")]
    pub audio_capture_source: AudioCaptureSource,
    #[serde(default)]
    pub selected_system_audio_device: Option<String>,
    #[serde(default = "default_translate_to_english")]
    pub translate_to_english: bool,
    #[serde(default = "default_selected_language")]
    pub selected_language: String,
    #[serde(default = "default_overlay_position")]
    pub overlay_position: OverlayPosition,
    #[serde(default = "default_debug_mode")]
    pub debug_mode: bool,
    #[serde(default = "default_log_level")]
    pub log_level: LogLevel,
    #[serde(default)]
    pub custom_words: Vec<String>,
    #[serde(default)]
    pub model_unload_timeout: ModelUnloadTimeout,
    #[serde(default = "default_word_correction_threshold")]
    pub word_correction_threshold: f64,
    #[serde(default = "default_history_limit")]
    pub history_limit: usize,
    #[serde(default = "default_recording_retention_period")]
    pub recording_retention_period: RecordingRetentionPeriod,
    #[serde(default)]
    pub paste_method: PasteMethod,
    #[serde(default)]
    pub clipboard_handling: ClipboardHandling,
    #[serde(default = "default_auto_submit")]
    pub auto_submit: bool,
    #[serde(default)]
    pub auto_submit_key: AutoSubmitKey,
    #[serde(default = "default_local_transcription_enabled")]
    pub local_transcription_enabled: bool,
    #[serde(default = "default_transcription_providers")]
    pub transcription_providers: Vec<TranscriptionProvider>,
    #[serde(default = "default_transcription_api_keys")]
    pub transcription_api_keys: SecretMap,
    #[serde(default = "default_transcription_models")]
    pub transcription_models: HashMap<String, String>,
    #[serde(default = "default_post_process_enabled")]
    pub post_process_enabled: bool,
    #[serde(default = "default_post_process_provider_id")]
    pub post_process_provider_id: String,
    #[serde(default = "default_post_process_providers")]
    pub post_process_providers: Vec<PostProcessProvider>,
    #[serde(default = "default_post_process_api_keys")]
    pub post_process_api_keys: SecretMap,
    #[serde(default = "default_post_process_models")]
    pub post_process_models: HashMap<String, String>,
    #[serde(default = "default_web_providers")]
    pub web_providers: Vec<WebProvider>,
    #[serde(default = "default_web_api_keys")]
    pub web_api_keys: SecretMap,
    #[serde(default = "default_post_process_prompts")]
    pub post_process_prompts: Vec<LLMPrompt>,
    #[serde(default)]
    pub post_process_selected_prompt_id: Option<String>,
    #[serde(default = "default_agents")]
    pub agents: Vec<AgentConfig>,
    #[serde(default)]
    pub selected_agent_id: Option<String>,
    #[serde(default = "default_agent_chat_retention_days")]
    pub agent_chat_retention_days: u32,
    #[serde(default)]
    pub mute_while_recording: bool,
    #[serde(default)]
    pub append_trailing_space: bool,
    #[serde(default = "default_app_language")]
    pub app_language: String,
    #[serde(default = "default_theme")]
    pub theme: Theme,
    /// Tray icon style. GNOME Shell's top bar is dark by default, so `Dark`
    /// (light glyphs) is the default; `Light` suits light bars, `Colored`
    /// uses the full-color app icons.
    #[serde(default)]
    pub tray_theme: TrayTheme,
    #[serde(default)]
    pub experimental_enabled: bool,
    #[serde(default)]
    pub lazy_stream_close: bool,
    #[serde(default)]
    pub keyboard_implementation: KeyboardImplementation,
    #[serde(default = "default_show_tray_icon")]
    pub show_tray_icon: bool,
    #[serde(default = "default_paste_delay_ms")]
    pub paste_delay_ms: u64,
    #[serde(default = "default_paste_delay_after_ms")]
    pub paste_delay_after_ms: u64,
    #[serde(default = "default_typing_tool")]
    pub typing_tool: TypingTool,
    #[serde(default)]
    pub external_script_path: Option<String>,
    #[serde(default = "default_filler_word_removal_enabled")]
    pub filler_word_removal_enabled: bool,
    #[serde(default)]
    pub custom_filler_words: Option<Vec<String>>,
    #[serde(default)]
    pub transcribe_accelerator: TranscribeAcceleratorSetting,
    #[serde(default)]
    pub ort_accelerator: OrtAcceleratorSetting,
    /// Stable transcribe.cpp device selector. This is derived from the backend's
    /// `device_id` when available (or its name for backends such as Metal),
    /// never from the process-local device registry index.
    #[serde(
        default = "default_transcribe_gpu_device",
        deserialize_with = "deserialize_transcribe_gpu_device"
    )]
    pub transcribe_gpu_device: Option<String>,
    #[serde(default)]
    pub extra_recording_buffer_ms: u64,
    #[serde(default = "default_vad_enabled")]
    pub vad_enabled: bool,
    /// Experimental detector implementation. Silero remains the stable default.
    #[serde(default)]
    pub vad_backend: VadBackend,
    /// Which recording overlay to show: None / Minimal / Live. Streaming mode is
    /// not gated on this — that follows model capability. Migrated from the old
    /// `overlay_position` (position `none` → style `None`).
    #[serde(default = "default_overlay_style")]
    pub overlay_style: OverlayStyle,
    /// Software input gain multiplier (e.g. 1.0 = standard, 2.0 = +6dB, 4.0 = +12dB).
    #[serde(default = "default_audio_input_gain")]
    pub audio_input_gain: f32,
    /// Dynamic RMS & Peak voice normalization with soft-knee limiter.
    #[serde(default = "default_audio_normalization_enabled")]
    pub audio_normalization_enabled: bool,
    /// 2nd-order Butterworth high-pass filter at 80 Hz (removes DC, rumble, 60Hz hum).
    #[serde(default = "default_audio_high_pass_filter_enabled")]
    pub audio_high_pass_filter_enabled: bool,
    /// Adaptive noise gate (suppresses background hiss and fan hum during speech pauses).
    #[serde(default = "default_audio_noise_reduction_enabled")]
    pub audio_noise_reduction_enabled: bool,
    /// Noise gate threshold in dB (-60.0 to -25.0 dB, default -45.0 dB).
    #[serde(default = "default_audio_noise_gate_threshold_db")]
    pub audio_noise_gate_threshold_db: f32,
}

impl Default for AppSettings {
    fn default() -> Self {
        get_default_settings()
    }
}

impl AppSettings {
    pub fn active_post_process_provider(&self) -> Option<&PostProcessProvider> {
        self.post_process_providers
            .iter()
            .find(|provider| provider.id == self.post_process_provider_id)
    }

    pub fn post_process_provider(&self, provider_id: &str) -> Option<&PostProcessProvider> {
        self.post_process_providers
            .iter()
            .find(|provider| provider.id == provider_id)
    }

    pub fn post_process_provider_mut(
        &mut self,
        provider_id: &str,
    ) -> Option<&mut PostProcessProvider> {
        self.post_process_providers
            .iter_mut()
            .find(|provider| provider.id == provider_id)
    }

    pub fn transcription_provider(&self, provider_id: &str) -> Option<&TranscriptionProvider> {
        self.transcription_providers
            .iter()
            .find(|provider| provider.id == provider_id)
    }

    pub fn transcription_provider_mut(
        &mut self,
        provider_id: &str,
    ) -> Option<&mut TranscriptionProvider> {
        self.transcription_providers
            .iter_mut()
            .find(|provider| provider.id == provider_id)
    }

    pub fn first_enabled_transcription_provider(&self) -> Option<&TranscriptionProvider> {
        self.transcription_providers
            .iter()
            .find(|provider| provider.enabled)
    }

    pub fn agent(&self, agent_id: &str) -> Option<&AgentConfig> {
        self.agents.iter().find(|agent| agent.id == agent_id)
    }

    pub fn selected_agent(&self) -> Option<&AgentConfig> {
        self.selected_agent_id
            .as_deref()
            .and_then(|id| self.agent(id))
            .filter(|agent| agent.enabled)
            .or_else(|| self.agents.iter().find(|agent| agent.enabled))
    }

    pub fn is_deepgram_streaming_active(&self) -> bool {
        if self.local_transcription_enabled {
            return false;
        }
        if let Some(first) = self.first_enabled_transcription_provider() {
            if first.id == "deepgram" {
                if let Some(key) = self.transcription_api_keys.get("deepgram") {
                    return !key.trim().is_empty();
                }
            }
        }
        false
    }
}
