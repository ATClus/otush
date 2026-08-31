use crate::context::AppContext;
use crate::shortcut;
use log::info;

// Re-export all utility modules for easy access
pub use crate::clipboard::*;
pub use crate::overlay::*;
pub use crate::tray::*;

/// Preserve diagnostic text in development builds, but redact it in releases.
/// Do not use for secrets such as API keys, which must always be redacted.
pub fn redact_text(text: &str) -> &str {
    if cfg!(debug_assertions) {
        text
    } else {
        "[REDACTED]"
    }
}

/// Centralized cancellation function that can be called from anywhere in the app.
/// Handles cancelling both recording and transcription operations and updates UI state.
pub fn cancel_current_operation(ctx: &AppContext) {
    info!("Initiating operation cancellation...");

    // Unregister the cancel shortcut asynchronously
    shortcut::unregister_cancel_shortcut(ctx);

    // Cancel any ongoing recording
    let audio_manager = &ctx.audio;
    let recording_was_active = audio_manager.is_recording();
    audio_manager.cancel_recording();

    // Abandon any live streaming transcription
    let tm = &ctx.transcription;
    tm.cancel_stream();

    // Update tray icon and hide overlay
    set_tray_state(ctx, crate::tray::TrayIconState::Idle);
    hide_recording_overlay(ctx);

    // Unload model if immediate unload is enabled
    tm.maybe_unload_immediately("cancellation");

    // Notify coordinator so it can keep lifecycle state coherent.
    ctx.coordinator.notify_cancel(recording_was_active);

    info!("Operation cancellation completed - returned to idle state");
}

/// Check if running on a Wayland display server session.
pub fn is_wayland() -> bool {
    std::env::var("WAYLAND_DISPLAY").is_ok()
        || std::env::var("XDG_SESSION_TYPE")
            .map(|v| v.to_lowercase() == "wayland")
            .unwrap_or(false)
}

/// Check if running on KDE Plasma desktop environment.
pub fn is_kde_plasma() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .map(|v| v.to_uppercase().contains("KDE"))
        .unwrap_or(false)
        || std::env::var("KDE_SESSION_VERSION").is_ok()
}

/// Check if running on KDE Plasma with Wayland.
pub fn is_kde_wayland() -> bool {
    is_wayland() && is_kde_plasma()
}

/// Check if running on GNOME desktop environment.
pub fn is_gnome() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .map(|v| v.to_uppercase().contains("GNOME"))
        .unwrap_or(false)
}

/// Check if running on GNOME with Wayland.
pub fn is_gnome_wayland() -> bool {
    is_wayland() && is_gnome()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_text_keeps_text_in_debug_builds() {
        assert_eq!(redact_text("hello"), "hello");
    }
}
