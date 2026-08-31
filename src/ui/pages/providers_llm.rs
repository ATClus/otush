//! AI & Prompts settings page: consolidating AI Post-Processing configuration,
//! prompt templates management, and LLM provider fallback chains into one suite.

use crate::context::{AppContext, AppEvent};
use crate::settings;
use crate::shortcut;
use libadwaita::prelude::*;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

/// Build the unified AI & Prompts preferences page.
pub fn build(ctx: &AppContext) -> gtk4::Widget {
    let page = libadwaita::PreferencesPage::new();
    page.set_title("AI &amp; Prompts");
    page.set_icon_name(Some("starred-symbolic"));

    // --- 1. AI Master Mode & Active Template ---
    let post_process_group = libadwaita::PreferencesGroup::new();
    post_process_group.set_widget_name("post_processing_mode_group");
    post_process_group.set_title("AI Post-Processing &amp; Mode");
    post_process_group.set_description(Some(
        "Automatically refine, format, or translate transcripts using Large Language Models.",
    ));
    page.add(&post_process_group);

    // --- 2. Prompt Templates ---
    let prompts_group = libadwaita::PreferencesGroup::new();
    prompts_group.set_widget_name("prompts_templates_group");
    prompts_group.set_title("Prompt Templates");
    prompts_group.set_description(Some(
        "Manage reusable prompt templates. Use ${output} as the placeholder for transcribed or selected text.",
    ));
    page.add(&prompts_group);

    // --- 3. LLM Providers & Fallback Chain ---
    let providers_group = libadwaita::PreferencesGroup::new();
    providers_group.set_widget_name("post_processing_providers_group");
    providers_group.set_title("LLM Providers &amp; Fallback Chain");
    providers_group.set_description(Some(
        "Providers are queried in order of priority as a resilient fallback chain on rate limits or errors.",
    ));
    page.add(&providers_group);

    let expanded_providers: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

    // Initial render
    refresh_master_group(ctx, &post_process_group);
    refresh_prompts_group(ctx, &prompts_group);
    refresh_providers_group(ctx, &providers_group, &expanded_providers);

    // Live refresh on bus events
    let post_weak = glib::SendWeakRef::from(post_process_group.downgrade());
    let prompts_weak = glib::SendWeakRef::from(prompts_group.downgrade());
    let prov_weak = glib::SendWeakRef::from(providers_group.downgrade());
    let ctx_bus = ctx.clone();

    ctx.bus.subscribe(move |event| {
        let ctx = ctx_bus.clone();
        let post_weak = post_weak.clone();
        let prompts_weak = prompts_weak.clone();
        let prov_weak = prov_weak.clone();
        let exp_prov = expanded_providers.clone();

        glib::MainContext::default().invoke(move || {
            if let AppEvent::SettingsChanged { setting, .. } = event {
                if setting == "post_process_enabled" || setting == "post_process_selected_prompt_id"
                {
                    if let Some(grp) = post_weak.into_weak_ref().upgrade() {
                        refresh_master_group(&ctx, &grp);
                    }
                } else if setting == "post_process_prompts"
                    || setting == "post_process_prompts_structure"
                {
                    if let Some(grp) = prompts_weak.into_weak_ref().upgrade() {
                        refresh_prompts_group(&ctx, &grp);
                    }
                    if let Some(grp) = post_weak.into_weak_ref().upgrade() {
                        refresh_master_group(&ctx, &grp);
                    }
                } else if setting == "post_process_providers_reordered"
                    || setting == "post_process_provider_models"
                {
                    if let Some(grp) = prov_weak.into_weak_ref().upgrade() {
                        refresh_providers_group(&ctx, &grp, &exp_prov);
                    }
                }
            }
        });
    });

    page.upcast::<gtk4::Widget>()
}

fn refresh_master_group(ctx: &AppContext, group: &libadwaita::PreferencesGroup) {
    crate::ui::pages::clear_group_rows(group);

    let settings = ctx.settings();

    // Enable / Disable master switch
    let master_switch = libadwaita::SwitchRow::new();
    master_switch.set_title("Enable AI Post-Processing");
    master_switch.set_subtitle("Apply LLM transformations automatically upon speech capture");
    master_switch.set_active(settings.post_process_enabled);

    let switch_ctx = ctx.clone();
    master_switch.connect_active_notify(move |row| {
        let _ = shortcut::change_post_process_enabled_setting(&switch_ctx, row.is_active());
    });
    group.add(&master_switch);
    crate::ui::pages::track_row(group, &master_switch);

    // Default Prompt Selector
    let prompt_row = libadwaita::ComboRow::new();
    prompt_row.set_title("Active Default Prompt");
    prompt_row.set_subtitle("Default template used for speech transcription post-processing");

    let prompt_names: Vec<String> = settings
        .post_process_prompts
        .iter()
        .map(|p| p.name.clone())
        .collect();
    let prompt_ids: Vec<String> = settings
        .post_process_prompts
        .iter()
        .map(|p| p.id.clone())
        .collect();

    let model = gtk4::StringList::new(&prompt_names.iter().map(|s| s.as_str()).collect::<Vec<_>>());
    prompt_row.set_model(Some(&model));

    let selected_id = settings
        .post_process_selected_prompt_id
        .as_deref()
        .unwrap_or("default_improve_transcriptions");
    if let Some(pos) = prompt_ids.iter().position(|id| id == selected_id) {
        prompt_row.set_selected(pos as u32);
    }

    let p_ctx = ctx.clone();
    prompt_row.connect_selected_notify(move |r| {
        let idx = r.selected() as usize;
        if let Some(id) = prompt_ids.get(idx) {
            let _ = shortcut::set_post_process_selected_prompt(&p_ctx, id.clone());
        }
    });
    group.add(&prompt_row);
    crate::ui::pages::track_row(group, &prompt_row);
}

fn refresh_prompts_group(ctx: &AppContext, group: &libadwaita::PreferencesGroup) {
    crate::ui::pages::clear_group_rows(group);

    let settings = ctx.settings();
    let prompts = settings.post_process_prompts;
    let providers = settings.post_process_providers;

    let mut provider_names = vec!["Follow provider priority".to_string()];
    let mut provider_ids = vec![None];
    for p in &providers {
        if p.enabled {
            provider_names.push(p.label.clone());
            provider_ids.push(Some(p.id.clone()));
        }
    }

    for prompt in prompts {
        let row = libadwaita::ExpanderRow::new();
        row.set_widget_name(&format!("prompt-{}", prompt.id));
        row.set_title(&prompt.name);
        row.set_subtitle(&prompt.id);

        // Delete Button (only for custom prompts)
        if !prompt.id.starts_with("default_") {
            let del_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
            del_btn.add_css_class("flat");
            del_btn.set_tooltip_text(Some("Delete Custom Prompt"));
            del_btn.set_valign(gtk4::Align::Center);
            let del_ctx = ctx.clone();
            let del_id = prompt.id.clone();
            del_btn.connect_clicked(move |_| {
                let _ = shortcut::remove_post_process_prompt(&del_ctx, del_id.clone());
            });
            row.add_suffix(&del_btn);
        }

        // 1. Name Row
        let name_row = libadwaita::EntryRow::new();
        name_row.set_title("Prompt Name");
        name_row.set_text(&prompt.name);
        let n_ctx = ctx.clone();
        let n_id = prompt.id.clone();
        let n_current_prompt = prompt.prompt.clone();
        name_row.connect_changed(move |r| {
            let _ = shortcut::update_post_process_prompt(
                &n_ctx,
                n_id.clone(),
                r.text().to_string(),
                n_current_prompt.clone(),
            );
        });
        row.add_row(&name_row);

        // 2. Preferred Provider Row
        let pref_prov_row = libadwaita::ComboRow::new();
        pref_prov_row.set_title("Preferred Provider");
        let prov_list = gtk4::StringList::new(
            &provider_names
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>(),
        );
        pref_prov_row.set_model(Some(&prov_list));

        let current_pref_idx = provider_ids
            .iter()
            .position(|id| *id == prompt.preferred_provider_id)
            .unwrap_or(0);
        pref_prov_row.set_selected(current_pref_idx as u32);

        let pref_ctx = ctx.clone();
        let pref_prompt_id = prompt.id.clone();
        let pref_ids = provider_ids.clone();
        pref_prov_row.connect_selected_notify(move |r| {
            let idx = r.selected() as usize;
            if let Some(target_pref) = pref_ids.get(idx) {
                let _ = shortcut::set_post_process_prompt_preferred_provider(
                    &pref_ctx,
                    pref_prompt_id.clone(),
                    target_pref.clone(),
                );
            }
        });
        row.add_row(&pref_prov_row);

        // 3. Prompt Template Row
        let template_row = libadwaita::PreferencesRow::new();
        let template_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        template_box.set_margin_top(8);
        template_box.set_margin_bottom(8);
        template_box.set_margin_start(12);
        template_box.set_margin_end(12);

        let title_label = gtk4::Label::new(Some(
            "Template Prompt Content (${output} is replaced by the input text):",
        ));
        title_label.set_halign(gtk4::Align::Start);
        title_label.add_css_class("dim-label");
        title_label.add_css_class("caption");
        template_box.append(&title_label);

        let text_view = gtk4::TextView::new();
        text_view.set_wrap_mode(gtk4::WrapMode::WordChar);
        text_view.set_monospace(true);
        text_view.set_top_margin(8);
        text_view.set_bottom_margin(8);
        text_view.set_left_margin(8);
        text_view.set_right_margin(8);

        let buffer = text_view.buffer();
        buffer.set_text(&prompt.prompt);

        let t_ctx = ctx.clone();
        let t_id = prompt.id.clone();
        let t_name = prompt.name.clone();
        buffer.connect_changed(move |buf| {
            let start = buf.start_iter();
            let end = buf.end_iter();
            let text = buf.text(&start, &end, false);
            let _ = shortcut::update_post_process_prompt(
                &t_ctx,
                t_id.clone(),
                t_name.clone(),
                text.to_string(),
            );
        });

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_min_content_height(100);
        scrolled.set_max_content_height(280);
        scrolled.set_child(Some(&text_view));
        scrolled.add_css_class("card");
        template_box.append(&scrolled);

        template_row.set_child(Some(&template_box));
        row.add_row(&template_row);

        group.add(&row);
        crate::ui::pages::track_row(group, &row);
    }

    // Add Custom Prompt Button
    let add_row = libadwaita::ActionRow::new();
    add_row.set_title("Create New Custom Prompt Template");
    add_row.set_subtitle("Add a custom prompt instruction with the ${output} variable");

    let add_btn = gtk4::Button::with_label("Add Prompt");
    add_btn.set_valign(gtk4::Align::Center);
    add_btn.add_css_class("suggested-action");
    let add_ctx = ctx.clone();
    add_btn.connect_clicked(move |_| {
        let _ = shortcut::add_post_process_prompt(
            &add_ctx,
            "New Prompt".to_string(),
            "Please edit this transcript:\n${output}".to_string(),
        );
    });
    add_row.add_suffix(&add_btn);
    group.add(&add_row);
    crate::ui::pages::track_row(group, &add_row);
}

fn refresh_providers_group(
    ctx: &AppContext,
    group: &libadwaita::PreferencesGroup,
    expanded_ids: &Arc<Mutex<HashSet<String>>>,
) {
    crate::ui::pages::clear_group_rows(group);

    let settings = settings::get_settings(ctx);
    let total_providers = settings.post_process_providers.len();

    for (idx, provider) in settings.post_process_providers.iter().enumerate() {
        let row = libadwaita::ExpanderRow::new();
        row.set_title(&format!("#{}: {}", idx + 1, provider.label));

        let current_model = settings
            .post_process_models
            .get(&provider.id)
            .cloned()
            .unwrap_or_default();

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
                let _ = shortcut::move_post_process_provider_priority(&up_ctx, &up_pid, true);
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
                let _ = shortcut::move_post_process_provider_priority(&down_ctx, &down_pid, false);
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
            let _ = shortcut::toggle_post_process_provider_enabled(
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
            .post_process_api_keys
            .get(&provider.id)
            .cloned()
            .unwrap_or_default();
        api_key_row.set_text(&api_key);

        let key_ctx = ctx.clone();
        let key_id = provider.id.clone();
        api_key_row.connect_changed(move |r| {
            let _ = shortcut::change_post_process_api_key_setting(
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
            let _ = shortcut::change_post_process_model_setting(
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
                let _ = shortcut::change_post_process_base_url_setting(
                    &url_ctx,
                    url_id.clone(),
                    r.text().to_string(),
                );
            });
            row.add_row(&url_row);
        }

        // 4. Test Connection Row
        let test_row = libadwaita::ActionRow::new();
        test_row.set_title("Connection Test");
        test_row.set_subtitle("Send a short test query to verify API key and model availability");

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
                r.set_subtitle("Connecting to LLM provider endpoint…");
            }

            let t_ctx = test_ctx.clone();
            let t_id = test_id.clone();
            let t_row_weak = test_row_weak.clone();
            let t_btn_weak = test_btn_weak.clone();

            crate::runtime::spawn(async move {
                let res = shortcut::test_post_process_provider_connection(&t_ctx, t_id).await;
                glib::MainContext::default().invoke(move || {
                    if let Some(btn) = t_btn_weak.into_weak_ref().upgrade() {
                        btn.set_sensitive(true);
                        btn.set_label("Test Connection");
                    }
                    if let Some(r) = t_row_weak.into_weak_ref().upgrade() {
                        match res {
                            Ok((content, ms)) => {
                                let label = if content.is_empty() {
                                    format!("<span foreground=\"#2ec27e\">✓ Connected! Response latency: {}ms</span>", ms)
                                } else {
                                    format!("<span foreground=\"#2ec27e\">✓ Connected ({}ms): \"{}\"</span>", ms, glib::markup_escape_text(&content))
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
