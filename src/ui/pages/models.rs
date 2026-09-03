//! Speech-to-Text settings page: consolidating local offline models,
//! hardware GPU acceleration, and external cloud STT providers into
//! a single, unified transcription suite.

use crate::commands::models as model_cmds;
use crate::context::{AppContext, AppEvent};
use crate::settings::{self, OrtAcceleratorSetting, TranscribeAcceleratorSetting};
use crate::shortcut;
use gdk4::prelude::*;
use gtk4::prelude::*;
use libadwaita::prelude::*;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

/// Build the unified Speech-to-Text preferences page.
pub fn build(ctx: &AppContext) -> gtk4::Widget {
    let page = libadwaita::PreferencesPage::new();
    page.set_title("Speech-to-Text");
    page.set_icon_name(Some("audio-speakers-symbolic"));

    // --- File Transcription group ---
    let transcribe_group = libadwaita::PreferencesGroup::new();
    transcribe_group.set_title("File Transcription");
    transcribe_group.set_description(Some(
        "Drag &amp; drop audio or video files here, or click to transcribe media files into text and subtitles.",
    ));

    let file_action_row = libadwaita::ActionRow::new();
    file_action_row.set_title("Transcribe Audio/Video File…");
    file_action_row.set_subtitle("Supports MP3, WAV, M4A, MP4, FLAC, OGG, AAC, WebM, MKV");
    file_action_row.set_activatable(true);

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

    // --- 1. Engine Mode Switch ---
    let mode_group = libadwaita::PreferencesGroup::new();
    mode_group.set_widget_name("transcription_mode_group");
    mode_group.set_title("Transcription Engine &amp; Mode");
    page.add(&mode_group);

    // --- 2. Local Models Catalog ---
    let models_group = libadwaita::PreferencesGroup::new();
    models_group.set_widget_name("models_list_group");
    models_group.set_title("Local Speech Models");
    models_group.set_description(Some(
        "Offline Whisper and Parakeet models. Smaller models are faster; larger models offer higher accuracy.",
    ));
    page.add(&models_group);

    // --- 2b. Custom Model Folder & Actions ---
    let action_group = libadwaita::PreferencesGroup::new();
    action_group.set_title("Custom Models Folder");
    action_group.set_description(Some(
        "Load custom Whisper or ONNX models placed in the Otush models directory.",
    ));

    let folder_row = libadwaita::ActionRow::new();
    folder_row.set_title("Open Models Folder");
    folder_row.set_subtitle(&ctx.paths.models_dir().to_string_lossy());

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
            let _ = model_cmds::rescan_local_models(&ctx).await;
        });
    });
    folder_row.add_suffix(&refresh_button);
    action_group.add(&folder_row);
    page.add(&action_group);

    // --- 3. Hardware & GPU Acceleration ---
    let accel_group = libadwaita::PreferencesGroup::new();
    accel_group.set_title("Hardware &amp; GPU Acceleration");
    accel_group.set_description(Some(
        "Configure compute hardware accelerators for local model inference.",
    ));
    populate_acceleration_group(ctx, &accel_group);
    page.add(&accel_group);

    // --- 4. Cloud STT Providers ---
    let providers_group = libadwaita::PreferencesGroup::new();
    providers_group.set_widget_name("transcription_providers_group");
    providers_group.set_title("Cloud Providers &amp; Fallback Chain");
    providers_group.set_description(Some(
        "External Speech-to-Text services (Deepgram, Groq, OpenAI, Gemini, Custom). Evaluated in priority order with automatic fallback.",
    ));
    page.add(&providers_group);

    let expanded_ids: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

    // Initial render
    refresh_mode_group(ctx, &mode_group);
    refresh_models_group(ctx, &models_group);
    refresh_providers_group(ctx, &providers_group, &expanded_ids);

    // Live refresh on bus events
    let mode_weak = glib::SendWeakRef::from(mode_group.downgrade());
    let models_weak = glib::SendWeakRef::from(models_group.downgrade());
    let prov_weak = glib::SendWeakRef::from(providers_group.downgrade());
    let ctx_bus = ctx.clone();

    ctx.bus.subscribe(move |event| {
        let ctx = ctx_bus.clone();
        let mode_weak = mode_weak.clone();
        let models_weak = models_weak.clone();
        let prov_weak = prov_weak.clone();
        let exp_ids = expanded_ids.clone();

        glib::MainContext::default().invoke(move || match event {
            AppEvent::ModelsUpdated
            | AppEvent::ModelDownloadFinished(_)
            | AppEvent::ModelDeleted(_)
            | AppEvent::ModelStateChanged(_) => {
                if let Some(grp) = models_weak.into_weak_ref().upgrade() {
                    refresh_models_group(&ctx, &grp);
                }
            }
            AppEvent::ModelDownloadProgress(progress) => {
                if let Some(grp) = models_weak.into_weak_ref().upgrade() {
                    update_progress(&grp, &progress);
                }
            }
            AppEvent::SettingsChanged { setting, .. } => {
                if setting == "transcription_providers_reordered" {
                    if let Some(grp) = prov_weak.into_weak_ref().upgrade() {
                        refresh_providers_group(&ctx, &grp, &exp_ids);
                    }
                } else if setting == "local_transcription_enabled" {
                    if let Some(grp) = mode_weak.into_weak_ref().upgrade() {
                        refresh_mode_group(&ctx, &grp);
                    }
                }
            }
            _ => {}
        });
    });

    page.upcast::<gtk4::Widget>()
}

fn refresh_mode_group(ctx: &AppContext, group: &libadwaita::PreferencesGroup) {
    crate::ui::pages::clear_group_rows(group);

    let settings = settings::get_settings(ctx);

    let local_switch_row = libadwaita::SwitchRow::new();
    local_switch_row.set_title("Prefer Local Transcription (Offline)");
    local_switch_row.set_subtitle(
        "When enabled and a local model is loaded, speech is transcribed locally on your device. When disabled, cloud providers below are used.",
    );
    local_switch_row.set_active(settings.local_transcription_enabled);

    let switch_ctx = ctx.clone();
    local_switch_row.connect_active_notify(move |row| {
        let _ = shortcut::toggle_local_transcription_setting(&switch_ctx, row.is_active());
    });

    group.add(&local_switch_row);
    crate::ui::pages::track_row(group, &local_switch_row);
}

fn populate_acceleration_group(ctx: &AppContext, group: &libadwaita::PreferencesGroup) {
    let settings = ctx.settings();

    // Transcribe accelerator
    let accel_row = libadwaita::ComboRow::new();
    accel_row.set_title("Transcription Accelerator");
    accel_row.set_subtitle("Compute backend for transcribe.cpp GGML models");
    let accel_labels = [("auto", "Auto"), ("cpu", "CPU"), ("gpu", "GPU (Vulkan)")];
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
    group.add(&accel_row);

    // ONNX accelerator
    let ort_row = libadwaita::ComboRow::new();
    ort_row.set_title("ONNX Accelerator (transcribe-rs)");
    ort_row.set_subtitle("Execution provider for ONNX/Parakeet models");
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
    let ctx2 = ctx.clone();
    ort_row.connect_selected_notify(move |row| {
        let accel = match row.selected() {
            1 => OrtAcceleratorSetting::Cpu,
            2 => OrtAcceleratorSetting::Cuda,
            3 => OrtAcceleratorSetting::Rocm,
            _ => OrtAcceleratorSetting::Auto,
        };
        let _ = shortcut::change_ort_accelerator_setting(&ctx2, accel);
    });
    group.add(&ort_row);
}

fn refresh_models_group(ctx: &AppContext, group: &libadwaita::PreferencesGroup) {
    crate::ui::pages::clear_group_rows(group);

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
    group.add(&status_row);
    crate::ui::pages::track_row(group, &status_row);

    if models.is_empty() {
        let row = libadwaita::ActionRow::new();
        row.set_title("No models found");
        row.set_subtitle("Check your connection and try again.");
        group.add(&row);
        crate::ui::pages::track_row(group, &row);
        return;
    }

    for model in models {
        let row = libadwaita::ExpanderRow::new();
        row.set_widget_name(&model.id);
        row.set_title(&model.name);
        row.set_subtitle(&format!("{} · {} MB", model.id, model.size_mb));
        if model.is_recommended {
            row.add_suffix(&badge("Recommended"));
        }
        if model.id == selected {
            row.add_suffix(&badge("Active"));
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
            if !is_loaded {
                let load_button = gtk4::Button::with_label("Load");
                load_button.add_css_class("suggested-action");
                load_button.set_valign(gtk4::Align::Center);
                let load_ctx = ctx.clone();
                let id = model.id.clone();
                load_button.connect_clicked(move |_| {
                    let _ = model_cmds::switch_active_model(&load_ctx, &id);
                });
                row.add_suffix(&load_button);
            }

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
                    let _ = model_cmds::delete_model(&ctx, id).await;
                });
            });
            row.add_suffix(&delete_button);
        } else {
            let download_button = gtk4::Button::with_label("Download");
            download_button.add_css_class("suggested-action");
            download_button.set_valign(gtk4::Align::Center);
            let dl_ctx = ctx.clone();
            let id = model.id.clone();
            download_button.connect_clicked(move |_| {
                let ctx = dl_ctx.clone();
                let id = id.clone();
                crate::runtime::spawn(async move {
                    let _ = crate::managers::model::download::download_model(&ctx, &id).await;
                });
            });
            row.add_suffix(&download_button);
        }

        group.add(&row);
        crate::ui::pages::track_row(group, &row);
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

fn badge(text: &str) -> gtk4::Widget {
    let label = gtk4::Label::new(Some(text));
    label.add_css_class("badge");
    label.upcast::<gtk4::Widget>()
}

fn refresh_providers_group(
    ctx: &AppContext,
    group: &libadwaita::PreferencesGroup,
    expanded_ids: &Arc<Mutex<HashSet<String>>>,
) {
    crate::ui::pages::clear_group_rows(group);

    let settings = settings::get_settings(ctx);
    let total_providers = settings.transcription_providers.len();

    for (idx, provider) in settings.transcription_providers.iter().enumerate() {
        let row = libadwaita::ExpanderRow::new();
        row.set_title(&format!("#{}: {}", idx + 1, provider.label));

        let current_model = settings
            .transcription_models
            .get(&provider.id)
            .cloned()
            .unwrap_or_else(|| provider.model.clone());

        let subtitle = if provider.allow_base_url_edit {
            format!("Model: {} • Base URL: {}", current_model, provider.base_url)
        } else {
            format!("Model: {}", current_model)
        };
        row.set_subtitle(&subtitle);

        let is_expanded = expanded_ids
            .lock()
            .map(|set| set.contains(&provider.id))
            .unwrap_or(false);
        row.set_expanded(is_expanded);

        let exp_track = expanded_ids.clone();
        let exp_pid = provider.id.clone();
        row.connect_expanded_notify(move |r| {
            if let Ok(mut set) = exp_track.lock() {
                if r.is_expanded() {
                    set.insert(exp_pid.clone());
                } else {
                    set.remove(&exp_pid);
                }
            }
        });

        // Priority Move Up button
        if idx > 0 {
            let up_btn = gtk4::Button::from_icon_name("go-up-symbolic");
            up_btn.set_valign(gtk4::Align::Center);
            up_btn.add_css_class("flat");
            up_btn.set_tooltip_text(Some("Increase priority"));
            let up_ctx = ctx.clone();
            let up_pid = provider.id.clone();
            up_btn.connect_clicked(move |_| {
                let _ = shortcut::move_transcription_provider_priority(&up_ctx, &up_pid, true);
            });
            row.add_suffix(&up_btn);
        }

        // Priority Move Down button
        if idx + 1 < total_providers {
            let down_btn = gtk4::Button::from_icon_name("go-down-symbolic");
            down_btn.set_valign(gtk4::Align::Center);
            down_btn.add_css_class("flat");
            down_btn.set_tooltip_text(Some("Decrease priority"));
            let down_ctx = ctx.clone();
            let down_pid = provider.id.clone();
            down_btn.connect_clicked(move |_| {
                let _ = shortcut::move_transcription_provider_priority(&down_ctx, &down_pid, false);
            });
            row.add_suffix(&down_btn);
        }

        // Enable / Disable switch
        let enable_switch = gtk4::Switch::new();
        enable_switch.set_active(provider.enabled);
        enable_switch.set_valign(gtk4::Align::Center);
        enable_switch.set_tooltip_text(Some("Enable/disable provider in fallback chain"));
        let en_ctx = ctx.clone();
        let en_id = provider.id.clone();
        enable_switch.connect_active_notify(move |sw| {
            let _ = shortcut::toggle_transcription_provider_enabled(
                &en_ctx,
                en_id.clone(),
                sw.is_active(),
            );
        });
        row.add_suffix(&enable_switch);

        // Inner Settings:
        // 1. API Key Row
        let api_key_row = libadwaita::PasswordEntryRow::new();
        api_key_row.set_title("API Key");
        let api_key = settings
            .transcription_api_keys
            .get(&provider.id)
            .cloned()
            .unwrap_or_default();
        api_key_row.set_text(&api_key);

        let key_ctx = ctx.clone();
        let key_id = provider.id.clone();
        api_key_row.connect_changed(move |r| {
            let _ = shortcut::change_transcription_api_key_setting(
                &key_ctx,
                key_id.clone(),
                r.text().to_string(),
            );
        });
        row.add_row(&api_key_row);

        // 2. Model Row
        let model_row = libadwaita::EntryRow::new();
        model_row.set_title("Model");
        model_row.set_text(&current_model);

        let model_ctx = ctx.clone();
        let model_id = provider.id.clone();
        model_row.connect_changed(move |r| {
            let _ = shortcut::change_transcription_model_setting(
                &model_ctx,
                model_id.clone(),
                r.text().to_string(),
            );
        });
        row.add_row(&model_row);

        // 3. Base URL Row (if editable)
        if provider.allow_base_url_edit {
            let url_row = libadwaita::EntryRow::new();
            url_row.set_title("Base URL");
            url_row.set_text(&provider.base_url);

            let url_ctx = ctx.clone();
            let url_id = provider.id.clone();
            url_row.connect_changed(move |r| {
                let _ = shortcut::change_transcription_base_url_setting(
                    &url_ctx,
                    url_id.clone(),
                    r.text().to_string(),
                );
            });
            row.add_row(&url_row);
        }

        // 4. Deepgram-specific Options
        if provider.id == "deepgram" {
            let dg_config = provider.deepgram.clone().unwrap_or_default();

            // Language Override
            let lang_override_row = libadwaita::EntryRow::new();
            lang_override_row.set_title("Language Override (e.g. pt-BR, en-US, auto)");
            lang_override_row.set_text(dg_config.language.as_deref().unwrap_or(""));
            let l_ctx = ctx.clone();
            lang_override_row.connect_changed(move |r| {
                let text = r.text().trim().to_string();
                let lang_opt = if text.is_empty() { None } else { Some(text) };
                let _ = shortcut::update_deepgram_config(&l_ctx, move |cfg| {
                    cfg.language = lang_opt;
                });
            });
            row.add_row(&lang_override_row);

            // Smart Format Switch
            let smart_format_row = libadwaita::SwitchRow::new();
            smart_format_row.set_title("Smart Formatting");
            smart_format_row
                .set_subtitle("Automatically formats dates, times, currencies, and numbers");
            smart_format_row.set_active(dg_config.smart_format);
            let sf_ctx = ctx.clone();
            smart_format_row.connect_active_notify(move |r| {
                let active = r.is_active();
                let _ = shortcut::update_deepgram_config(&sf_ctx, move |cfg| {
                    cfg.smart_format = active;
                });
            });
            row.add_row(&smart_format_row);

            // Punctuation Switch
            let punct_row = libadwaita::SwitchRow::new();
            punct_row.set_title("Punctuation");
            punct_row.set_subtitle("Add punctuation marks automatically");
            punct_row.set_active(dg_config.punctuate);
            let p_ctx = ctx.clone();
            punct_row.connect_active_notify(move |r| {
                let active = r.is_active();
                let _ = shortcut::update_deepgram_config(&p_ctx, move |cfg| {
                    cfg.punctuate = active;
                });
            });
            row.add_row(&punct_row);

            // Numerals Switch
            let numerals_row = libadwaita::SwitchRow::new();
            numerals_row.set_title("Numerals (Spoken Numbers to Digits)");
            numerals_row
                .set_subtitle("Converts spoken numbers ('quarenta e dois') into digits ('42')");
            numerals_row.set_active(dg_config.numerals);
            let num_ctx = ctx.clone();
            numerals_row.connect_active_notify(move |r| {
                let active = r.is_active();
                let _ = shortcut::update_deepgram_config(&num_ctx, move |cfg| {
                    cfg.numerals = active;
                });
            });
            row.add_row(&numerals_row);

            // Filler Words Switch
            let filler_words_row = libadwaita::SwitchRow::new();
            filler_words_row.set_title("Include Filler Words");
            filler_words_row.set_subtitle("Keep words like 'um', 'uh', 'tipo', 'é' in output");
            filler_words_row.set_active(dg_config.filler_words);
            let fw_ctx = ctx.clone();
            filler_words_row.connect_active_notify(move |r| {
                let active = r.is_active();
                let _ = shortcut::update_deepgram_config(&fw_ctx, move |cfg| {
                    cfg.filler_words = active;
                });
            });
            row.add_row(&filler_words_row);

            // Profanity Filter Switch
            let profanity_row = libadwaita::SwitchRow::new();
            profanity_row.set_title("Profanity Filter");
            profanity_row.set_subtitle("Filter and censor profane words");
            profanity_row.set_active(dg_config.profanity_filter);
            let pf_ctx = ctx.clone();
            profanity_row.connect_active_notify(move |r| {
                let active = r.is_active();
                let _ = shortcut::update_deepgram_config(&pf_ctx, move |cfg| {
                    cfg.profanity_filter = active;
                });
            });
            row.add_row(&profanity_row);
        }

        // 5. Test Connection Row
        let test_row = libadwaita::ActionRow::new();
        test_row.set_title("Connection &amp; Latency Test");
        test_row
            .set_subtitle("Send a short test audio signal to verify API key and measure latency");

        let test_btn = gtk4::Button::with_label("Test Connection");
        test_btn.set_valign(gtk4::Align::Center);
        test_btn.add_css_class("suggested-action");

        let test_ctx = ctx.clone();
        let test_id = provider.id.clone();
        let test_row_weak = glib::SendWeakRef::from(test_row.downgrade());
        let test_btn_weak = glib::SendWeakRef::from(test_btn.downgrade());

        test_btn.connect_clicked(move |_| {
            if let Some(btn) = test_btn_weak.clone().into_weak_ref().upgrade() {
                btn.set_sensitive(false);
                btn.set_label("Testing…");
            }
            if let Some(r) = test_row_weak.clone().into_weak_ref().upgrade() {
                r.set_subtitle("Connecting to provider STT endpoint…");
            }

            let t_ctx = test_ctx.clone();
            let t_id = test_id.clone();
            let t_row_weak = test_row_weak.clone();
            let t_btn_weak = test_btn_weak.clone();

            crate::runtime::spawn(async move {
                let res = shortcut::test_transcription_provider_connection(&t_ctx, t_id).await;
                glib::MainContext::default().invoke(move || {
                    if let Some(btn) = t_btn_weak.into_weak_ref().upgrade() {
                        btn.set_sensitive(true);
                        btn.set_label("Test Connection");
                    }
                    if let Some(r) = t_row_weak.into_weak_ref().upgrade() {
                        match res {
                            Ok((transcript, ms)) => {
                                let label = if transcript.is_empty() {
                                    format!("<span foreground=\"#2ec27e\">✓ Connected! Latency: {}ms</span>", ms)
                                } else {
                                    format!("<span foreground=\"#2ec27e\">✓ Connected ({}ms): \"{}\"</span>", ms, glib::markup_escape_text(&transcript))
                                };
                                r.set_subtitle(&label);
                            }
                            Err(e) => {
                                r.set_subtitle(&format!(
                                    "<span foreground=\"#e01b24\">✗ Connection failed: {}</span>",
                                    glib::markup_escape_text(&e)
                                ));
                            }
                        }
                    }
                });
            });
        });

        test_row.add_suffix(&test_btn);
        row.add_row(&test_row);

        group.add(&row);
        crate::ui::pages::track_row(group, &row);
    }
}
