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

/// The portal could not be used (no app id in dev launches, unsupported
/// session, bind rejected). Fall back to the evdev-keys (evdev) engine and
/// persist the choice so later launches go straight there.
fn fallback_to_evdev(ctx: &AppContext, reason: &str) {
    warn!("{reason}; falling back to the evdev-keys shortcut engine");
    let mut settings = crate::settings::get_settings(ctx);
    if settings.keyboard_implementation == crate::settings::KeyboardImplementation::Portal {
        settings.keyboard_implementation = crate::settings::KeyboardImplementation::Evdev;
        crate::settings::write_settings(ctx, settings);
    }
    let _ = crate::shortcut::evdev::init_shortcuts(ctx);
}

fn desired_map() -> Arc<Mutex<HashMap<String, ShortcutBinding>>> {
    Arc::clone(DESIRED.get_or_init(|| Arc::new(Mutex::new(HashMap::new()))))
}

/// Send the current desired list to the portal.
async fn rebind(
    handle: &PortalHandle,
    desired: &Arc<Mutex<HashMap<String, ShortcutBinding>>>,
) -> Result<usize, String> {
    let shortcuts: Vec<NewShortcut> = {
        let map = desired.lock().unwrap();
        map.values()
            .map(|b| {
                NewShortcut::new(b.id.clone(), b.name.clone())
                    .preferred_trigger(Some(b.current_binding.as_str()))
            })
            .collect()
    };
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
    let response = request.response().map_err(|e| e.to_string())?;
    Ok(response.shortcuts().len())
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
            Err(e) => fallback_to_evdev(&ctx, &e),
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

/// Validate a shortcut string for the portal engine. Accepts the same
/// modifier+key grammar as the evdev-keys engine.
pub fn validate_shortcut(raw: &str) -> Result<(), String> {
    if raw.trim().is_empty() {
        return Err("Shortcut cannot be empty".into());
    }
    raw.parse::<evdev_keys::Hotkey>()
        .map(|_| ())
        .map_err(|e| format!("Invalid shortcut: {}", e))
}
