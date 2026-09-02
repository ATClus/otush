//! Direct Linux evdev global keyboard shortcut listener.
//!
//! Queries physical key states directly from `/dev/input/event*` devices via `evdev`,
//! allowing shortcuts to work globally across all applications immediately upon
//! configuration, without relying exclusively on XDG Desktop Portal authorization.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use evdev::Key;
use log::{debug, info};

use crate::context::{AppContext, AppEvent};
use crate::settings::ShortcutBinding;
use crate::shortcut::handler::handle_shortcut_event;
use crate::shortcut::ptt::map_token_to_keys;

static EVDEV_RUNNING: AtomicBool = AtomicBool::new(false);
static ACTIVE_BINDINGS: OnceLock<Arc<Mutex<HashMap<String, ShortcutBinding>>>> = OnceLock::new();

fn get_active_bindings_map() -> Arc<Mutex<HashMap<String, ShortcutBinding>>> {
    Arc::clone(ACTIVE_BINDINGS.get_or_init(|| Arc::new(Mutex::new(HashMap::new()))))
}

pub fn init_shortcuts(ctx: &AppContext) {
    if EVDEV_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }

    // Seed bindings
    let bindings_map = get_active_bindings_map();
    {
        let settings = ctx.settings();
        let mut map = bindings_map.lock().unwrap();
        for (id, binding) in &settings.bindings {
            map.insert(id.clone(), binding.clone());
        }
    }

    // Subscribe to settings changes to keep bindings live
    let b_map = bindings_map.clone();
    let ctx_sub = ctx.clone();
    ctx.bus.subscribe(move |event| {
        if let AppEvent::SettingsChanged { setting, .. } = event {
            if setting == "bindings" {
                let settings = ctx_sub.settings();
                let mut map = b_map.lock().unwrap();
                map.clear();
                for (id, binding) in &settings.bindings {
                    map.insert(id.clone(), binding.clone());
                }
                debug!("Evdev listener updated with {} bindings", map.len());
            }
        }
    });

    // Spawn the evdev listener worker thread
    let ctx = ctx.clone();
    std::thread::Builder::new()
        .name("otush-evdev-listener".to_string())
        .spawn(move || {
            run_evdev_listener_loop(&ctx, &bindings_map);
        })
        .expect("failed to spawn evdev listener thread");
}

fn run_evdev_listener_loop(
    ctx: &AppContext,
    bindings_map: &Arc<Mutex<HashMap<String, ShortcutBinding>>>,
) {
    info!("Starting direct evdev global keyboard shortcut monitor");

    let mut triggered_bindings: HashSet<String> = HashSet::new();
    let mut current_held_keys: HashSet<Key> = HashSet::new();
    let poll_interval = Duration::from_millis(15);

    loop {
        // Enumerate accessible keyboard devices
        let mut devices = crate::shortcut::ptt::ShortcutKeyMatcher::open_key_devices();
        if devices.is_empty() {
            // If no devices accessible (e.g. running without input group permissions), wait before retrying
            std::thread::sleep(Duration::from_secs(3));
            continue;
        }

        loop {
            // When XDG Desktop Portal is active, it handles global shortcuts natively.
            // Direct evdev shortcut listener is only a fallback when portal is unavailable.
            if crate::shortcut::portal::is_initialized() {
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }

            current_held_keys.clear();

            for dev in &mut devices {
                if let Ok(state) = dev.get_key_state() {
                    for k in state.iter() {
                        current_held_keys.insert(k);
                    }
                }
            }

            // Check matches under lock without extra heap clones per iteration
            {
                let map = bindings_map.lock().unwrap();
                for binding in map.values() {
                    if binding.id == "cancel" && !ctx.audio.is_recording() {
                        continue;
                    }

                    let satisfied =
                        is_shortcut_satisfied(&binding.current_binding, &current_held_keys);

                    if satisfied {
                        if !triggered_bindings.contains(&binding.id) {
                            triggered_bindings.insert(binding.id.clone());
                            debug!(
                                "Direct evdev shortcut triggered: {} ({})",
                                binding.id, binding.current_binding
                            );
                            handle_shortcut_event(ctx, &binding.id, &binding.current_binding, true);
                        }
                    } else if triggered_bindings.contains(&binding.id) {
                        triggered_bindings.remove(&binding.id);
                        debug!(
                            "Direct evdev shortcut released: {} ({})",
                            binding.id, binding.current_binding
                        );
                        handle_shortcut_event(ctx, &binding.id, &binding.current_binding, false);
                    }
                }
            }

            std::thread::sleep(poll_interval);
        }
    }
}

/// Verify whether every token in `binding_str` (e.g. "ctrl+alt+space") has an active held key.
fn is_shortcut_satisfied(binding_str: &str, held_keys: &HashSet<Key>) -> bool {
    let mut tokens = binding_str
        .split('+')
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let mut has_tokens = false;

    for token in tokens.by_ref() {
        has_tokens = true;
        let possible_keys = map_token_to_keys(token);
        if possible_keys.is_empty() || !possible_keys.iter().any(|k| held_keys.contains(k)) {
            return false;
        }
    }

    has_tokens
}
