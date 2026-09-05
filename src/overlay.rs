//! Recording overlay: fullscreen transparent click-through surface with a
//! Cairo-painted pill (state, mic waveform, streaming text).
//!
//! Split into modules by concern; this facade re-exports the public surface
//! so all existing `crate::overlay::X` paths keep working:
//! [`state`] (lifecycle + events), [`paint`] (layout + drawing).

pub mod paint;
pub mod state;
#[cfg(test)]
mod tests;

pub use state::{
    emit_levels, emit_recording_ready, hide_recording_overlay, init_overlay,
    show_meeting_processing_overlay, show_meeting_recording_overlay,
    show_meeting_streaming_overlay, show_meeting_transcribing_overlay, show_processing_overlay,
    show_recording_overlay, show_streaming_overlay, show_transcribing_overlay,
    update_overlay_enabled_cache, update_overlay_position,
};
