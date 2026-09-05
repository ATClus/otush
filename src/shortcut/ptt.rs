//! Native Push-to-Talk key release detection on Linux.
//!
//! While XDG Desktop Portal delivers global shortcut trigger (`Activated`) events,
//! it does not emit key release events. When Push-to-Talk mode is enabled, this
//! module monitors the physical state of the shortcut's keys via `/dev/input` (`evdev`)
//! and dispatches a release event as soon as the user releases the key(s).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use evdev::Key;
use log::{debug, warn};

use crate::context::AppContext;

static CURRENT_WATCHER: LazyLock<Mutex<Option<Arc<AtomicBool>>>> =
    LazyLock::new(|| Mutex::new(None));

/// Map a shortcut token (e.g. "ctrl", "space", "a") to possible matching `evdev::Key`s.
pub fn map_token_to_keys(token: &str) -> Vec<Key> {
    match token.trim().to_lowercase().as_str() {
        "ctrl" | "control" => vec![Key::KEY_LEFTCTRL, Key::KEY_RIGHTCTRL],
        "alt" | "opt" | "option" => vec![Key::KEY_LEFTALT, Key::KEY_RIGHTALT],
        "shift" => vec![Key::KEY_LEFTSHIFT, Key::KEY_RIGHTSHIFT],
        "super" | "win" | "meta" | "cmd" | "command" => vec![Key::KEY_LEFTMETA, Key::KEY_RIGHTMETA],
        "space" => vec![Key::KEY_SPACE],
        "esc" | "escape" => vec![Key::KEY_ESC],
        "enter" | "return" => vec![Key::KEY_ENTER, Key::KEY_KPENTER],
        "tab" => vec![Key::KEY_TAB],
        "backspace" => vec![Key::KEY_BACKSPACE],
        "f1" => vec![Key::KEY_F1],
        "f2" => vec![Key::KEY_F2],
        "f3" => vec![Key::KEY_F3],
        "f4" => vec![Key::KEY_F4],
        "f5" => vec![Key::KEY_F5],
        "f6" => vec![Key::KEY_F6],
        "f7" => vec![Key::KEY_F7],
        "f8" => vec![Key::KEY_F8],
        "f9" => vec![Key::KEY_F9],
        "f10" => vec![Key::KEY_F10],
        "f11" => vec![Key::KEY_F11],
        "f12" => vec![Key::KEY_F12],
        "a" => vec![Key::KEY_A],
        "b" => vec![Key::KEY_B],
        "c" => vec![Key::KEY_C],
        "d" => vec![Key::KEY_D],
        "e" => vec![Key::KEY_E],
        "f" => vec![Key::KEY_F],
        "g" => vec![Key::KEY_G],
        "h" => vec![Key::KEY_H],
        "i" => vec![Key::KEY_I],
        "j" => vec![Key::KEY_J],
        "k" => vec![Key::KEY_K],
        "l" => vec![Key::KEY_L],
        "m" => vec![Key::KEY_M],
        "n" => vec![Key::KEY_N],
        "o" => vec![Key::KEY_O],
        "p" => vec![Key::KEY_P],
        "q" => vec![Key::KEY_Q],
        "r" => vec![Key::KEY_R],
        "s" => vec![Key::KEY_S],
        "t" => vec![Key::KEY_T],
        "u" => vec![Key::KEY_U],
        "v" => vec![Key::KEY_V],
        "w" => vec![Key::KEY_W],
        "x" => vec![Key::KEY_X],
        "y" => vec![Key::KEY_Y],
        "z" => vec![Key::KEY_Z],
        "0" => vec![Key::KEY_0],
        "1" => vec![Key::KEY_1],
        "2" => vec![Key::KEY_2],
        "3" => vec![Key::KEY_3],
        "4" => vec![Key::KEY_4],
        "5" => vec![Key::KEY_5],
        "6" => vec![Key::KEY_6],
        "7" => vec![Key::KEY_7],
        "8" => vec![Key::KEY_8],
        "9" => vec![Key::KEY_9],
        "pause" => vec![Key::KEY_PAUSE],
        "insert" => vec![Key::KEY_INSERT],
        "delete" => vec![Key::KEY_DELETE],
        "home" => vec![Key::KEY_HOME],
        "end" => vec![Key::KEY_END],
        "pageup" | "prior" => vec![Key::KEY_PAGEUP],
        "pagedown" | "next" => vec![Key::KEY_PAGEDOWN],
        "scroll_lock" | "scrolllock" => vec![Key::KEY_SCROLLLOCK],
        "caps_lock" | "capslock" => vec![Key::KEY_CAPSLOCK],
        "grave" | "backquote" | "`" => vec![Key::KEY_GRAVE],
        _ => Vec::new(),
    }
}

pub struct ShortcutKeyMatcher {
    pub primary_keys: Vec<Key>,
}

impl ShortcutKeyMatcher {
    pub fn from_binding(binding_str: &str) -> Self {
        let tokens: Vec<&str> = binding_str.split('+').map(|s| s.trim()).collect();
        let mut primary_keys = Vec::new();
        let mut all_keys = Vec::new();

        for token in &tokens {
            let mapped = map_token_to_keys(token);
            let is_modifier = matches!(
                token.to_lowercase().as_str(),
                "ctrl"
                    | "control"
                    | "alt"
                    | "opt"
                    | "option"
                    | "shift"
                    | "super"
                    | "win"
                    | "meta"
                    | "cmd"
                    | "command"
            );
            if !is_modifier {
                primary_keys.extend(mapped.iter().copied());
            }
            all_keys.extend(mapped);
        }

        // If shortcut consists only of modifiers (e.g. "ctrl+shift"), check all keys
        if primary_keys.is_empty() {
            primary_keys = all_keys;
        }

        Self { primary_keys }
    }

    pub fn open_key_devices() -> Vec<evdev::Device> {
        let mut devices = Vec::new();
        for (_, device) in evdev::enumerate() {
            if device.supported_keys().is_some() {
                devices.push(device);
            }
        }
        devices
    }

    pub fn is_held_on(&self, devices: &mut [evdev::Device]) -> bool {
        if self.primary_keys.is_empty() {
            return false;
        }
        for device in devices.iter_mut() {
            if let Ok(state) = device.get_key_state() {
                for key in &self.primary_keys {
                    if state.contains(*key) {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// Start watching for the release of the shortcut keys.
/// When the user releases the keys, sends `is_pressed: false` to the coordinator.
pub fn start_ptt_release_watcher(ctx: &AppContext, binding_id: &str, hotkey_str: &str) {
    // Cancel any previous watcher
    {
        let mut guard = CURRENT_WATCHER.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(prev) = guard.take() {
            prev.store(false, Ordering::SeqCst);
        }
    }

    let matcher = ShortcutKeyMatcher::from_binding(hotkey_str);
    if matcher.primary_keys.is_empty() {
        debug!(
            "No evdev keys mapped for shortcut '{}'; PTT watcher skipped",
            hotkey_str
        );
        return;
    }

    let is_active = Arc::new(AtomicBool::new(true));
    {
        let mut guard = CURRENT_WATCHER.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some(Arc::clone(&is_active));
    }

    let ctx = ctx.clone();
    let binding_id = binding_id.to_string();
    let hotkey_str = hotkey_str.to_string();

    crate::runtime::spawn(async move {
        // Open keyboard devices once for lightweight, low-latency polling
        let mut devices = ShortcutKeyMatcher::open_key_devices();
        if devices.is_empty() {
            warn!("No accessible evdev keyboard devices found for PTT release watcher");
            return;
        }

        // Small grace period for key-down state to stabilize
        tokio::time::sleep(Duration::from_millis(60)).await;

        let mut consecutive_released = 0;
        let check_interval = Duration::from_millis(20);

        while is_active.load(Ordering::SeqCst) {
            let pressed = matcher.is_held_on(&mut devices);
            if !pressed {
                consecutive_released += 1;
                // Require 2 consecutive release checks (~40ms) to ensure clean release
                if consecutive_released >= 2 {
                    debug!("PTT release detected for '{}'", binding_id);
                    ctx.coordinator
                        .send_input(&binding_id, &hotkey_str, false, true);
                    break;
                }
            } else {
                consecutive_released = 0;
            }

            tokio::time::sleep(check_interval).await;
        }

        let mut guard = CURRENT_WATCHER.lock().unwrap_or_else(|e| e.into_inner());
        *guard = None;
    });
}

/// Stop any active release watcher.
pub fn cancel_ptt_release_watcher() {
    let mut guard = CURRENT_WATCHER.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(watcher) = guard.take() {
        watcher.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evdev_devices() {
        println!("Inspecting evdev input devices...");
        let check_keys = vec![
            Key::KEY_SPACE,
            Key::KEY_LEFTCTRL,
            Key::KEY_RIGHTCTRL,
            Key::KEY_LEFTALT,
        ];
        let mut count = 0;
        for (path, device) in evdev::enumerate() {
            count += 1;
            let name = device.name().unwrap_or("unknown");
            if let Ok(state) = device.get_key_state() {
                for key in &check_keys {
                    if state.contains(*key) {
                        println!("ACTIVE KEY on {:?} ({}): {:?}", path, name, key);
                    }
                }
            }
        }
        println!("Checked {count} devices for active keys.");
    }
}
