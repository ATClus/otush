//! Advanced settings page: VAD, paste, accelerator and miscellaneous options.

use crate::context::{AppContext, AppEvent};
use crate::settings::{
    ClipboardHandling, ModelUnloadTimeout, OrtAcceleratorSetting, PasteMethod, ShortcutBinding,
    TranscribeAcceleratorSetting, TypingTool, VadBackend,
};
use crate::shortcut;
use libadwaita::prelude::*;

/// Build the Advanced preferences page.
pub fn build(ctx: &AppContext) -> gtk4::Widget {
    let page = libadwaita::PreferencesPage::new();
    page.set_title("Advanced");

    let settings = ctx.settings();

    // --- Voice Activity Detection ---
    let vad_group = libadwaita::PreferencesGroup::new();
    vad_group.set_title("Voice Detection");

    let vad_enabled = libadwaita::SwitchRow::new();
    vad_enabled.set_title("Enable voice activity detection");
    vad_enabled.set_subtitle("Trim silence while recording");
    vad_enabled.set_active(settings.vad_enabled);
    let ctx1 = ctx.clone();
    vad_enabled.connect_active_notify(move |row| {
        let _ = shortcut::change_vad_enabled_setting(&ctx1, row.is_active());
    });
    vad_group.add(&vad_enabled);

    let vad_backend_row = libadwaita::ComboRow::new();
    vad_backend_row.set_title("VAD backend");
    let vad_labels = ["Silero", "Earshot (experimental)"];
    let model = gtk4::StringList::new(&vad_labels);
    vad_backend_row.set_model(Some(&model));
    vad_backend_row.set_selected(match settings.vad_backend {
        VadBackend::Silero => 0,
        VadBackend::Earshot => 1,
    });
    let ctx1 = ctx.clone();
    vad_backend_row.connect_selected_notify(move |row| {
        let backend = match row.selected() {
            1 => VadBackend::Earshot,
            _ => VadBackend::Silero,
        };
        let ctx = ctx1.clone();
        glib::spawn_future_local(async move {
            let _ = shortcut::change_vad_backend_setting(&ctx, backend).await;
        });
    });
    vad_group.add(&vad_backend_row);

    let filler = libadwaita::SwitchRow::new();
    filler.set_title("Remove filler words");
    filler.set_subtitle("Strip 'um', 'uh' and similar from transcripts");
    filler.set_active(settings.filler_word_removal_enabled);
    let ctx1 = ctx.clone();
    filler.connect_active_notify(move |row| {
        let _ = shortcut::change_filler_word_removal_enabled_setting(&ctx1, row.is_active());
    });
    vad_group.add(&filler);

    page.add(&vad_group);

    // --- Pasting ---
    let paste_group = libadwaita::PreferencesGroup::new();
    paste_group.set_title("Pasting");

    let paste_method_row = libadwaita::ComboRow::new();
    paste_method_row.set_title("Paste method");
    let method_labels = [
        ("ctrl_v", "Ctrl+V"),
        ("direct", "Direct (type text)"),
        ("none", "None (clipboard only)"),
        ("shift_insert", "Shift+Insert"),
        ("ctrl_shift_v", "Ctrl+Shift+V"),
        ("external_script", "External script"),
    ];
    let model = gtk4::StringList::new(
        &method_labels
            .iter()
            .map(|(_, label)| *label)
            .collect::<Vec<_>>(),
    );
    paste_method_row.set_model(Some(&model));
    if let Some(i) = method_labels
        .iter()
        .position(|(id, _)| method_to_id(settings.paste_method) == *id)
    {
        paste_method_row.set_selected(i as u32);
    }
    let ctx1 = ctx.clone();
    paste_method_row.connect_selected_notify(move |row| {
        if let Some((id, _)) = method_labels.get(row.selected() as usize) {
            let _ = shortcut::change_paste_method_setting(&ctx1, id.to_string());
        }
    });
    paste_group.add(&paste_method_row);

    let typing_tool_row = libadwaita::ComboRow::new();
    typing_tool_row.set_title("Typing tool");
    let tool_labels = [
        ("auto", "Auto"),
        ("wtype", "wtype (Wayland)"),
        ("xdotool", "xdotool (X11)"),
        ("ydotool", "ydotool"),
        ("dotool", "dotool"),
        ("kwtype", "kwtype"),
    ];
    let model = gtk4::StringList::new(
        &tool_labels
            .iter()
            .map(|(_, label)| *label)
            .collect::<Vec<_>>(),
    );
    typing_tool_row.set_model(Some(&model));
    if let Some(i) = tool_labels
        .iter()
        .position(|(id, _)| tool_to_id(settings.typing_tool) == *id)
    {
        typing_tool_row.set_selected(i as u32);
    }
    let ctx1 = ctx.clone();
    typing_tool_row.connect_selected_notify(move |row| {
        if let Some((id, _)) = tool_labels.get(row.selected() as usize) {
            let _ = shortcut::change_typing_tool_setting(&ctx1, id.to_string());
        }
    });
    paste_group.add(&typing_tool_row);

    let paste_delay_adjustment =
        gtk4::Adjustment::new(settings.paste_delay_ms as f64, 0.0, 1000.0, 10.0, 50.0, 0.0);
    let paste_delay = libadwaita::SpinRow::new(Some(&paste_delay_adjustment), 0.0, 0);
    paste_delay.set_title("Paste delay (ms)");
    paste_delay.set_snap_to_ticks(true);
    let ctx1 = ctx.clone();
    paste_delay_adjustment.connect_value_changed(move |adj| {
        let _ = shortcut::change_paste_delay_ms_setting(&ctx1, adj.value() as u64);
    });
    paste_group.add(&paste_delay);

    let clipboard_row = libadwaita::ComboRow::new();
    clipboard_row.set_title("Clipboard handling");
    let clip_labels = [
        ("dont_modify", "Don't modify"),
        ("copy_to_clipboard", "Copy to clipboard"),
    ];
    let model = gtk4::StringList::new(
        &clip_labels
            .iter()
            .map(|(_, label)| *label)
            .collect::<Vec<_>>(),
    );
    clipboard_row.set_model(Some(&model));
    if settings.clipboard_handling == ClipboardHandling::CopyToClipboard {
        clipboard_row.set_selected(1);
    }
    let ctx1 = ctx.clone();
    clipboard_row.connect_selected_notify(move |row| {
        if let Some((id, _)) = clip_labels.get(row.selected() as usize) {
            let _ = shortcut::change_clipboard_handling_setting(&ctx1, id.to_string());
        }
    });
    paste_group.add(&clipboard_row);

    page.add(&paste_group);

    // --- Model group ---
    let model_group = libadwaita::PreferencesGroup::new();
    model_group.set_title("Model");

    let unload_row = libadwaita::ComboRow::new();
    unload_row.set_title("Unload model after inactivity");
    let unload_labels = [
        ("never", "Never"),
        ("immediately", "Immediately"),
        ("min2", "2 minutes"),
        ("min5", "5 minutes"),
        ("min10", "10 minutes"),
        ("min15", "15 minutes"),
        ("hour1", "1 hour"),
    ];
    let model = gtk4::StringList::new(
        &unload_labels
            .iter()
            .map(|(_, label)| *label)
            .collect::<Vec<_>>(),
    );
    unload_row.set_model(Some(&model));
    if let Some(i) = unload_labels
        .iter()
        .position(|(id, _)| timeout_to_id(settings.model_unload_timeout) == *id)
    {
        unload_row.set_selected(i as u32);
    }
    let ctx1 = ctx.clone();
    unload_row.connect_selected_notify(move |row| {
        if let Some((id, _)) = unload_labels.get(row.selected() as usize) {
            let timeout = id_to_timeout(id);
            crate::commands::transcription::set_model_unload_timeout(&ctx1, timeout);
        }
    });
    model_group.add(&unload_row);

    let accel_row = libadwaita::ComboRow::new();
    accel_row.set_title("Transcription accelerator");
    let accel_labels = [("auto", "Auto"), ("cpu", "CPU"), ("gpu", "GPU")];
    let model = gtk4::StringList::new(
        &accel_labels
            .iter()
            .map(|(_, label)| *label)
            .collect::<Vec<_>>(),
    );
    accel_row.set_model(Some(&model));
    accel_row.set_selected(match settings.transcribe_accelerator {
        TranscribeAcceleratorSetting::Cpu => 1,
        TranscribeAcceleratorSetting::Gpu => 2,
        _ => 0,
    });
    let ctx1 = ctx.clone();
    accel_row.connect_selected_notify(move |row| {
        let accel = match row.selected() {
            1 => TranscribeAcceleratorSetting::Cpu,
            2 => TranscribeAcceleratorSetting::Gpu,
            _ => TranscribeAcceleratorSetting::Auto,
        };
        let _ = shortcut::change_transcribe_accelerator_setting(&ctx1, accel);
    });
    model_group.add(&accel_row);

    let ort_row = libadwaita::ComboRow::new();
    ort_row.set_title("ONNX accelerator (transcribe-rs)");
    let ort_labels = [
        ("auto", "Auto"),
        ("cpu", "CPU"),
        ("cuda", "CUDA"),
        ("rocm", "ROCm"),
    ];
    let model = gtk4::StringList::new(
        &ort_labels
            .iter()
            .map(|(_, label)| *label)
            .collect::<Vec<_>>(),
    );
    ort_row.set_model(Some(&model));
    ort_row.set_selected(match settings.ort_accelerator {
        OrtAcceleratorSetting::Cpu => 1,
        OrtAcceleratorSetting::Cuda => 2,
        OrtAcceleratorSetting::Rocm => 3,
        _ => 0,
    });
    let ctx1 = ctx.clone();
    ort_row.connect_selected_notify(move |row| {
        let accel = match row.selected() {
            1 => OrtAcceleratorSetting::Cpu,
            2 => OrtAcceleratorSetting::Cuda,
            3 => OrtAcceleratorSetting::Rocm,
            _ => OrtAcceleratorSetting::Auto,
        };
        let _ = shortcut::change_ort_accelerator_setting(&ctx1, accel);
    });
    model_group.add(&ort_row);

    page.add(&model_group);

    // --- Misc group ---
    let misc_group = libadwaita::PreferencesGroup::new();
    misc_group.set_title("Miscellaneous");

    add_switch(
        &misc_group,
        ctx,
        "Mute microphone while recording",
        settings.mute_while_recording,
        |c, v| {
            let _ = shortcut::change_mute_while_recording_setting(c, v);
        },
    );
    add_switch(
        &misc_group,
        ctx,
        "Append trailing space",
        settings.append_trailing_space,
        |c, v| {
            let _ = shortcut::change_append_trailing_space_setting(c, v);
        },
    );
    add_switch(
        &misc_group,
        ctx,
        "Lazy stream close",
        settings.lazy_stream_close,
        |c, v| {
            let _ = shortcut::change_lazy_stream_close_setting(c, v);
        },
    );
    add_switch(
        &misc_group,
        ctx,
        "Launch at login",
        settings.autostart_enabled,
        |c, v| {
            let _ = shortcut::change_autostart_setting(c, v);
        },
    );
    add_switch(
        &misc_group,
        ctx,
        "Check for updates",
        settings.update_checks_enabled,
        |c, v| {
            let _ = shortcut::change_update_checks_setting(c, v);
        },
    );
    add_switch(
        &misc_group,
        ctx,
        "Experimental features",
        settings.experimental_enabled,
        |c, v| {
            let _ = shortcut::change_experimental_enabled_setting(c, v);
        },
    );

    page.add(&misc_group);

    // --- Shortcuts ---
    build_shortcuts(ctx, &page);

    page.upcast::<gtk4::Widget>()
}

// ============================================================================
// Shortcut capture (record a new global shortcut by pressing keys)
// ============================================================================

/// One in-progress capture: which binding is being re-bound and the live
/// hotkey string built from the key events.
#[derive(Clone)]
struct CaptureState {
    binding_id: String,
    hotkey: String,
}

static CAPTURE: std::sync::Mutex<Option<CaptureState>> = std::sync::Mutex::new(None);

fn is_capturing() -> bool {
    CAPTURE.lock().unwrap().is_some()
}

/// The Shortcuts preferences group: one row per binding with Change/Reset
/// actions, plus a bus subscription that feeds live key events into the
/// in-progress capture.
fn build_shortcuts(ctx: &AppContext, page: &libadwaita::PreferencesPage) {
    let group = libadwaita::PreferencesGroup::new();
    group.set_widget_name("shortcuts");
    group.set_title("Shortcuts");
    group.set_description(Some(
        "Global shortcuts. Click Change, then press the new key combination.",
    ));
    page.add(&group);

    rebuild_shortcuts(ctx, &group);

    // Live key events while a capture is in progress.
    let ctx = ctx.clone();
    let group_weak = glib::SendWeakRef::from(group.downgrade());
    let bus = ctx.bus.clone();
    bus.subscribe(move |event| {
        if let AppEvent::EvdevKeysEvent(value) = event {
            let ctx = ctx.clone();
            let group_weak = group_weak.clone();
            glib::MainContext::default().invoke(move || {
                let Some(group) = group_weak.into_weak_ref().upgrade() else {
                    return;
                };
                handle_capture_event(&ctx, &group, &value);
            });
        }
    });
}

fn rebuild_shortcuts(ctx: &AppContext, group: &libadwaita::PreferencesGroup) {
    crate::ui::pages::clear_group_rows(group);

    let settings = ctx.settings();
    let mut bindings: Vec<ShortcutBinding> = settings.bindings.values().cloned().collect();
    bindings.sort_by(|a, b| a.id.cmp(&b.id));

    for binding in bindings {
        let row = libadwaita::ActionRow::new();
        row.set_widget_name(&format!("sc-{}", binding.id));
        row.set_title(&binding.name);
        row.set_subtitle(&binding.current_binding);
        row.set_activatable(false);

        let change_button = gtk4::Button::with_label("Change…");
        let reset_button = gtk4::Button::with_label("Reset");
        reset_button.add_css_class("flat");

        let ctx_start = ctx.clone();
        let id_start = binding.id.clone();
        let group_start = group.clone();
        let row_start = row.clone();
        change_button.connect_clicked(move |btn| {
            if is_capturing() {
                save_capture(&ctx_start, &group_start);
            } else {
                start_capture(&ctx_start, &group_start, &row_start, &id_start, btn);
            }
        });

        let ctx_reset = ctx.clone();
        let id_reset = binding.id.clone();
        let group_reset = group.clone();
        reset_button.connect_clicked(move |_| {
            if is_capturing() {
                cancel_capture(&ctx_reset, &group_reset);
            } else {
                let _ = shortcut::reset_binding(&ctx_reset, id_reset.clone());
                rebuild_shortcuts(&ctx_reset, &group_reset);
            }
        });

        row.add_suffix(&change_button);
        row.add_suffix(&reset_button);
        group.add(&row);
        crate::ui::pages::track_row(group, &row);
    }
}

fn find_shortcut_row(
    group: &libadwaita::PreferencesGroup,
    binding_id: &str,
) -> Option<libadwaita::ActionRow> {
    let target = format!("sc-{binding_id}");
    let mut child = group.first_child();
    while let Some(widget) = child {
        if let Some(row) = widget.downcast_ref::<libadwaita::ActionRow>() {
            if row.widget_name() == target {
                return Some(row.clone());
            }
        }
        child = widget.next_sibling();
    }
    None
}

/// Begin recording a new shortcut for `binding_id`. The Change button becomes
/// Save; the Reset button becomes Cancel.
fn start_capture(
    ctx: &AppContext,
    group: &libadwaita::PreferencesGroup,
    row: &libadwaita::ActionRow,
    binding_id: &str,
    change_button: &gtk4::Button,
) {
    if is_capturing() {
        return;
    }
    *CAPTURE.lock().unwrap() = Some(CaptureState {
        binding_id: binding_id.to_string(),
        hotkey: String::new(),
    });
    if let Err(e) = crate::shortcut::evdev::start_evdev_recording(ctx, binding_id.to_string()) {
        log::warn!("Failed to start shortcut capture: {e}");
        *CAPTURE.lock().unwrap() = None;
        return;
    }
    row.set_subtitle("Press the new shortcut…");
    change_button.set_label("Save");
    let _ = group;
}

/// Stop capturing and persist the recorded hotkey.
fn save_capture(ctx: &AppContext, group: &libadwaita::PreferencesGroup) {
    let cap = CAPTURE.lock().unwrap().take();
    let _ = crate::shortcut::evdev::stop_evdev_recording(ctx);
    if let Some(cap) = cap {
        if !cap.hotkey.trim().is_empty() {
            match shortcut::change_binding(ctx, cap.binding_id.clone(), cap.hotkey.clone()) {
                Ok(_) => log::info!("Shortcut '{}' set to {}", cap.binding_id, cap.hotkey),
                Err(e) => log::warn!(
                    "Failed to update shortcut '{}' to '{}': {e}",
                    cap.binding_id,
                    cap.hotkey
                ),
            }
        }
    }
    rebuild_shortcuts(ctx, group);
}

/// Abort the in-progress capture without saving.
fn cancel_capture(ctx: &AppContext, group: &libadwaita::PreferencesGroup) {
    *CAPTURE.lock().unwrap() = None;
    let _ = crate::shortcut::evdev::stop_evdev_recording(ctx);
    rebuild_shortcuts(ctx, group);
}

/// Update the row subtitle with the latest captured combination.
fn handle_capture_event(
    _ctx: &AppContext,
    group: &libadwaita::PreferencesGroup,
    value: &serde_json::Value,
) {
    let mut cap = match CAPTURE.lock().unwrap().clone() {
        Some(cap) => cap,
        None => return,
    };
    let is_down = value
        .get("is_key_down")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let hotkey = value
        .get("hotkey_string")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if is_down && !hotkey.is_empty() {
        cap.hotkey = hotkey.clone();
        *CAPTURE.lock().unwrap() = Some(cap.clone());
        if let Some(row) = find_shortcut_row(group, &cap.binding_id) {
            row.set_subtitle(&hotkey);
        }
    }
}

/// Helper: a switch row wired to a settings command.
fn add_switch(
    group: &libadwaita::PreferencesGroup,
    ctx: &AppContext,
    title: &str,
    value: bool,
    apply: impl Fn(&AppContext, bool) + Send + Sync + 'static,
) {
    let row = libadwaita::SwitchRow::new();
    row.set_title(title);
    row.set_active(value);
    let ctx = ctx.clone();
    row.connect_active_notify(move |row| {
        apply(&ctx, row.is_active());
    });
    group.add(&row);
}

fn method_to_id(method: PasteMethod) -> &'static str {
    match method {
        PasteMethod::CtrlV => "ctrl_v",
        PasteMethod::Direct => "direct",
        PasteMethod::None => "none",
        PasteMethod::ShiftInsert => "shift_insert",
        PasteMethod::CtrlShiftV => "ctrl_shift_v",
        PasteMethod::ExternalScript => "external_script",
    }
}

fn tool_to_id(tool: TypingTool) -> &'static str {
    match tool {
        TypingTool::Auto => "auto",
        TypingTool::Wtype => "wtype",
        TypingTool::Xdotool => "xdotool",
        TypingTool::Ydotool => "ydotool",
        TypingTool::Dotool => "dotool",
        TypingTool::Kwtype => "kwtype",
    }
}

fn timeout_to_id(timeout: ModelUnloadTimeout) -> &'static str {
    match timeout {
        ModelUnloadTimeout::Never => "never",
        ModelUnloadTimeout::Immediately => "immediately",
        ModelUnloadTimeout::Min2 => "min2",
        ModelUnloadTimeout::Min5 => "min5",
        ModelUnloadTimeout::Min10 => "min10",
        ModelUnloadTimeout::Min15 => "min15",
        ModelUnloadTimeout::Hour1 => "hour1",
        ModelUnloadTimeout::Sec15 => "sec15",
    }
}

fn id_to_timeout(id: &str) -> ModelUnloadTimeout {
    match id {
        "never" => ModelUnloadTimeout::Never,
        "immediately" => ModelUnloadTimeout::Immediately,
        "min2" => ModelUnloadTimeout::Min2,
        "min10" => ModelUnloadTimeout::Min10,
        "min15" => ModelUnloadTimeout::Min15,
        "hour1" => ModelUnloadTimeout::Hour1,
        "sec15" => ModelUnloadTimeout::Sec15,
        _ => ModelUnloadTimeout::Min5,
    }
}
