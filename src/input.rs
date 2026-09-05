//! Keyboard and mouse simulation utilities using Enigo.
//!
//! Provides wrappers for simulating paste keystrokes (`Ctrl+V`, `Ctrl+Shift+V`,
//! `Shift+Insert`), direct typing, and cursor tracking.

use enigo::{Enigo, Key, Keyboard, Settings};
use std::sync::{Mutex, OnceLock};

/// Thread-safe wrapper around [`Enigo`].
///
/// Enigo requires mutable access for keystroke simulation and cursor queries.
pub struct EnigoState(pub Mutex<Enigo>);

impl EnigoState {
    /// Create a new `EnigoState` with default platform settings.
    pub fn new() -> Result<Self, String> {
        let enigo = Enigo::new(&Settings::default())
            .map_err(|e| format!("Failed to initialize Enigo: {}", e))?;
        Ok(Self(Mutex::new(enigo)))
    }
}

/// Process-wide Enigo instance, initialized after onboarding or on application startup.
static ENIGO_STATE: OnceLock<EnigoState> = OnceLock::new();

/// Initialize the global Enigo instance.
///
/// This function is idempotent: calling it multiple times has no effect once initialized.
pub fn initialize_enigo() -> Result<(), String> {
    if ENIGO_STATE.get().is_none() {
        let state = EnigoState::new()?;
        let _ = ENIGO_STATE.set(state);
    }
    log::info!("Enigo initialized successfully");
    Ok(())
}

/// Access the initialized global Enigo state, if available.
pub fn get_enigo() -> Option<&'static EnigoState> {
    ENIGO_STATE.get()
}

/// Sends a `Ctrl+V` paste keystroke chord.
///
/// `hold_ms` specifies how many milliseconds the `Ctrl` modifier should remain held
/// after clicking `v` before being released.
pub fn send_paste_ctrl_v(enigo: &mut Enigo, hold_ms: u64) -> Result<(), String> {
    let modifier_key = Key::Control;
    let v_key_code = Key::Unicode('v');

    enigo
        .key(modifier_key, enigo::Direction::Press)
        .map_err(|e| format!("Failed to press modifier key: {}", e))?;
    enigo
        .key(v_key_code, enigo::Direction::Click)
        .map_err(|e| format!("Failed to click V key: {}", e))?;

    if hold_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(hold_ms));
    }

    enigo
        .key(modifier_key, enigo::Direction::Release)
        .map_err(|e| format!("Failed to release modifier key: {}", e))?;

    Ok(())
}

/// Sends a `Ctrl+Shift+V` paste keystroke chord.
///
/// Commonly used in terminal emulators and rich-text editors for unformatted paste.
pub fn send_paste_ctrl_shift_v(enigo: &mut Enigo, hold_ms: u64) -> Result<(), String> {
    let modifier_key = Key::Control;
    let v_key_code = Key::Unicode('v');

    enigo
        .key(modifier_key, enigo::Direction::Press)
        .map_err(|e| format!("Failed to press modifier key: {}", e))?;
    enigo
        .key(Key::Shift, enigo::Direction::Press)
        .map_err(|e| format!("Failed to press Shift key: {}", e))?;
    enigo
        .key(v_key_code, enigo::Direction::Click)
        .map_err(|e| format!("Failed to click V key: {}", e))?;

    if hold_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(hold_ms));
    }

    enigo
        .key(Key::Shift, enigo::Direction::Release)
        .map_err(|e| format!("Failed to release Shift key: {}", e))?;
    enigo
        .key(modifier_key, enigo::Direction::Release)
        .map_err(|e| format!("Failed to release modifier key: {}", e))?;

    Ok(())
}

/// Sends a `Shift+Insert` paste keystroke chord.
///
/// Universal alternative for terminal applications and legacy X11/Wayland software.
pub fn send_paste_shift_insert(enigo: &mut Enigo, hold_ms: u64) -> Result<(), String> {
    let insert_key_code = Key::Other(0x76); // XK_Insert (keycode 118 / 0x76)

    enigo
        .key(Key::Shift, enigo::Direction::Press)
        .map_err(|e| format!("Failed to press Shift key: {}", e))?;
    enigo
        .key(insert_key_code, enigo::Direction::Click)
        .map_err(|e| format!("Failed to click Insert key: {}", e))?;

    if hold_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(hold_ms));
    }

    enigo
        .key(Key::Shift, enigo::Direction::Release)
        .map_err(|e| format!("Failed to release Shift key: {}", e))?;

    Ok(())
}

/// Pastes text directly by simulating sequential character input.
pub fn paste_text_direct(enigo: &mut Enigo, text: &str) -> Result<(), String> {
    enigo
        .text(text)
        .map_err(|e| format!("Failed to send text directly: {}", e))?;

    Ok(())
}
