//! Shortcut rebinding (editor backend) (split from `shortcut/mod.rs`; same behavior, same paths).

use super::settings_keyboard::validate_shortcut_for_implementation;
use super::{handler, portal, register_shortcut, unregister_shortcut};
use crate::context::AppContext;
use crate::settings::{self, get_settings, ShortcutBinding};
use log::{error, warn};
use serde::Serialize;

/// Response of a binding change: the updated binding, or an error message.
#[derive(Serialize)]
pub struct BindingResponse {
    pub success: bool,
    pub binding: Option<ShortcutBinding>,
    pub error: Option<String>,
}

pub fn change_binding(
    ctx: &AppContext,
    id: String,
    binding: String,
) -> Result<BindingResponse, String> {
    // Reject empty bindings — every shortcut should have a value
    if binding.trim().is_empty() {
        return Err("Binding cannot be empty".to_string());
    }

    let mut settings = settings::get_settings(ctx);

    // Get the binding to modify, or create it from defaults if it doesn't exist
    let binding_to_modify = match settings.bindings.get(&id) {
        Some(binding) => binding.clone(),
        None => {
            // Try to get the default binding for this id
            let default_settings = settings::get_default_settings();
            match default_settings.bindings.get(&id) {
                Some(default_binding) => {
                    warn!(
                        "Binding '{}' not found in settings, creating from defaults",
                        id
                    );
                    default_binding.clone()
                }
                None => {
                    let error_msg = format!("Binding with id '{}' not found in defaults", id);
                    warn!("change_binding error: {}", error_msg);
                    return Ok(BindingResponse {
                        success: false,
                        binding: None,
                        error: Some(error_msg),
                    });
                }
            }
        }
    };

    // If this is the cancel binding, just update the settings and return
    // It's managed dynamically, so we don't register/unregister here
    if id == "cancel" {
        if let Some(mut b) = settings.bindings.get(&id).cloned() {
            b.current_binding = binding;
            settings.bindings.insert(id.clone(), b.clone());
            ctx.notify_setting_changed("bindings", serde_json::json!(&settings.bindings));
            settings::write_settings(ctx, settings);
            return Ok(BindingResponse {
                success: true,
                binding: Some(b),
                error: None,
            });
        }
    }

    // Unregister the existing binding
    if let Err(e) = unregister_shortcut(ctx, binding_to_modify.clone()) {
        let error_msg = format!("Failed to unregister shortcut: {}", e);
        error!("change_binding error: {}", error_msg);
    }

    // Validate the new shortcut for the current keyboard implementation
    if let Err(e) = validate_shortcut_for_implementation(&binding, settings.keyboard_implementation)
    {
        warn!("change_binding validation error: {}", e);
        restore_registration(ctx, &binding_to_modify);
        return Err(e);
    }

    // Create an updated binding
    let mut updated_binding = binding_to_modify.clone();
    updated_binding.current_binding = binding.clone();

    // Register the new binding
    if let Err(e) = register_shortcut(ctx, updated_binding.clone()) {
        let error_msg = format!("Failed to register shortcut: {}", e);
        error!("change_binding error: {}", error_msg);
        restore_registration(ctx, &binding_to_modify);
        return Ok(BindingResponse {
            success: false,
            binding: None,
            error: Some(error_msg),
        });
    }

    // Unregister and clear any conflicting binding that previously held this exact key combination
    let mut conflicting_ids = Vec::new();
    for (other_id, other_binding) in &settings.bindings {
        if other_id != &id
            && !other_binding.current_binding.trim().is_empty()
            && other_binding
                .current_binding
                .trim()
                .eq_ignore_ascii_case(binding.trim())
        {
            conflicting_ids.push(other_id.clone());
        }
    }
    for conflict_id in conflicting_ids {
        if let Some(mut conflict_binding) = settings.bindings.get(&conflict_id).cloned() {
            let _ = unregister_shortcut(ctx, conflict_binding.clone());
            conflict_binding.current_binding = String::new();
            settings
                .bindings
                .insert(conflict_id.clone(), conflict_binding.clone());
            ctx.notify_setting_changed("bindings", serde_json::json!(&conflict_binding));
        }
    }

    // Update the binding in the settings
    settings.bindings.insert(id, updated_binding.clone());

    // Save the settings
    settings::write_settings(ctx, settings.clone());
    portal::sync_bindings_to_gnome_gsettings(ctx);
    portal::sync_desired_bindings(&settings);
    ctx.notify_setting_changed("bindings", serde_json::json!(&updated_binding));

    // Return the updated binding
    Ok(BindingResponse {
        success: true,
        binding: Some(updated_binding),
        error: None,
    })
}

/// Best-effort re-register of the previous binding after a failed change,
/// so a failure leaves the user's shortcut working exactly as before.
fn restore_registration(ctx: &AppContext, binding: &ShortcutBinding) {
    if let Err(e) = register_shortcut(ctx, binding.clone()) {
        error!(
            "Failed to restore previous binding '{}' ({}): {}",
            binding.id, binding.current_binding, e
        );
    }
}

pub fn reset_binding(ctx: &AppContext, id: String) -> Result<BindingResponse, String> {
    let default_binding = settings::get_default_settings()
        .bindings
        .get(&id)
        .map(|b| b.default_binding.clone())
        .unwrap_or_default();
    // placeholder renamed below
    change_binding(ctx, id, default_binding)
}

/// Temporarily suspend global shortcut execution while the user is recording a new shortcut in
/// the UI, so no existing shortcut can fire mid-capture.
pub fn suspend_all_shortcuts(_ctx: &AppContext) {
    handler::set_shortcuts_suspended(true);
}

/// Re-enable shortcut execution after shortcut recording ends and synchronize bindings.
pub fn resume_all_shortcuts(ctx: &AppContext) {
    handler::set_shortcuts_suspended(false);
    let settings = get_settings(ctx);
    portal::sync_bindings_to_gnome_gsettings(ctx);
    portal::sync_desired_bindings(&settings);
}
