#![allow(dead_code)]
//! XDG Desktop Portal `GlobalShortcuts` engine (Wayland-native, GNOME).
//!
//! GNOME blocks raw global grabs on Wayland, so bindings go through
//! `org.freedesktop.portal.GlobalShortcuts` (via `ashpd`): the compositor
//! owns the actual key capture and delivers press events to us through the
//! portal's `Activated` signal. Binding is session-based and replaces the
//! whole set, so every change (register/unregister/cancel lifecycle) updates
//! the shared desired list and re-sends it.
//!
//! On sessions without the portal (older GNOME, missing xdg-desktop-portal),
//! `init_shortcuts` logs the failure and the app falls back to the
//! evdev-keys (evdev) engine.

use crate::context::AppContext;
use crate::settings::ShortcutBinding;
use crate::shortcut::handler::handle_shortcut_event;
use ashpd::desktop::global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut};
use ashpd::desktop::Session;
use futures_util::StreamExt;
use log::{debug, warn};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock};

/// Desired bindings (binding_id → binding), shared with the portal task.
static DESIRED: OnceLock<Arc<Mutex<HashMap<String, ShortcutBinding>>>> = OnceLock::new();
/// Signals the portal task to re-bind after a change to `DESIRED`.
static REBIND_TX: OnceLock<async_channel::Sender<()>> = OnceLock::new();
/// True once the portal session is established and bindings are live.
static PORTAL_READY: AtomicBool = AtomicBool::new(false);

/// The live portal proxy + session, established asynchronously.
struct PortalHandle {
    proxy: GlobalShortcuts,
    session: Session<GlobalShortcuts>,
}

static PORTAL: OnceLock<Arc<PortalHandle>> = OnceLock::new();

pub fn is_initialized() -> bool {
    PORTAL_READY.load(std::sync::atomic::Ordering::Relaxed)
}

fn desired_map() -> Arc<Mutex<HashMap<String, ShortcutBinding>>> {
    Arc::clone(DESIRED.get_or_init(|| Arc::new(Mutex::new(HashMap::new()))))
}

fn format_portal_trigger(binding: &str) -> String {
    let parts: Vec<String> = binding
        .split('+')
        .map(|token| match token.trim().to_lowercase().as_str() {
            "ctrl" | "control" => "Ctrl".to_string(),
            "alt" | "opt" | "option" => "Alt".to_string(),
            "shift" => "Shift".to_string(),
            "super" | "win" | "meta" | "cmd" | "command" => "Super".to_string(),
            "space" => "Space".to_string(),
            "esc" | "escape" => "Escape".to_string(),
            "enter" | "return" => "Return".to_string(),
            "tab" => "Tab".to_string(),
            "backspace" => "BackSpace".to_string(),
            "delete" => "Delete".to_string(),
            "insert" => "Insert".to_string(),
            "pause" => "Pause".to_string(),
            "home" => "Home".to_string(),
            "end" => "End".to_string(),
            "pageup" | "prior" => "Page_Up".to_string(),
            "pagedown" | "next" => "Page_Down".to_string(),
            "scroll_lock" | "scrolllock" => "Scroll_Lock".to_string(),
            other if other.len() == 1 => other.to_uppercase(),
            other => {
                let mut c = other.chars();
                match c.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
                    None => other.to_string(),
                }
            }
        })
        .collect();
    parts.join("+")
}

/// Send the current desired list to the portal.
async fn rebind(
    handle: &PortalHandle,
    desired: &Arc<Mutex<HashMap<String, ShortcutBinding>>>,
) -> Result<usize, String> {
    let formatted_triggers: Vec<(String, String, String)> = {
        let map = desired.lock().unwrap();
        map.values()
            .filter(|b| !b.current_binding.trim().is_empty())
            .map(|b| {
                (
                    b.id.clone(),
                    b.name.clone(),
                    format_portal_trigger(&b.current_binding),
                )
            })
            .collect()
    };

    let shortcuts: Vec<NewShortcut> = formatted_triggers
        .iter()
        .map(|(id, name, trigger)| {
            NewShortcut::new(id.clone(), name.clone()).preferred_trigger(Some(trigger.as_str()))
        })
        .collect();

    let request = handle
        .proxy
        .bind_shortcuts(
            &handle.session,
            &shortcuts,
            None,
            BindShortcutsOptions::default(),
        )
        .await
        .map_err(|e| e.to_string())?;

    match request.response() {
        Ok(response) => Ok(response.shortcuts().len()),
        Err(e) => {
            debug!("Portal bind_shortcuts status: {e}");
            Ok(0)
        }
    }
}

/// Initialize the portal engine: establish a session, bind the current
/// shortcuts, and listen for activations. Safe to call from any thread.
pub fn init_shortcuts(ctx: &AppContext) {
    if PORTAL.get().is_some() {
        return;
    }

    // Seed the desired list with the persisted bindings (cancel is dynamic).
    let desired = desired_map();
    {
        let settings = crate::settings::get_settings(ctx);
        let mut map = desired.lock().unwrap();
        for (id, binding) in &settings.bindings {
            if id != "cancel" {
                map.insert(id.clone(), binding.clone());
            }
        }
    }

    let (tx, rx) = async_channel::unbounded::<()>();
    let _ = REBIND_TX.set(tx);

    let ctx = ctx.clone();
    crate::runtime::spawn(async move {
        // Ensure desktop entry and prgname are registered for XDG Desktop Portal
        glib::set_prgname(Some("com.clusterat.otush"));
        crate::autostart::ensure_desktop_entry_registered();

        // Modern xdg-desktop-portal (GNOME 46+) requires host apps to register their App ID
        if let Ok(app_id) = ashpd::AppID::from_str("com.clusterat.otush") {
            if !ashpd::is_sandboxed() {
                if let Err(e) = ashpd::register_host_app(app_id).await {
                    debug!("Host app registration note: {e}");
                }
            }
        }

        // Establish the portal session and bind the current shortcuts.
        let setup = async {
            let proxy = GlobalShortcuts::new().await.map_err(|e| e.to_string())?;
            let session = proxy
                .create_session(Default::default())
                .await
                .map_err(|e| e.to_string())?;
            let handle = Arc::new(PortalHandle { proxy, session });
            let _ = PORTAL.set(handle.clone());
            rebind(&handle, &desired).await?;
            Ok::<Arc<PortalHandle>, String>(handle)
        };

        match setup.await {
            Ok(handle) => {
                PORTAL_READY.store(true, std::sync::atomic::Ordering::Relaxed);

                // Listen for activations and dispatch.
                let ctx2 = ctx.clone();
                let handle2 = handle.clone();
                crate::runtime::spawn(async move {
                    match handle2.proxy.receive_activated().await {
                        Ok(mut stream) => {
                            while let Some(activated) = stream.next().await {
                                let shortcut_id = activated.shortcut_id().to_string();
                                debug!("Portal shortcut activated: {shortcut_id}");
                                handle_shortcut_event(&ctx2, &shortcut_id, "", true);
                            }
                        }
                        Err(e) => {
                            warn!("Failed to listen for portal shortcut activations: {e}")
                        }
                    }
                });

                // Rebind loop: react to register/unregister/cancel changes.
                while rx.recv().await.is_ok() {
                    if let Err(e) = rebind(&handle, &desired).await {
                        warn!("Failed to rebind portal shortcuts: {e}");
                    }
                }
            }
            Err(e) => {
                warn!("Global shortcuts portal could not be initialized: {e}");
            }
        }
    });
}

/// Register a shortcut: add to the desired list and re-bind.
pub fn register_shortcut(_ctx: &AppContext, binding: ShortcutBinding) -> Result<(), String> {
    desired_map()
        .lock()
        .unwrap()
        .insert(binding.id.clone(), binding);
    let _ = REBIND_TX.get().map(|tx| tx.try_send(()));
    Ok(())
}

/// Unregister a shortcut: remove from the desired list and re-bind.
pub fn unregister_shortcut(_ctx: &AppContext, binding: ShortcutBinding) -> Result<(), String> {
    desired_map().lock().unwrap().remove(&binding.id);
    let _ = REBIND_TX.get().map(|tx| tx.try_send(()));
    Ok(())
}

/// Register the cancel shortcut (called when recording starts).
pub fn register_cancel_shortcut(ctx: &AppContext) {
    if let Some(binding) = crate::settings::get_settings(ctx)
        .bindings
        .get("cancel")
        .cloned()
    {
        desired_map()
            .lock()
            .unwrap()
            .insert("cancel".to_string(), binding);
        let _ = REBIND_TX.get().map(|tx| tx.try_send(()));
    }
}

/// Unregister the cancel shortcut (called when recording stops).
pub fn unregister_cancel_shortcut(_ctx: &AppContext) {
    desired_map().lock().unwrap().remove("cancel");
    let _ = REBIND_TX.get().map(|tx| tx.try_send(()));
}

/// Validate a shortcut string for the portal engine (e.g. "Ctrl+Space", "Super+Shift+R").
pub fn validate_shortcut(raw: &str) -> Result<(), String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Shortcut cannot be empty".into());
    }
    let parts: Vec<&str> = trimmed.split('+').map(str::trim).collect();
    if parts.is_empty() || parts.iter().any(|p| p.is_empty()) {
        return Err("Invalid shortcut format".into());
    }
    Ok(())
}
