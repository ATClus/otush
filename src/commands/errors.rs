//! Typed errors for the UI-facing command layer.
//!
//! The command functions in [`crate::commands`] previously returned
//! `Result<_, String>`, which erased the failure cause at the type level:
//! every `anyhow::Error`, I/O error, and domain validation collapsed into an
//! opaque string, so callers could only pattern-match on message text. This
//! module defines one [`CommandError`] enum (via `thiserror`) that preserves
//! each failure category as a distinct variant while keeping the existing
//! user-facing messages byte-identical (`Display` output is unchanged, so
//! toasts, logs, and headless CLI output render exactly as before).
//!
//! Conversion rules:
//! - `anyhow::Error` (managers, audio toolkit) → [`CommandError::Backend`]
//!   via `#[from]`, keeping the full error chain for logs while `Display`
//!   shows the same message the old `e.to_string()` produced.
//! - Tokio `JoinError` (spawned blocking tasks) → [`CommandError::TaskJoin`],
//!   which formats as `"audio task join failed: {0}"` just like before
//!   (transcription tasks use the `"Transcription task panicked: {0}"` and
//!   `"Decoding task panicked: {0}"` spellings via explicit constructors).
//! - Domain validations (missing entry, empty recording, busy loader) →
//!   dedicated variants with the exact legacy message.
//!
//! Callers keep working unchanged: `CommandError` implements
//! [`std::fmt::Display`], so `report_error(ctx, err)` and `format!("{e}")`
//! behave as with the old `String` errors.

use std::path::PathBuf;

/// Failure of a UI-facing command.
///
/// Every variant's `Display` text matches the legacy `String` message the
/// command layer produced before, so toasts and logs are unchanged.
#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    /// A backend operation failed (managers, audio toolkit, STT clients).
    /// Carries the original `anyhow::Error` chain; displays its message.
    #[error(transparent)]
    Backend(#[from] anyhow::Error),

    /// A spawned blocking/audio task failed to join.
    #[error("audio task join failed: {0}")]
    TaskJoin(String),

    /// A spawned transcription task panicked or was cancelled.
    #[error("Transcription task panicked: {0}")]
    TranscriptionJoin(String),

    /// A spawned media-decoding task panicked or was cancelled.
    #[error("Decoding task panicked: {0}")]
    DecodeJoin(String),

    /// A spawned model-rescan task panicked or was cancelled.
    #[error("rescan task panicked: {0}")]
    RescanJoin(String),

    /// cpal device enumeration or configuration failed.
    #[error("{0}")]
    AudioDevices(String),

    /// No audio input device is available.
    #[error("No input device available")]
    NoInputDevice,

    /// The selected sample format is not supported by the mic monitor.
    #[error("Unsupported sample format")]
    UnsupportedSampleFormat,

    /// A history entry id has no matching database row.
    #[error("History entry {0} not found")]
    HistoryEntryNotFound(i64),

    /// The history entry's audio file is missing from disk.
    #[error("Audio file '{0}' does not exist")]
    AudioFileMissing(String),

    /// The archived recording holds no PCM samples.
    #[error("Recording has no audio samples")]
    EmptyRecording,

    /// The transcription came back empty (silence / no speech).
    #[error("Recording contains no speech")]
    NoSpeech,

    /// A media file could not be decoded.
    #[error("Failed to decode media file: {0}")]
    Decode(String),

    /// Local/cloud transcription of the samples failed.
    #[error("Transcription failed: {0}")]
    Transcribe(String),

    /// Loading archived audio from disk failed.
    #[error("Failed to load audio: {0}")]
    AudioLoad(String),

    /// A model id has no catalog entry.
    #[error("Model not found: {0}")]
    ModelNotFound(String),

    /// The model exists but its files are not downloaded.
    #[error("Model not downloaded: {0}")]
    ModelNotDownloaded(String),

    /// A second model load was attempted while one is in flight.
    #[error("Model load already in progress")]
    ModelLoadBusy,

    /// Unloading the resident model failed.
    #[error("Failed to unload model: {0}")]
    ModelUnload(String),

    /// Updating the microphone mode failed.
    #[error("Failed to update microphone mode: {0}")]
    MicrophoneMode(String),

    /// Updating the selected input device failed.
    #[error("Failed to update selected device: {0}")]
    SelectedDevice(String),

    /// Updating the capture source failed.
    #[error("Failed to update capture source: {0}")]
    CaptureSource(String),

    /// Updating the loopback device failed.
    #[error("Failed to update system audio device: {0}")]
    SystemAudioDevice(String),

    /// Keyboard/mouse simulation could not be initialized or driven.
    #[error("{0}")]
    Input(String),

    /// An agent id has no matching configuration.
    #[error("Agent '{0}' not found")]
    AgentNotFound(String),

    /// The agent's provider binding is unusable (unknown/disabled).
    #[error("{0}")]
    AgentProvider(String),

    /// No model is configured for the agent's provider.
    #[error("{0}")]
    AgentModel(String),

    /// A tool call was rejected before execution (unknown tool, bad args).
    #[error("{0}")]
    ToolDenied(String),

    /// A tool's network call failed or timed out.
    #[error("{0}")]
    ToolFailed(String),

    /// TTS synthesis or playback failed.
    #[error("Text-to-speech failed: {0}")]
    Tts(String),

    /// There is nothing speakable in the requested text.
    #[error("Nothing to read: the text is empty")]
    TtsNothingToRead,
}

impl CommandError {
    /// Wrap a Tokio join failure with the legacy `"audio task join failed"`
    /// message used across the audio/model command paths.
    pub fn task_join(err: tokio::task::JoinError) -> Self {
        Self::TaskJoin(err.to_string())
    }

    /// Wrap a Tokio join failure from the model-rescan path.
    pub fn rescan_join(err: tokio::task::JoinError) -> Self {
        Self::RescanJoin(err.to_string())
    }

    /// Wrap a Tokio join failure from the transcription path.
    pub fn transcription_join(err: tokio::task::JoinError) -> Self {
        Self::TranscriptionJoin(err.to_string())
    }

    /// Wrap a Tokio join failure from the media-decode path.
    pub fn decode_join(err: tokio::task::JoinError) -> Self {
        Self::DecodeJoin(err.to_string())
    }

    /// Build the legacy `"Failed to list audio devices: {e}"` error.
    pub fn list_devices(err: impl std::fmt::Display) -> Self {
        Self::AudioDevices(format!("Failed to list audio devices: {err}"))
    }

    /// Build the legacy `"Failed to list system audio sources: {e}"` error.
    pub fn list_system_sources(err: impl std::fmt::Display) -> Self {
        Self::AudioDevices(format!("Failed to list system audio sources: {err}"))
    }

    /// Build the legacy `"Failed to get input config: {e}"` monitor error.
    pub fn input_config(err: impl std::fmt::Display) -> Self {
        Self::AudioDevices(format!("Failed to get input config: {err}"))
    }

    /// Build the legacy `"Failed to build monitor stream: {e}"` error.
    pub fn build_monitor_stream(err: impl std::fmt::Display) -> Self {
        Self::AudioDevices(format!("Failed to build monitor stream: {err}"))
    }

    /// Build the legacy `"Failed to play monitor stream: {e}"` error.
    pub fn play_monitor_stream(err: impl std::fmt::Display) -> Self {
        Self::AudioDevices(format!("Failed to play monitor stream: {err}"))
    }

    /// Missing audio path for file-based flows (keeps the `PathBuf` for
    /// programmatic callers while displaying the legacy file-name message).
    pub fn audio_file_missing(path: &PathBuf, file_name: &str) -> Self {
        let _ = path;
        Self::AudioFileMissing(file_name.to_string())
    }
}

/// Legacy alias: command functions return `Result<T, CommandError>`.
pub type CommandResult<T> = Result<T, CommandError>;
