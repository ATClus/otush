//! Speech Recognition settings page: consolidating local offline models,
//! hardware GPU acceleration, memory management, custom vocabulary fine-tuning,
//! and media file transcription into a unified suite.

use crate::commands::models as model_cmds;
use crate::context::{AppContext, AppEvent};
use crate::settings::{
    self, ModelUnloadTimeout, OrtAcceleratorSetting, TranscribeAcceleratorSetting,
};
use crate::shortcut;
use gdk4::prelude::*;
use gtk4::prelude::*;
use libadwaita::prelude::*;
use std::sync::{Arc, Mutex};

/// Build the Speech Recognition preferences page.
pub fn build(ctx: &AppContext) -> gtk4::Widget {
    let page = libadwaita::PreferencesPage::new();
    page.set_title("Speech Recognition");
    page.set_icon_name(Some("audio-speakers-symbolic"));

    // --- 1. Engine & Memory Mode Group ---
    let mode_group = libadwaita::PreferencesGroup::new();
    mode_group.set_widget_name("transcription_mode_group");
    mode_group.set_title("Transcription Engine &amp; Memory");
    mode_group.set_description(Some(
        "Select between offline local models and cloud transcription, and configure VRAM/RAM memory release.",
    ));
    mode_group.set_hexpand(true);
    page.add(&mode_group);

    // --- 2. Local Models Catalog Group ---
    let models_group = libadwaita::PreferencesGroup::new();
    models_group.set_widget_name("models_list_group");
    models_group.set_title("Offline Speech Models");
    models_group.set_description(Some(
        "Local Whisper and Parakeet models. Smaller models are faster; larger models offer higher accuracy.",
    ));
    models_group.set_hexpand(true);
    page.add(&models_group);

    // --- 3. Custom Model Folder & Actions ---
    let action_group = libadwaita::PreferencesGroup::new();
    action_group.set_title("Custom Models Folder");
    action_group.set_description(Some(
        "Load custom Whisper or ONNX models placed in the Otush models directory.",
    ));
    action_group.set_hexpand(true);

    let folder_row = libadwaita::ActionRow::new();
    folder_row.set_title("Open Models Folder");
    folder_row.set_subtitle(&ctx.paths.models_dir().to_string_lossy());
    let folder_icon = gtk4::Image::from_icon_name("folder-open-symbolic");
    folder_row.add_prefix(&folder_icon);

    let open_button = gtk4::Button::from_icon_name("folder-open-symbolic");
    open_button.set_tooltip_text(Some("Open Models Folder in File Manager"));
    open_button.set_valign(gtk4::Align::Center);
    open_button.add_css_class("flat");
    let models_dir = ctx.paths.models_dir();
    open_button.connect_clicked(move |_| {
        let _ = opener::open(&models_dir);
    });
    folder_row.add_suffix(&open_button);

    let refresh_button = gtk4::Button::from_icon_name("view-refresh-symbolic");
    refresh_button.set_tooltip_text(Some("Rescan Folder for New Models"));
    refresh_button.set_valign(gtk4::Align::Center);
    refresh_button.add_css_class("flat");
    let refresh_ctx = ctx.clone();
    refresh_button.connect_clicked(move |_| {
        let ctx = refresh_ctx.clone();
        crate::runtime::spawn(async move {
            if let Err(err) = model_cmds::rescan_local_models(&ctx).await {
                ctx.report_error("rescan_local_models", err);
            }
        });
    });
    folder_row.add_suffix(&refresh_button);
    action_group.add(&folder_row);
    page.add(&action_group);

    // --- 4. Hardware & GPU Acceleration ---
    let accel_group = libadwaita::PreferencesGroup::new();
    accel_group.set_title("Hardware &amp; GPU Acceleration");
    accel_group.set_description(Some(
        "Configure compute hardware accelerators for local model inference.",
    ));
    accel_group.set_hexpand(true);
    populate_acceleration_group(ctx, &accel_group);
    page.add(&accel_group);

    // --- 5. Vocabulary & Recognition Fine-Tuning ---
    let vocab_group = libadwaita::PreferencesGroup::new();
    vocab_group.set_title("Vocabulary &amp; Recognition Tuning");
    vocab_group.set_description(Some(
        "Vocabulary adaptation, custom technical jargon, and fuzzy replacement sensitivity.",
    ));
    vocab_group.set_hexpand(true);
    populate_vocab_group(ctx, &vocab_group);
    page.add(&vocab_group);

    // --- 6. Media File Transcription Tool ---
    let transcribe_group = libadwaita::PreferencesGroup::new();
    transcribe_group.set_title("File Transcription");
    transcribe_group.set_description(Some(
        "Transcribe pre-recorded audio or video files into text and subtitles.",
    ));
    transcribe_group.set_hexpand(true);

    let file_action_row = libadwaita::ActionRow::new();
    file_action_row.set_title("Transcribe Audio/Video File…");
    file_action_row.set_subtitle("Supports MP3, WAV, M4A, MP4, FLAC, OGG, AAC, WebM, MKV");
    file_action_row.set_activatable(true);

    let file_icon = gtk4::Image::from_icon_name("document-open-symbolic");
    file_action_row.add_prefix(&file_icon);

    let upload_btn = gtk4::Button::from_icon_name("document-open-symbolic");
    upload_btn.set_tooltip_text(Some("Open Media File Transcriber"));
    upload_btn.set_valign(gtk4::Align::Center);
    upload_btn.add_css_class("flat");
    file_action_row.add_suffix(&upload_btn);

    let ctx_dialog = ctx.clone();
    file_action_row.connect_activated(move |_| {
        crate::ui::file_transcription::show_file_transcription_dialog(&ctx_dialog, None);
    });

    let ctx_btn = ctx.clone();
    upload_btn.connect_clicked(move |_| {
        crate::ui::file_transcription::show_file_transcription_dialog(&ctx_btn, None);
    });

    transcribe_group.add(&file_action_row);
    page.add(&transcribe_group);

    // Drop target for drag-and-dropping files on the Speech-to-Text page
    let drop_target = gtk4::DropTarget::new(gio::File::static_type(), gdk4::DragAction::COPY);
    let ctx_drop = ctx.clone();
    drop_target.connect_drop(move |_, value, _, _| {
        if let Ok(file) = value.get::<gio::File>() {
            if let Some(path) = file.path() {
                crate::ui::file_transcription::show_file_transcription_dialog(
                    &ctx_drop,
                    Some(path),
                );
                return true;
            }
        }
        false
    });
    page.add_controller(drop_target);

    // Owned row lists, one per rebuilt group (see `PageGroup`).
    let mode_rows = Arc::new(Mutex::new(crate::ui::pages::PageGroup::new()));
    let models_rows = Arc::new(Mutex::new(crate::ui::pages::PageGroup::new()));

    // Initial render
    refresh_mode_group(ctx, &mode_group, &mode_rows);
    refresh_models_group(ctx, &models_group, &models_rows);

    // Live refresh on bus events
    let mode_weak = glib::SendWeakRef::from(mode_group.downgrade());
    let models_weak = glib::SendWeakRef::from(models_group.downgrade());
    let ctx_bus = ctx.clone();
    let mode_rows_bus = mode_rows.clone();
    let models_rows_bus = models_rows.clone();

    ctx.bus.subscribe(move |event| {
        let ctx = ctx_bus.clone();
        let mode_weak = mode_weak.clone();
        let models_weak = models_weak.clone();
        let mode_rows = mode_rows_bus.clone();
        let models_rows = models_rows_bus.clone();

        glib::MainContext::default().invoke(move || match event {
            AppEvent::ModelsUpdated
            | AppEvent::ModelDownloadFinished(_)
            | AppEvent::ModelDeleted(_)
            | AppEvent::ModelStateChanged(_) => {
                if let Some(grp) = models_weak.into_weak_ref().upgrade() {
                    refresh_models_group(&ctx, &grp, &models_rows);
                }
            }
            AppEvent::ModelDownloadProgress(progress) => {
                if let Some(grp) = models_weak.into_weak_ref().upgrade() {
                    update_progress(&grp, &progress);
                }
            }
            AppEvent::SettingsChanged { setting, .. }
                if setting == "local_transcription_enabled"
                    || setting == "model_unload_timeout" =>
            {
                if let Some(grp) = mode_weak.into_weak_ref().upgrade() {
                    refresh_mode_group(&ctx, &grp, &mode_rows);
                }
            }
            _ => {}
        });
    });

    page.upcast::<gtk4::Widget>()
}

fn refresh_mode_group(
    ctx: &AppContext,
    group: &libadwaita::PreferencesGroup,
    rows: &Arc<Mutex<crate::ui::pages::PageGroup>>,
) {
    rows.lock().unwrap_or_else(|e| e.into_inner()).clear(group);

    let settings = settings::get_settings(ctx);

    // Prefer Local Transcription switch
    let local_switch_row = libadwaita::SwitchRow::new();
    local_switch_row.set_title("Prefer Local Transcription (Offline)");
    local_switch_row.set_subtitle(
        "When enabled and a local model is loaded, speech is transcribed locally on your device. When disabled, cloud STT is used.",
    );
    let local_icon = gtk4::Image::from_icon_name("audio-speakers-symbolic");
    local_switch_row.add_prefix(&local_icon);
    local_switch_row.set_active(settings.local_transcription_enabled);

    // Unload Model After Inactivity
    let unload_row = libadwaita::ComboRow::new();
    unload_row.set_title("Unload Model After Inactivity");
    unload_row
        .set_subtitle("Release VRAM and system memory when no recordings are made for a period");
    let unload_icon = gtk4::Image::from_icon_name("preferences-system-time-symbolic");
    unload_row.add_prefix(&unload_icon);
    unload_row.set_visible(settings.local_transcription_enabled);

    let unload_labels = [
        ("never", "Never (Keep in Memory)"),
        ("immediately", "Immediately"),
        ("sec15", "15 seconds"),
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
    let unload_ctx = ctx.clone();
    unload_row.connect_selected_notify(move |row| {
        if let Some((id, _)) = unload_labels.get(row.selected() as usize) {
            let timeout = id_to_timeout(id);
            crate::commands::transcription::set_model_unload_timeout(&unload_ctx, timeout);
        }
    });

    let switch_ctx = ctx.clone();
    let unload_weak = glib::SendWeakRef::from(unload_row.downgrade());
    local_switch_row.connect_active_notify(move |row| {
        let is_active = row.is_active();
        if let Err(err) = shortcut::toggle_local_transcription_setting(&switch_ctx, is_active) {
            switch_ctx.report_error("toggle_local_transcription_setting", err);
        }
        if let Some(ur) = unload_weak.clone().into_weak_ref().upgrade() {
            ur.set_visible(is_active);
        }
    });
    rows.lock()
        .unwrap_or_else(|e| e.into_inner())
        .add(group, &local_switch_row);
    rows.lock()
        .unwrap_or_else(|e| e.into_inner())
        .add(group, &unload_row);
}

fn populate_acceleration_group(ctx: &AppContext, group: &libadwaita::PreferencesGroup) {
    let settings = ctx.settings();
    // Real detection snapshot: compiled-in ORT providers, static transcribe
    // choices, and probed Vulkan GPU devices (cached process-wide).
    let detected = crate::managers::transcription::get_available_accelerators();

    // Transcribe accelerator: Auto/CPU always; GPU only when a real Vulkan
    // device was probed, so GPU cannot be selected on GPU-less machines.
    let accel_row = libadwaita::ComboRow::new();
    accel_row.set_title("Transcription Accelerator");
    accel_row.set_subtitle("Compute backend for transcribe.cpp GGML models");
    let accel_icon = gtk4::Image::from_icon_name("video-display-symbolic");
    accel_row.add_prefix(&accel_icon);
    let mut accel_ids: Vec<&str> = vec!["auto", "cpu"];
    let mut accel_labels: Vec<String> = vec!["Auto".to_string(), "CPU".to_string()];
    let gpu_detected = !detected.gpu_devices.is_empty();
    if gpu_detected {
        accel_ids.push("gpu");
        accel_labels.push("GPU (Vulkan)".to_string());
    }
    let saved_accel_id = match settings.transcribe_accelerator {
        TranscribeAcceleratorSetting::Cpu => "cpu",
        TranscribeAcceleratorSetting::Gpu => "gpu",
        _ => "auto",
    };
    // A saved GPU choice on a now-GPU-less machine stays visible with a
    // warning instead of silently switching the semantic.
    let saved_accel_missing = saved_accel_id == "gpu" && !gpu_detected;
    if saved_accel_missing {
        accel_ids.push("gpu");
        accel_labels.push("GPU (Vulkan, no device detected)".to_string());
        accel_row.set_subtitle("Saved GPU backend has no device on this machine");
    }
    let accel_label_refs: Vec<&str> = accel_labels.iter().map(String::as_str).collect();
    accel_row.set_model(Some(&gtk4::StringList::new(&accel_label_refs)));
    accel_row.set_selected(
        accel_ids
            .iter()
            .position(|id| *id == saved_accel_id)
            .unwrap_or(0) as u32,
    );
    let ctx1 = ctx.clone();
    accel_row.connect_selected_notify(move |row| {
        let accel = match accel_ids.get(row.selected() as usize) {
            Some(&"cpu") => TranscribeAcceleratorSetting::Cpu,
            Some(&"gpu") => TranscribeAcceleratorSetting::Gpu,
            _ => TranscribeAcceleratorSetting::Auto,
        };
        if let Err(err) = shortcut::change_transcribe_accelerator_setting(&ctx1, accel) {
            ctx1.report_error("change_transcribe_accelerator_setting", err);
        }
    });
    group.add(&accel_row);

    // GPU device picker: visible only when the GPU backend is selected and
    // more than one device exists (a single device needs no choice).
    if detected.gpu_devices.len() > 1 {
        let gpu_row = libadwaita::ComboRow::new();
        gpu_row.set_title("GPU Device");
        gpu_row.set_subtitle("Vulkan device for GPU transcription");
        let gpu_icon = gtk4::Image::from_icon_name("video-display-symbolic");
        gpu_row.add_prefix(&gpu_icon);
        let mut device_ids: Vec<String> = vec!["auto".to_string()];
        let mut device_labels: Vec<String> = vec!["Automatic".to_string()];
        for device in &detected.gpu_devices {
            device_ids.push(device.id.clone());
            device_labels.push(if device.total_vram_mb > 0 {
                format!(
                    "{} ({:.1} GiB)",
                    device.name,
                    device.total_vram_mb as f64 / 1024.0
                )
            } else {
                device.name.clone()
            });
        }
        let saved_device = settings.transcribe_gpu_device.clone().unwrap_or_default();
        let saved_device_missing = !saved_device.is_empty() && !device_ids.contains(&saved_device);
        if saved_device_missing {
            device_ids.push(saved_device.clone());
            device_labels.push(format!("{saved_device} (not detected)"));
        }
        let device_label_refs: Vec<&str> = device_labels.iter().map(String::as_str).collect();
        gpu_row.set_model(Some(&gtk4::StringList::new(&device_label_refs)));
        let saved_device_id = if saved_device.is_empty() {
            "auto".to_string()
        } else {
            saved_device
        };
        gpu_row.set_selected(
            device_ids
                .iter()
                .position(|id| *id == saved_device_id)
                .unwrap_or(0) as u32,
        );
        gpu_row.set_visible(matches!(
            settings.transcribe_accelerator,
            TranscribeAcceleratorSetting::Gpu
        ));
        let gpu_ctx = ctx.clone();
        gpu_row.connect_selected_notify(move |row| {
            let id = device_ids
                .get(row.selected() as usize)
                .cloned()
                .unwrap_or_default();
            let mut s = gpu_ctx.settings();
            s.transcribe_gpu_device = if id == "auto" { None } else { Some(id.clone()) };
            gpu_ctx.write_settings(&s);
            gpu_ctx.notify_setting_changed(
                "transcribe_gpu_device",
                serde_json::json!(s.transcribe_gpu_device),
            );
        });
        group.add(&gpu_row);
    }

    // ONNX accelerator: Auto/CPU always plus exactly the providers compiled
    // into this binary, so CUDA/ROCm cannot be selected when unsupported.
    let ort_row = libadwaita::ComboRow::new();
    ort_row.set_title("ONNX Accelerator (transcribe-rs)");
    ort_row.set_subtitle("Execution provider for ONNX/Parakeet models");
    let ort_icon = gtk4::Image::from_icon_name("preferences-system-symbolic");
    ort_row.add_prefix(&ort_icon);
    fn ort_entry(id: &str) -> Option<(&str, OrtAcceleratorSetting)> {
        match id {
            "auto" => Some(("Auto", OrtAcceleratorSetting::Auto)),
            "cpu" => Some(("CPU", OrtAcceleratorSetting::Cpu)),
            "cuda" => Some(("CUDA", OrtAcceleratorSetting::Cuda)),
            "directml" => Some(("DirectML", OrtAcceleratorSetting::DirectMl)),
            "rocm" => Some(("ROCm", OrtAcceleratorSetting::Rocm)),
            _ => None,
        }
    }
    let mut ort_ids: Vec<String> = vec!["auto".to_string(), "cpu".to_string()];
    let mut ort_labels: Vec<String> = vec!["Auto".to_string(), "CPU".to_string()];
    for id in &detected.ort {
        if id == "auto" || id == "cpu" {
            continue;
        }
        if let Some((label, _)) = ort_entry(id) {
            ort_ids.push(id.clone());
            ort_labels.push(label.to_string());
        }
    }
    let saved_ort_id = match settings.ort_accelerator {
        OrtAcceleratorSetting::Cpu => "cpu",
        OrtAcceleratorSetting::Cuda => "cuda",
        OrtAcceleratorSetting::DirectMl => "directml",
        OrtAcceleratorSetting::Rocm => "rocm",
        _ => "auto",
    };
    let saved_ort_missing = !ort_ids.iter().any(|id| id == saved_ort_id);
    if saved_ort_missing {
        ort_ids.push(saved_ort_id.to_string());
        let label = ort_entry(saved_ort_id).map(|(l, _)| l).unwrap_or("Unknown");
        ort_labels.push(format!("{label} (not compiled in)"));
        ort_row.set_subtitle("Saved provider is not compiled into this build");
    }
    let ort_label_refs: Vec<&str> = ort_labels.iter().map(String::as_str).collect();
    ort_row.set_model(Some(&gtk4::StringList::new(&ort_label_refs)));
    ort_row.set_selected(
        ort_ids
            .iter()
            .position(|id| id == saved_ort_id)
            .unwrap_or(0) as u32,
    );
    let ctx2 = ctx.clone();
    ort_row.connect_selected_notify(move |row| {
        let accel = ort_ids
            .get(row.selected() as usize)
            .and_then(|id| ort_entry(id).map(|(_, setting)| setting))
            .unwrap_or(OrtAcceleratorSetting::Auto);
        if let Err(err) = shortcut::change_ort_accelerator_setting(&ctx2, accel) {
            ctx2.report_error("change_ort_accelerator_setting", err);
        }
    });
    group.add(&ort_row);
}

fn populate_vocab_group(ctx: &AppContext, group: &libadwaita::PreferencesGroup) {
    let settings = ctx.settings();

    // Custom Vocabulary
    let custom_words_row = libadwaita::EntryRow::new();
    custom_words_row.set_title("Custom Vocabulary / Jargon (Comma-Separated)");
    let vocab_icon = gtk4::Image::from_icon_name("accessories-dictionary-symbolic");
    custom_words_row.add_prefix(&vocab_icon);
    custom_words_row.set_text(&settings.custom_words.join(", "));
    let cw_ctx = ctx.clone();
    custom_words_row.connect_changed(move |r| {
        let words: Vec<String> = r
            .text()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let mut s = cw_ctx.settings();
        s.custom_words = words;
        cw_ctx.write_settings(&s);
    });
    group.add(&custom_words_row);

    // Word Correction Fuzzy Threshold
    let word_adj = gtk4::Adjustment::new(
        settings.word_correction_threshold * 100.0,
        0.0,
        100.0,
        1.0,
        5.0,
        0.0,
    );
    let word_row = libadwaita::SpinRow::new(Some(&word_adj), 1.0, 0);
    word_row.set_title("Word Correction Fuzzy Threshold (%)");
    word_row.set_subtitle("Fuzzy matching sensitivity for vocabulary substitution (0 = disabled)");
    let word_icon = gtk4::Image::from_icon_name("edit-find-replace-symbolic");
    word_row.add_prefix(&word_icon);
    word_row.set_snap_to_ticks(true);
    let word_ctx = ctx.clone();
    word_adj.connect_value_changed(move |adj| {
        let mut s = word_ctx.settings();
        s.word_correction_threshold = adj.value() / 100.0;
        word_ctx.write_settings(&s);
    });
    group.add(&word_row);
}

fn refresh_models_group(
    ctx: &AppContext,
    group: &libadwaita::PreferencesGroup,
    rows: &Arc<Mutex<crate::ui::pages::PageGroup>>,
) {
    rows.lock().unwrap_or_else(|e| e.into_inner()).clear(group);

    let settings = ctx.settings();
    let selected = settings.selected_model;
    let loaded_model = ctx.transcription.get_current_model();
    let models = ctx.model.get_available_models();

    // Loaded-model status row
    let status_row = libadwaita::ActionRow::new();
    status_row.set_widget_name("model-status");
    match &loaded_model {
        Some(id) => {
            status_row.set_title("Loaded model in memory");
            status_row.set_subtitle(id);
            let unload_button = gtk4::Button::with_label("Unload");
            unload_button.add_css_class("flat");
            unload_button.set_valign(gtk4::Align::Center);
            let unload_ctx = ctx.clone();
            unload_button.connect_clicked(move |_| {
                let _ = crate::commands::transcription::unload_model_manually(&unload_ctx);
            });
            status_row.add_suffix(&unload_button);
        }
        None => {
            status_row.set_title("No model loaded in memory");
            status_row.set_subtitle("A model loads automatically when recording starts");
        }
    }
    rows.lock()
        .unwrap_or_else(|e| e.into_inner())
        .add(group, &status_row);

    if models.is_empty() {
        let row = libadwaita::ActionRow::new();
        row.set_title("No models found");
        row.set_subtitle("Check your connection and try again.");
        rows.lock()
            .unwrap_or_else(|e| e.into_inner())
            .add(group, &row);
        return;
    }

    for model in models {
        let row = libadwaita::ExpanderRow::new();
        row.set_widget_name(&model.id);
        row.set_title(&model.name);
        row.set_subtitle(&format!("{} · {} MB", model.id, model.size_mb));

        // Prefix on the left: Active model indicator vs standard audio icon
        let prefix_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        prefix_box.set_size_request(80, -1);
        prefix_box.set_valign(gtk4::Align::Center);

        if model.id == selected {
            let active_icon = gtk4::Image::from_icon_name("object-select-symbolic");
            active_icon.set_pixel_size(16);
            active_icon.add_css_class("accent");
            prefix_box.append(&active_icon);

            let active_badge = badge("Active");
            active_badge.add_css_class("accent");
            prefix_box.append(&active_badge);
        } else {
            let inactive_icon = gtk4::Image::from_icon_name("audio-speakers-symbolic");
            inactive_icon.set_pixel_size(16);
            inactive_icon.add_css_class("dim-label");
            prefix_box.append(&inactive_icon);
        }
        row.add_prefix(&prefix_box);

        if model.is_recommended {
            row.add_suffix(&badge("Recommended"));
        }
        row.set_expanded(false);

        // Status line
        let is_loaded = loaded_model.as_deref() == Some(model.id.as_str());
        let status = gtk4::Label::new(Some(
            match (is_loaded, model.is_downloaded, model.is_downloading) {
                (true, _, _) => "Loaded",
                (false, true, _) => "Installed",
                (false, false, true) => "Downloading…",
                (false, false, false) => "Not installed",
            },
        ));
        status.add_css_class("dim-label");
        row.add_row(&status);

        // Progress bar while downloading
        if model.is_downloading {
            let bar = gtk4::ProgressBar::new();
            let total = model.size_mb.saturating_mul(1024 * 1024) as f64;
            let fraction = if total > 0.0 {
                (model.partial_size as f64 / total).clamp(0.0, 1.0)
            } else {
                0.0
            };
            bar.set_fraction(fraction);
            bar.set_show_text(false);
            row.add_row(&bar);
        }

        // Actions
        let ctx = ctx.clone();
        if model.is_downloaded {
            let is_loaded = loaded_model.as_deref() == Some(model.id.as_str());

            // Action slot with fixed width to preserve delete icon column alignment
            let action_slot = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            action_slot.set_size_request(85, -1);
            action_slot.set_valign(gtk4::Align::Center);

            if !is_loaded {
                let load_button = gtk4::Button::new();
                let load_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
                let load_icon = gtk4::Image::from_icon_name("media-playback-start-symbolic");
                load_icon.set_pixel_size(14);
                load_box.append(&load_icon);
                load_box.append(&gtk4::Label::new(Some("Load")));
                load_button.set_child(Some(&load_box));
                load_button.add_css_class("suggested-action");
                load_button.set_valign(gtk4::Align::Center);
                load_button.set_tooltip_text(Some("Load model into memory"));
                let load_ctx = ctx.clone();
                let id = model.id.clone();
                load_button.connect_clicked(move |_| {
                    if let Err(err) = model_cmds::switch_active_model(&load_ctx, &id) {
                        load_ctx.report_error("switch_active_model", err);
                    }
                });
                action_slot.append(&load_button);
            }
            row.add_suffix(&action_slot);

            // Standardized trash icon delete button
            let delete_button = gtk4::Button::from_icon_name("user-trash-symbolic");
            delete_button.set_tooltip_text(Some("Delete Model from Disk"));
            delete_button.set_valign(gtk4::Align::Center);
            delete_button.add_css_class("flat");
            let delete_ctx = ctx.clone();
            let id = model.id.clone();
            delete_button.connect_clicked(move |_| {
                let ctx = delete_ctx.clone();
                let id = id.clone();
                crate::runtime::spawn(async move {
                    if let Err(err) = model_cmds::delete_model(&ctx, id).await {
                        ctx.report_error("delete_model", err);
                    }
                });
            });
            row.add_suffix(&delete_button);
        } else {
            let action_slot = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            action_slot.set_size_request(115, -1);
            action_slot.set_valign(gtk4::Align::Center);

            let download_button = gtk4::Button::new();
            let dl_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
            if model.is_downloading {
                let cancel_icon = gtk4::Image::from_icon_name("process-stop-symbolic");
                cancel_icon.set_pixel_size(14);
                dl_box.append(&cancel_icon);
                dl_box.append(&gtk4::Label::new(Some("Cancel")));
                download_button.set_child(Some(&dl_box));
                download_button.set_valign(gtk4::Align::Center);
                download_button.set_tooltip_text(Some("Cancel model download"));
                download_button.connect_clicked(move |_| {
                    crate::managers::model::download::cancel_download();
                });
            } else {
                let dl_icon = gtk4::Image::from_icon_name("folder-download-symbolic");
                dl_icon.set_pixel_size(14);
                dl_box.append(&dl_icon);
                dl_box.append(&gtk4::Label::new(Some("Download")));
                download_button.set_child(Some(&dl_box));
                download_button.add_css_class("suggested-action");
                download_button.set_valign(gtk4::Align::Center);
                download_button.set_tooltip_text(Some("Download model"));
                let dl_ctx = ctx.clone();
                let id = model.id.clone();
                download_button.connect_clicked(move |_| {
                    let ctx = dl_ctx.clone();
                    let id = id.clone();
                    crate::runtime::spawn(async move {
                        if let Err(err) =
                            crate::managers::model::download::download_model(&ctx, &id).await
                        {
                            ctx.report_error("download_model", err);
                        }
                    });
                });
            }
            action_slot.append(&download_button);
            row.add_suffix(&action_slot);
        }

        rows.lock()
            .unwrap_or_else(|e| e.into_inner())
            .add(group, &row);
    }
}

fn update_progress(
    group: &libadwaita::PreferencesGroup,
    progress: &crate::context::ModelDownloadProgressEvent,
) {
    let fraction = (progress.percentage / 100.0).clamp(0.0, 1.0);
    let mut child = group.first_child();
    while let Some(widget) = child {
        if let Some(row) = widget.downcast_ref::<libadwaita::ExpanderRow>() {
            let mut row_child = row.first_child();
            while let Some(rc) = row_child {
                if let Some(bar) = rc.downcast_ref::<gtk4::ProgressBar>() {
                    bar.set_fraction(fraction);
                }
                row_child = rc.next_sibling();
            }
        }
        child = widget.next_sibling();
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

fn badge(text: &str) -> gtk4::Widget {
    let label = gtk4::Label::new(Some(text));
    label.add_css_class("badge");
    label.upcast::<gtk4::Widget>()
}
