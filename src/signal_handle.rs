//! Unix signal handling for remote transcription control.
//!
//! Listens for POSIX signals (`SIGUSR2`) to remotely toggle recording/transcription.

use crate::context::AppContext;
use log::debug;
use signal_hook::consts::SIGUSR2;
use signal_hook::iterator::Signals;
use std::thread;

/// Send an external transcription input event to the coordinator.
///
/// Used by signal handlers, CLI flags, and external IPC integrations.
pub fn send_transcription_input(ctx: &AppContext, binding_id: &str, source: &str) {
    ctx.coordinator.send_external_input(binding_id, source);
}

/// Listen for Unix signals that remotely toggle transcription.
///
/// On Linux, `SIGUSR2` toggles plain transcription. `SIGUSR1` is intentionally
/// unhandled because WebKitGTK / JavaScriptCore threads use `SIGUSR1` internally
/// for garbage collector thread suspension. Linux remote post-processing control
/// is provided via `otush --toggle-post-process` CLI flag instead.
#[cfg(unix)]
pub fn setup_signal_handler(ctx: &AppContext) {
    let mut signals =
        Signals::new([SIGUSR2]).expect("failed to register transcription signal handlers");
    debug!("Signal handler registered (SIGUSR2; SIGUSR1 is left to WebKitGTK)");

    let ctx = ctx.clone();
    thread::spawn(move || {
        for sig in signals.forever() {
            if sig == SIGUSR2 {
                debug!("Received SIGUSR2");
                send_transcription_input(&ctx, "transcribe", "SIGUSR2");
            }
        }
    });
}
