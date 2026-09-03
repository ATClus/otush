//! Host abstraction for the backend, plus the backend → UI event bus.
//!
//! The Rust core (managers, audio pipeline, transcription, downloads, history)
//! never talks to a GUI toolkit. It talks to [`AppContext`] — paths, settings,
//! shared managers and the [`EventBus`] — and the GNOME (GTK4/libadwaita) shell
//! subscribes to [`AppEvent`]s to update widgets.

use crate::managers::audio::AudioRecordingManager;
use crate::managers::history::{HistoryManager, HistoryUpdatePayload};
use crate::managers::model::ModelManager;
use crate::managers::transcription::{
    ModelStateEvent, StreamPhaseEvent, StreamTextEvent, TranscriptionManager,
};
use crate::settings::{AppSettings, Theme};
use crate::TranscriptionCoordinator;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// App-wide filesystem locations, portable-aware.
#[derive(Clone, Debug)]
pub struct AppPaths {
    /// User data root (settings, models, history DB, recordings, logs).
    pub data_dir: PathBuf,
    /// Bundled resources (sounds, tray icons, default settings, VAD model).
    pub resource_dir: PathBuf,
    /// Log directory.
    pub log_dir: PathBuf,
}

impl AppPaths {
    /// Resolve app dirs. `data_dir` honors portable mode; otherwise it is
    /// `$XDG_DATA_HOME/com.clusterat.otush`.
    pub fn resolve() -> Self {
        let data_dir = crate::portable::data_root();
        let log_dir = crate::portable::log_root(&data_dir);
        let resource_dir = crate::resources::resource_dir();
        Self {
            data_dir,
            resource_dir,
            log_dir,
        }
    }

    /// Directory where downloaded models live (and are extracted).
    pub fn models_dir(&self) -> PathBuf {
        self.data_dir.join("models")
    }

    /// Directory where recordings are archived.
    pub fn recordings_dir(&self) -> PathBuf {
        self.data_dir.join("recordings")
    }

    /// Absolute path of the JSON settings store (`settings_store.json`).
    pub fn settings_store_path(&self) -> PathBuf {
        self.data_dir.join(crate::settings::SETTINGS_STORE_PATH)
    }

    /// Ensure the data + log directories exist.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(&self.log_dir)?;
        Ok(())
    }
}

/// Recording error surfaced to the UI (toast/banner).
#[derive(Clone, Debug, serde::Serialize)]
pub struct RecordingErrorEvent {
    pub error_type: String,
    pub detail: Option<String>,
}

/// Model download progress event surfaced to the UI.
#[derive(Clone, Debug)]
pub struct ModelDownloadProgressEvent {
    pub url: String,
    pub filename: String,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub percentage: f64,
    pub speed_mb_s: f64,
}

/// Model download completion event surfaced to the UI.
#[derive(Clone, Debug)]
pub struct ModelDownloadFinishedEvent {
    pub filename: String,
    pub success: bool,
    pub error: Option<String>,
}

/// One backend → UI event. The GTK shell subscribes via [`EventBus::subscribe`].
#[derive(Clone, Debug)]
pub enum AppEvent {
    // --- settings ---
    /// A setting changed (name + new value as JSON).
    SettingsChanged {
        setting: String,
        value: serde_json::Value,
    },
    ThemeChanged(Theme),
    CheckForUpdates,
    // --- model lifecycle ---
    ModelStateChanged(ModelStateEvent),
    ModelsUpdated,
    ModelDeleted(String),
    ModelDownloadProgress(ModelDownloadProgressEvent),
    ModelDownloadFinished(ModelDownloadFinishedEvent),
    // --- transcription / streaming ---
    StreamText(StreamTextEvent),
    StreamPhase(StreamPhaseEvent),
    TranscriptionError(String),
    // --- audio / recording ---
    RecordingError(RecordingErrorEvent),
    /// Mic level for the overlay level meter (throttled).
    MicLevel(f32),
    ShowOverlay,
    HideOverlay,
    // --- clipboard / paste ---
    PasteError,
    TextCopiedToClipboard {
        message: String,
    },
    // --- history ---
    HistoryUpdated(HistoryUpdatePayload),
    // --- debug ---
    /// A log line, forwarded only while debug mode is on (live log viewer).
    LogRecord(String),
}

/// A single event-bus subscriber callback.
pub type EventBusSubscriber = Arc<dyn Fn(AppEvent) + Send + Sync>;

/// Broadcast bus from backend threads to one-or-more UI subscribers.
///
/// Subscribers register a callback (typically marshaling onto the GTK main
/// loop via `glib::MainContext::default().invoke(...)`); `send` fans out to
/// all live subscribers from any thread.
#[derive(Clone, Default)]
pub struct EventBus {
    subscribers: Arc<Mutex<Arc<[EventBusSubscriber]>>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a subscriber callback. It may be invoked from any thread, so
    /// the callback should marshal to the UI thread (e.g. with
    /// `glib::MainContext::default().invoke(...)`).
    pub fn subscribe(&self, callback: impl Fn(AppEvent) + Send + Sync + 'static) {
        let mut guard = self.subscribers.lock().unwrap();
        let mut list: Vec<EventBusSubscriber> = (**guard).to_vec();
        list.push(Arc::new(callback));
        *guard = list.into();
    }

    /// Fan out to all subscribers without allocating a vector on send.
    pub fn send(&self, event: AppEvent) {
        let subscribers = Arc::clone(&*self.subscribers.lock().unwrap());
        if subscribers.is_empty() {
            return;
        }

        let count = subscribers.len();
        for (i, callback) in subscribers.iter().enumerate() {
            if i + 1 == count {
                callback(event);
                break;
            } else {
                callback(event.clone());
            }
        }
    }
}

/// Everything the backend needs: paths, settings, managers and the event bus.
/// Cheap to clone (`Arc` fields + cloneable bus); shared across threads.
#[derive(Clone)]
pub struct AppContext {
    pub paths: AppPaths,
    pub bus: EventBus,
    pub model: Arc<ModelManager>,
    pub transcription: Arc<TranscriptionManager>,
    pub audio: Arc<AudioRecordingManager>,
    pub history: Arc<HistoryManager>,
    pub coordinator: Arc<TranscriptionCoordinator>,
}

impl AppContext {
    pub fn settings(&self) -> AppSettings {
        crate::settings::get_settings(self)
    }

    pub fn write_settings(&self, settings: &AppSettings) {
        crate::settings::write_settings(self, settings.clone());
    }

    /// Emit the standard `settings-changed` event the UI listens for.
    pub fn notify_setting_changed(&self, setting: &str, value: serde_json::Value) {
        self.bus.send(AppEvent::SettingsChanged {
            setting: setting.to_string(),
            value,
        });
    }

    pub fn models_dir(&self) -> PathBuf {
        self.paths.models_dir()
    }

    pub fn resource_dir(&self) -> &Path {
        &self.paths.resource_dir
    }
}
