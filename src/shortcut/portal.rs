//! XDG Desktop Portal `GlobalShortcuts` engine (Wayland-native, GNOME).
//!
//! GNOME blocks raw global grabs on Wayland, so bindings go through
//! `org.freedesktop.portal.GlobalShortcuts` (via `ashpd`): the compositor
//! owns the actual key capture and delivers press events to us through the
//! portal's `Activated` signal.
//!
//! In addition to XDG Desktop Portal, Otush integrates directly with GNOME's
//! `org.gnome.settings-daemon.global-shortcuts.application` GSettings schema:
//! - Direct sync allows shortcut changes made inside Otush to be immediately
//!   registered and activated by GNOME Shell / Mutter without requiring manual
//!   intervention in GNOME Settings.
//! - Live GSettings & ShortcutsChanged watchers detect when the user modifies
//!   shortcuts in GNOME System Settings (Settings -> Apps -> Otush) and synchronize
//!   them back to Otush in real-time.

use crate::context::AppContext;
use crate::settings::ShortcutBinding;
use crate::shortcut::handler::handle_shortcut_event;
use ashpd::desktop::global_shortcuts::{
    BindShortcutsOptions, GlobalShortcuts, NewShortcut, Shortcut,
};
use ashpd::desktop::Session;
use futures_util::StreamExt;
use gio::prelude::*;
use log::{debug, info, warn};
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
/// Sync guard preventing recursive write-back signals between Otush and GSettings.
static IS_SYNCING_GSETTINGS: AtomicBool = AtomicBool::new(false);

/// The live portal proxy + session, established asynchronously.
struct PortalHandle {
    proxy: GlobalShortcuts,
    session: futures_util::lock::Mutex<Option<Session<GlobalShortcuts>>>,
    last_bound: Mutex<Option<Vec<(String, String)>>>,
}

static PORTAL: OnceLock<Arc<PortalHandle>> = OnceLock::new();

pub fn is_initialized() -> bool {
    PORTAL_READY.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn desired_map() -> Arc<Mutex<HashMap<String, ShortcutBinding>>> {
    Arc::clone(DESIRED.get_or_init(|| Arc::new(Mutex::new(HashMap::new()))))
}

pub fn trigger_rebind() {
    let _ = REBIND_TX.get().map(|tx| tx.try_send(()));
}

/// Sync desired shortcut map with AppSettings and trigger rebind on portal session.
pub fn sync_desired_bindings(settings: &crate::settings::AppSettings) {
    let desired = desired_map();
    let mut map = desired.lock().unwrap_or_else(|e| e.into_inner());
    map.clear();
    for (id, binding) in &settings.bindings {
        if id != "cancel" && !binding.current_binding.trim().is_empty() {
            if id == "transcribe_with_post_process" && !settings.post_process_enabled {
                continue;
            }
            map.insert(id.clone(), binding.clone());
        }
    }
    drop(map);
    trigger_rebind();
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

/// Convert an Otush binding string (e.g. "ctrl+shift+space") into GTK/GNOME accelerator format ("<Control><Shift>space").
pub fn otush_binding_to_accelerator(binding: &str) -> String {
    let clean_binding = if binding.to_lowercase().starts_with("press ") || binding.contains('<') {
        portal_trigger_to_otush_binding(binding).unwrap_or_else(|| binding.to_string())
    } else {
        binding.to_string()
    };
    let mut mods = Vec::new();
    let mut key = "";
    for token in clean_binding.split('+') {
        let t = token.trim();
        match t.to_lowercase().as_str() {
            "ctrl" | "control" => mods.push("<Control>"),
            "alt" | "opt" | "option" => mods.push("<Alt>"),
            "shift" => mods.push("<Shift>"),
            "super" | "win" | "meta" | "cmd" | "command" => mods.push("<Super>"),
            _ => key = t,
        }
    }
    let key_lower = key.to_lowercase();
    let key_token = match key_lower.as_str() {
        "`" | "grave" | "dead_grave" => "dead_grave",
        "~" | "asciitilde" | "dead_tilde" => "dead_tilde",
        "'" | "dead_acute" => "dead_acute",
        "^" | "dead_circumflex" => "dead_circumflex",
        "space" => "space",
        "enter" | "return" => "Return",
        "escape" | "esc" => "Escape",
        "backspace" => "BackSpace",
        "tab" => "Tab",
        other => other,
    };
    format!("{}{}", mods.concat(), key_token)
}

/// Convert a GTK accelerator (e.g. "<Shift><Control>space" or "<Control>dead_grave") or portal trigger description into Otush binding format.
pub fn portal_trigger_to_otush_binding(trigger: &str) -> Option<String> {
    let mut s = trigger.trim();
    if s.is_empty() {
        return None;
    }

    // Strip case-insensitive "press " prefix if present (emitted by XDG desktop portal on GNOME)
    if let Some(stripped) = s.strip_prefix("Press ") {
        s = stripped.trim();
    } else if let Some(stripped) = s.strip_prefix("press ") {
        s = stripped.trim();
    } else if let Some(stripped) = s.strip_prefix("PRESS ") {
        s = stripped.trim();
    }

    let mut mods = Vec::new();
    let mut key_str = String::new();

    if let Some(first_bracket) = s.find('<') {
        if s.contains('>') {
            let mut rest = &s[first_bracket..];
            while rest.starts_with('<') {
                if let Some(end) = rest.find('>') {
                    let mod_name = rest[1..end].to_lowercase();
                    match mod_name.as_str() {
                        "control" | "ctrl" => mods.push("ctrl"),
                        "alt" | "option" | "opt" => mods.push("alt"),
                        "shift" => mods.push("shift"),
                        "super" | "meta" | "win" => mods.push("super"),
                        _ => {}
                    }
                    rest = rest[end + 1..].trim_start();
                } else {
                    break;
                }
            }
            key_str = rest.trim().to_string();
        }
    }

    if key_str.is_empty() {
        for t in s.split('+') {
            let t_trimmed = t.trim().to_lowercase();
            match t_trimmed.as_str() {
                "control" | "ctrl" => mods.push("ctrl"),
                "alt" | "option" | "opt" => mods.push("alt"),
                "shift" => mods.push("shift"),
                "super" | "meta" | "win" => mods.push("super"),
                other => key_str = other.to_string(),
            }
        }
    }

    if key_str.is_empty() {
        return None;
    }

    let k = key_str.to_lowercase();
    let normalized_key = match k.as_str() {
        "dead_grave" | "grave" | "asciitilde" => "`".to_string(),
        "dead_acute" | "dead_circumflex" => "'".to_string(),
        "return" | "kp_enter" | "iso_enter" => "enter".to_string(),
        "escape" | "esc" => "escape".to_string(),
        "kp_space" | "space" => "space".to_string(),
        "kp_tab" | "tab" => "tab".to_string(),
        "backspace" => "backspace".to_string(),
        other => {
            if other.starts_with("kp_") {
                other.replace("kp_", "")
            } else {
                other.to_string()
            }
        }
    };

    let mut sorted_mods = Vec::new();
    for m in &["ctrl", "alt", "shift", "super"] {
        if mods.contains(m) {
            sorted_mods.push(*m);
        }
    }
    sorted_mods.push(&normalized_key);
    Some(sorted_mods.join("+"))
}

/// Synchronize all current Otush bindings directly to GNOME Settings Daemon global-shortcuts GSettings.
pub fn sync_bindings_to_gnome_gsettings(ctx: &AppContext) {
    let schema_source = gio::SettingsSchemaSource::default();
    if let Some(source) = schema_source {
        if source
            .lookup(
                "org.gnome.settings-daemon.global-shortcuts.application",
                true,
            )
            .is_none()
        {
            return;
        }
    } else {
        return;
    }

    let gsettings = gio::Settings::with_path(
        "org.gnome.settings-daemon.global-shortcuts.application",
        "/org/gnome/settings-daemon/global-shortcuts/com.clusterat.otush/",
    );

    let settings = ctx.settings();
    let mut items = Vec::new();

    for (id, binding) in &settings.bindings {
        if id == "cancel" || binding.current_binding.trim().is_empty() {
            continue;
        }
        let accel = otush_binding_to_accelerator(&binding.current_binding);
        if accel.trim().is_empty() {
            continue;
        }
        let desc = binding.name.replace('\'', "\\'");
        items.push(format!(
            "('{}', {{'shortcuts': <['{}']>, 'description': <'{}'>}})",
            id, accel, desc
        ));
    }

    let text = format!("[{}]", items.join(", "));
    if let Ok(ty) = glib::VariantTy::new("a(sa{sv})") {
        match glib::Variant::parse(Some(ty), &text) {
            Ok(variant) => {
                IS_SYNCING_GSETTINGS.store(true, std::sync::atomic::Ordering::SeqCst);
                if let Err(e) = gsettings.set_value("shortcuts", &variant) {
                    warn!("Failed to write GNOME global shortcuts to GSettings: {e}");
                } else {
                    gsettings.apply();
                    debug!(
                        "Directly synchronized {} shortcuts to GNOME GSettings",
                        items.len()
                    );
                }
                IS_SYNCING_GSETTINGS.store(false, std::sync::atomic::Ordering::SeqCst);
            }
            Err(e) => {
                warn!("Failed to parse GVariant string for GNOME shortcuts: {e}");
            }
        }
    }
}

/// Read shortcuts from GNOME Settings Daemon global-shortcuts GSettings and synchronize into Otush.
pub fn sync_from_gnome_gsettings(ctx: &AppContext, gsettings: &gio::Settings) {
    let val = gsettings.value("shortcuts");
    let mut changed = false;
    let mut settings = ctx.settings();

    for i in 0..val.n_children() {
        let item = val.child_value(i);
        let id_var = item.child_value(0);
        let dict_var = item.child_value(1);
        if let Some(id) = id_var.str() {
            for j in 0..dict_var.n_children() {
                let entry = dict_var.child_value(j);
                let key = entry.child_value(0);
                if key.str() == Some("shortcuts") {
                    let v = entry.child_value(1);
                    if let Some(inner) = v.as_variant() {
                        if inner.n_children() > 0 {
                            let first = inner.child_value(0);
                            if let Some(accel) = first.str() {
                                if let Some(otush_combo) = portal_trigger_to_otush_binding(accel) {
                                    if let Some(b) = settings.bindings.get_mut(id) {
                                        if b.current_binding != otush_combo {
                                            info!(
                                                "Synchronized shortcut '{}' from GNOME GSettings: '{}' -> '{}'",
                                                id, b.current_binding, otush_combo
                                            );
                                            b.current_binding = otush_combo;
                                            changed = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if changed {
        crate::settings::write_settings(ctx, settings.clone());
        ctx.notify_setting_changed("bindings", serde_json::json!(&settings.bindings));
        sync_desired_bindings(&settings);
    }
}

/// Synchronize shortcuts delivered via the XDG Desktop Portal `ShortcutsChanged` signal.
pub fn sync_shortcuts_from_portal(ctx: &AppContext, shortcuts: &[Shortcut]) {
    let mut settings = ctx.settings();
    let mut changed = false;

    for s in shortcuts {
        let id = s.id();
        let trigger = s.trigger_description();
        if trigger.trim().is_empty() {
            continue;
        }
        if let Some(otush_combo) = portal_trigger_to_otush_binding(trigger) {
            if let Some(binding) = settings.bindings.get_mut(id) {
                if binding.current_binding != otush_combo {
                    info!(
                        "Syncing shortcut '{}' from portal signal: '{}' -> '{}'",
                        id, binding.current_binding, otush_combo
                    );
                    binding.current_binding = otush_combo;
                    changed = true;
                }
            }
        }
    }

    if changed {
        crate::settings::write_settings(ctx, settings.clone());
        ctx.notify_setting_changed("bindings", serde_json::json!(&settings.bindings));
        sync_desired_bindings(&settings);
    }
}

/// Heal legacy or corrupted shortcuts (e.g. ones with "press <...>" or unparsed markup) in settings.
pub fn auto_repair_bindings(ctx: &AppContext) {
    let mut settings = ctx.settings();
    let mut changed = false;

    for (id, binding) in &mut settings.bindings {
        let cur = binding.current_binding.trim();
        if cur.to_lowercase().starts_with("press ") || cur.contains('<') {
            if let Some(repaired) = portal_trigger_to_otush_binding(cur) {
                if repaired != binding.current_binding {
                    info!(
                        "Auto-repaired shortcut binding for '{}': '{}' -> '{}'",
                        id, binding.current_binding, repaired
                    );
                    binding.current_binding = repaired;
                    changed = true;
                }
            }
        }
    }

    if changed {
        crate::settings::write_settings(ctx, settings.clone());
        ctx.notify_setting_changed("bindings", serde_json::json!(&settings.bindings));
    }
}

/// Initialize the live watcher for GNOME's GSettings shortcuts storage.
fn init_gnome_settings_watcher(ctx: &AppContext) {
    let ctx_clone = ctx.clone();
    glib::MainContext::default().invoke(move || {
        let schema_source = gio::SettingsSchemaSource::default();
        if let Some(source) = schema_source {
            if source
                .lookup(
                    "org.gnome.settings-daemon.global-shortcuts.application",
                    true,
                )
                .is_none()
            {
                return;
            }
        } else {
            return;
        }

        let gsettings = gio::Settings::with_path(
            "org.gnome.settings-daemon.global-shortcuts.application",
            "/org/gnome/settings-daemon/global-shortcuts/com.clusterat.otush/",
        );

        // Initial read from GNOME GSettings
        sync_from_gnome_gsettings(&ctx_clone, &gsettings);

        // Ensure our defined shortcuts are written to GNOME GSettings
        sync_bindings_to_gnome_gsettings(&ctx_clone);

        // Live watcher on GSettings
        let ctx_watcher = ctx_clone.clone();
        gsettings.connect_changed(Some("shortcuts"), move |gs, _| {
            if IS_SYNCING_GSETTINGS.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            sync_from_gnome_gsettings(&ctx_watcher, gs);
        });

        // Retain gsettings alive for the entire app lifetime
        std::mem::forget(gsettings);
    });
}

/// Open the GNOME system shortcut configuration interface.
pub fn open_gnome_settings(_ctx: &AppContext) {
    launch_gnome_control_center();
}

fn launch_gnome_control_center() {
    std::thread::spawn(|| {
        let _ = std::process::Command::new("gnome-control-center")
            .arg("applications")
            .arg("com.clusterat.otush")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    });
}

/// Send the current desired list to the portal by updating or recreating the portal session.
async fn rebind(
    handle: &PortalHandle,
    desired: &Arc<Mutex<HashMap<String, ShortcutBinding>>>,
) -> Result<usize, String> {
    let formatted_triggers: Vec<(String, String, String)> = {
        let map = desired.lock().unwrap_or_else(|e| e.into_inner());
        let mut list: Vec<(String, String, String)> = map
            .values()
            .filter(|b| !b.current_binding.trim().is_empty() && b.id != "cancel")
            .map(|b| {
                (
                    b.id.clone(),
                    b.name.clone(),
                    format_portal_trigger(&b.current_binding),
                )
            })
            .collect();
        list.sort_by(|a, b| a.0.cmp(&b.0));
        list
    };

    let current_fingerprint: Vec<(String, String)> = formatted_triggers
        .iter()
        .map(|(id, _, trigger)| (id.clone(), trigger.clone()))
        .collect();

    let mut session_guard = handle.session.lock().await;

    // If an active session already has these exact shortcuts bound, skip redundant recreation.
    if session_guard.is_some() {
        let last = handle.last_bound.lock().unwrap_or_else(|e| e.into_inner());
        if last.as_ref() == Some(&current_fingerprint) {
            debug!("Portal shortcuts unchanged, skipping rebind");
            return Ok(current_fingerprint.len());
        }
    }

    // XDG Desktop Portal specification disallows calling BindShortcuts more than once
    // on the same session. To dynamically register updated shortcuts at runtime on Wayland,
    // we cleanly close the previous session and establish a new session.
    if let Some(old_session) = session_guard.take() {
        debug!("Closing previous portal session to rebind shortcuts");
        let _ = old_session.close().await;
    }

    let new_session = handle
        .proxy
        .create_session(Default::default())
        .await
        .map_err(|e| format!("Failed to create portal session: {e}"))?;

    let shortcuts: Vec<NewShortcut> = formatted_triggers
        .iter()
        .map(|(id, name, trigger)| {
            NewShortcut::new(id.clone(), name.clone()).preferred_trigger(Some(trigger.as_str()))
        })
        .collect();

    let request = match handle
        .proxy
        .bind_shortcuts(
            &new_session,
            &shortcuts,
            None,
            BindShortcutsOptions::default(),
        )
        .await
    {
        Ok(req) => req,
        Err(e) => {
            let _ = new_session.close().await;
            return Err(format!("Failed to bind portal shortcuts: {e}"));
        }
    };

    let count = match request.response() {
        Ok(resp) => resp.shortcuts().len(),
        Err(e) => {
            debug!("Portal bind_shortcuts response: {e}");
            0
        }
    };

    *session_guard = Some(new_session);
    *handle.last_bound.lock().unwrap_or_else(|e| e.into_inner()) = Some(current_fingerprint);
    info!("Successfully bound {count} global shortcuts on portal session");
    Ok(count)
}

/// Initialize the portal engine: establish a session, bind the current
/// shortcuts, and listen for activations and configuration changes.
pub fn init_shortcuts(ctx: &AppContext) {
    if PORTAL.get().is_some() {
        return;
    }

    // 0. Auto-repair any legacy or corrupted bindings in settings
    auto_repair_bindings(ctx);

    // 1. Initial sync and watcher for GNOME Settings Daemon global-shortcuts GSettings
    init_gnome_settings_watcher(ctx);

    // 2. Seed the desired list with current settings
    let desired = desired_map();
    {
        let settings = crate::settings::get_settings(ctx);
        let mut map = desired.lock().unwrap_or_else(|e| e.into_inner());
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
            let handle = Arc::new(PortalHandle {
                proxy,
                session: futures_util::lock::Mutex::new(None),
                last_bound: Mutex::new(None),
            });
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

                // Listen for ShortcutsChanged signal (emitted when user edits shortcuts in GNOME Settings)
                let ctx_changed = ctx.clone();
                let handle_changed = handle.clone();
                crate::runtime::spawn(async move {
                    match handle_changed.proxy.receive_shortcuts_changed().await {
                        Ok(mut stream) => {
                            while let Some(changed) = stream.next().await {
                                info!("Portal ShortcutsChanged signal received from desktop");
                                sync_shortcuts_from_portal(&ctx_changed, changed.shortcuts());
                            }
                        }
                        Err(e) => {
                            debug!("Could not listen for portal ShortcutsChanged: {e}");
                        }
                    }
                });

                // Rebind loop: react to register/unregister/cancel changes.
                while rx.recv().await.is_ok() {
                    while rx.try_recv().is_ok() {} // drain burst notifications
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
pub fn register_shortcut(ctx: &AppContext, binding: ShortcutBinding) -> Result<(), String> {
    if !binding.current_binding.trim().is_empty() && binding.id != "cancel" {
        desired_map()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(binding.id.clone(), binding);
        trigger_rebind();
        sync_bindings_to_gnome_gsettings(ctx);
    }
    Ok(())
}

/// Unregister a shortcut: remove from the desired list and re-bind.
pub fn unregister_shortcut(ctx: &AppContext, binding: ShortcutBinding) -> Result<(), String> {
    desired_map()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&binding.id);
    trigger_rebind();
    sync_bindings_to_gnome_gsettings(ctx);
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
            .unwrap_or_else(|e| e.into_inner())
            .insert("cancel".to_string(), binding);
    }
}

/// Unregister the cancel shortcut (called when recording stops).
pub fn unregister_cancel_shortcut(_ctx: &AppContext) {
    desired_map()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove("cancel");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_otush_binding_to_accelerator() {
        assert_eq!(otush_binding_to_accelerator("ctrl+space"), "<Control>space");
        assert_eq!(otush_binding_to_accelerator("alt+space"), "<Alt>space");
        assert_eq!(otush_binding_to_accelerator("super+space"), "<Super>space");
        assert_eq!(otush_binding_to_accelerator("ctrl+s"), "<Control>s");
        assert_eq!(
            otush_binding_to_accelerator("ctrl+`"),
            "<Control>dead_grave"
        );
        assert_eq!(
            otush_binding_to_accelerator("ctrl+shift+space"),
            "<Control><Shift>space"
        );
        assert_eq!(
            otush_binding_to_accelerator("ctrl+shift+d"),
            "<Control><Shift>d"
        );
    }

    #[test]
    fn test_portal_trigger_to_otush_binding() {
        assert_eq!(
            portal_trigger_to_otush_binding("<Control>space"),
            Some("ctrl+space".to_string())
        );
        assert_eq!(
            portal_trigger_to_otush_binding("<Shift><Control>space"),
            Some("ctrl+shift+space".to_string())
        );
        assert_eq!(
            portal_trigger_to_otush_binding("<Control>dead_grave"),
            Some("ctrl+`".to_string())
        );
        assert_eq!(
            portal_trigger_to_otush_binding("Ctrl + Shift + Space"),
            Some("ctrl+shift+space".to_string())
        );
        assert_eq!(
            portal_trigger_to_otush_binding("ctrl+alt+t"),
            Some("ctrl+alt+t".to_string())
        );
        assert_eq!(
            portal_trigger_to_otush_binding("<Super>space"),
            Some("super+space".to_string())
        );
        // Test recovery of portal trigger descriptions with "press " prefixes
        assert_eq!(
            portal_trigger_to_otush_binding("press <shift><control>d"),
            Some("ctrl+shift+d".to_string())
        );
        assert_eq!(
            portal_trigger_to_otush_binding("Press <Shift><Control>D"),
            Some("ctrl+shift+d".to_string())
        );
        assert_eq!(
            portal_trigger_to_otush_binding("press <control>z"),
            Some("ctrl+z".to_string())
        );
        assert_eq!(
            portal_trigger_to_otush_binding("press <control><alt>s"),
            Some("ctrl+alt+s".to_string())
        );
        assert_eq!(
            portal_trigger_to_otush_binding("press <control>dead_grave"),
            Some("ctrl+`".to_string())
        );
        assert_eq!(
            portal_trigger_to_otush_binding("<Shift><Control>d"),
            Some("ctrl+shift+d".to_string())
        );
    }

    #[test]
    fn test_accelerator_corrupted_recovery() {
        assert_eq!(
            otush_binding_to_accelerator("press <shift><control>d"),
            "<Control><Shift>d"
        );
        assert_eq!(
            otush_binding_to_accelerator("press <control>z"),
            "<Control>z"
        );
    }

    #[test]
    fn test_accelerator_roundtrip() {
        let bindings = [
            "ctrl+space",
            "ctrl+shift+space",
            "alt+space",
            "super+space",
            "ctrl+s",
            "ctrl+`",
            "ctrl+shift+d",
            "ctrl+alt+s",
        ];
        for b in bindings {
            let accel = otush_binding_to_accelerator(b);
            let roundtrip = portal_trigger_to_otush_binding(&accel);
            assert_eq!(roundtrip.as_deref(), Some(b), "Roundtrip failed for {b}");
        }
    }
}
