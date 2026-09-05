//! Speech-to-text pipeline (transcribe-cpp / transcribe-rs).
//!
//! Split into modules by concern; this facade re-exports the public surface
//! so all existing `crate::managers::transcription::X` paths keep working:
//! [`types`] (stream events, router, engine guards), [`manager`]
//! (manager lifecycle + streaming + batch methods), [`streaming`]
//! (worker helpers), [`transcribe`] (batch helpers),
//! [`backend`] (devices/accelerators).

pub mod backend;
pub mod manager;
pub mod streaming;
#[cfg(test)]
mod tests;
pub mod transcribe;
pub mod types;

pub use backend::{
    apply_accelerator_settings, describe_compute_devices, get_available_accelerators,
    init_transcribe_backend,
};
pub use manager::TranscriptionManager;
pub use types::{
    FinalizedStreamText, ModelStateEvent, StreamCmd, StreamPhaseEvent, StreamRouter,
    StreamTextEvent, StreamWorkKind,
};
// NOTE: `types::LoadingGuard` is constructed via `TranscriptionManager::
// try_start_loading` and named only inside the tree (`super::types::LoadingGuard`);
// callers hold it as `let _guard`. The facade omits it: zero external name uses.
// NOTE: `streaming::StreamPerf`, `types::{GpuDeviceOption, LoadedEngine,
// StreamPhase, StreamWorkerGuard}` stay module-private to the transcription
// tree via `super::` imports (no facade re-export: zero external callers).
