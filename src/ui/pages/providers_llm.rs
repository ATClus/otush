//! AI & Prompts settings page: AI Post-Processing configuration,
//! prompt templates management, and custom LLM prompt workflows.

use crate::context::{AppContext, AppEvent};
use crate::shortcut;
use gtk4::prelude::*;
use libadwaita::prelude::*;
use std::sync::{Arc, Mutex};

/// Build the AI & Prompts preferences page.
pub fn build(ctx: &AppContext) -> gtk4::Widget {
    let page = libadwaita::PreferencesPage::new();
    page.set_title("AI &amp; Prompts");
    page.set_icon_name(Some("starred-symbolic"));

    // --- 1. AI Master Mode & Active Template ---
    let post_process_group = libadwaita::PreferencesGroup::new();
    post_process_group.set_widget_name("post_processing_mode_group");
    post_process_group.set_title("AI Post-Processing");
    post_process_group.set_description(Some(
        "Automatically refine, summarize, format, or translate transcripts using Large Language Models.",
    ));
    post_process_group.set_hexpand(true);
    page.add(&post_process_group);

    // --- 2. Prompt Templates ---
    let prompts_group = libadwaita::PreferencesGroup::new();
    prompts_group.set_widget_name("prompts_templates_group");
    prompts_group.set_title("Prompt Templates");
    prompts_group.set_description(Some(
        "Reusable prompt templates. Available variables: ${output}, ${selected_text}, ${clipboard}, ${active_window}, ${date}, ${language}.",
    ));
    prompts_group.set_hexpand(true);
    page.add(&prompts_group);

    // Owned row lists, one per rebuilt group (see `PageGroup`).
    let master_rows = Arc::new(Mutex::new(crate::ui::pages::PageGroup::new()));
    let prompts_rows = Arc::new(Mutex::new(crate::ui::pages::PageGroup::new()));

    // Initial render
    refresh_master_group(ctx, &post_process_group, &master_rows);
    refresh_prompts_group(ctx, &prompts_group, &prompts_rows);

    // Live refresh on bus events
    let post_weak = glib::SendWeakRef::from(post_process_group.downgrade());
    let prompts_weak = glib::SendWeakRef::from(prompts_group.downgrade());
    let ctx_bus = ctx.clone();
    let master_rows_bus = master_rows.clone();
    let prompts_rows_bus = prompts_rows.clone();

    ctx.bus.subscribe(move |event| {
        let ctx = ctx_bus.clone();
        let post_weak = post_weak.clone();
        let prompts_weak = prompts_weak.clone();
        let master_rows = master_rows_bus.clone();
        let prompts_rows = prompts_rows_bus.clone();

        glib::MainContext::default().invoke(move || {
            if let AppEvent::SettingsChanged { setting, .. } = event {
                if setting == "post_process_enabled"
                    || setting == "post_process_selected_prompt_id"
                    || setting == "post_process_prompt_name"
                {
                    if let Some(grp) = post_weak.into_weak_ref().upgrade() {
                        refresh_master_group(&ctx, &grp, &master_rows);
                    }
                } else if setting == "post_process_prompts"
                    || setting == "post_process_prompts_structure"
                    || setting == "post_process_providers"
                    || setting == "post_process_providers_reordered"
                {
                    if let Some(grp) = prompts_weak.into_weak_ref().upgrade() {
                        refresh_prompts_group(&ctx, &grp, &prompts_rows);
                    }
                    if let Some(grp) = post_weak.into_weak_ref().upgrade() {
                        refresh_master_group(&ctx, &grp, &master_rows);
                    }
                }
            }
        });
    });

    page.upcast::<gtk4::Widget>()
}

fn refresh_master_group(
    ctx: &AppContext,
    group: &libadwaita::PreferencesGroup,
    rows: &Arc<Mutex<crate::ui::pages::PageGroup>>,
) {
    rows.lock().unwrap_or_else(|e| e.into_inner()).clear(group);

    let settings = ctx.settings();

    // Enable / Disable master switch
    let master_switch = libadwaita::SwitchRow::new();
    master_switch.set_title("Enable AI Post-Processing");
    master_switch.set_subtitle("Apply LLM transformations automatically upon speech capture");
    let star_icon = gtk4::Image::from_icon_name("starred-symbolic");
    master_switch.add_prefix(&star_icon);
    master_switch.set_active(settings.post_process_enabled);

    // Default Prompt Selector
    let prompt_row = libadwaita::ComboRow::new();
    prompt_row.set_title("Active Default Prompt");
    prompt_row.set_subtitle("Template executed for speech dictation post-processing");
    let prompt_icon = gtk4::Image::from_icon_name("document-open-symbolic");
    prompt_row.add_prefix(&prompt_icon);
    prompt_row.set_visible(settings.post_process_enabled);

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
            if let Err(err) = shortcut::set_post_process_selected_prompt(&p_ctx, id.clone()) {
                p_ctx.report_error("set_post_process_selected_prompt", err);
            }
        }
    });

    let switch_ctx = ctx.clone();
    let prompt_row_weak = glib::SendWeakRef::from(prompt_row.downgrade());
    master_switch.connect_active_notify(move |row| {
        let is_active = row.is_active();
        if let Err(err) = shortcut::change_post_process_enabled_setting(&switch_ctx, is_active) {
            switch_ctx.report_error("change_post_process_enabled_setting", err);
        }
        if let Some(pr) = prompt_row_weak.clone().into_weak_ref().upgrade() {
            pr.set_visible(is_active);
        }
    });

    rows.lock()
        .unwrap_or_else(|e| e.into_inner())
        .add(group, &master_switch);
    rows.lock()
        .unwrap_or_else(|e| e.into_inner())
        .add(group, &prompt_row);
}

fn refresh_prompts_group(
    ctx: &AppContext,
    group: &libadwaita::PreferencesGroup,
    rows: &Arc<Mutex<crate::ui::pages::PageGroup>>,
) {
    rows.lock().unwrap_or_else(|e| e.into_inner()).clear(group);

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

        let icon = gtk4::Image::from_icon_name(if prompt.id.starts_with("default_") {
            "starred-symbolic"
        } else {
            "accessories-text-editor-symbolic"
        });
        row.add_prefix(&icon);

        // Delete Button (only for custom prompts)
        if !prompt.id.starts_with("default_") {
            let del_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
            del_btn.add_css_class("flat");
            del_btn.set_tooltip_text(Some("Delete Custom Prompt"));
            del_btn.set_valign(gtk4::Align::Center);
            let del_ctx = ctx.clone();
            let del_id = prompt.id.clone();
            del_btn.connect_clicked(move |_| {
                if let Err(err) = shortcut::remove_post_process_prompt(&del_ctx, del_id.clone()) {
                    del_ctx.report_error("remove_post_process_prompt", err);
                }
            });
            row.add_suffix(&del_btn);
        }

        // 1. Name Row
        let name_row = libadwaita::EntryRow::new();
        name_row.set_title("Prompt Name");
        let name_icon = gtk4::Image::from_icon_name("document-edit-symbolic");
        name_row.add_prefix(&name_icon);
        name_row.set_text(&prompt.name);
        let n_ctx = ctx.clone();
        let n_id = prompt.id.clone();
        let row_weak = glib::SendWeakRef::from(row.downgrade());
        name_row.connect_changed(move |r| {
            let new_name = r.text().to_string();
            let title = if new_name.trim().is_empty() {
                "Untitled Prompt".to_string()
            } else {
                new_name.clone()
            };
            if let Some(row) = row_weak.clone().into_weak_ref().upgrade() {
                row.set_title(&title);
            }
            if let Err(err) =
                shortcut::update_post_process_prompt_name(&n_ctx, n_id.clone(), new_name)
            {
                n_ctx.report_error("update_post_process_prompt_name", err);
            }
        });
        row.add_row(&name_row);

        // 2. Preferred Provider Row
        let pref_prov_row = libadwaita::ComboRow::new();
        pref_prov_row.set_title("Preferred Provider");
        let prov_icon = gtk4::Image::from_icon_name("network-server-symbolic");
        pref_prov_row.add_prefix(&prov_icon);
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
                if let Err(err) = shortcut::set_post_process_prompt_preferred_provider(
                    &pref_ctx,
                    pref_prompt_id.clone(),
                    target_pref.clone(),
                ) {
                    pref_ctx.report_error("set_post_process_prompt_preferred_provider", err);
                }
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
            "Template Prompt Content (Variables: ${output}, ${selected_text}, ${clipboard}, ${active_window}, ${date}, ${language}):",
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
        buffer.connect_changed(move |buf| {
            let start = buf.start_iter();
            let end = buf.end_iter();
            let text = buf.text(&start, &end, false);
            if let Err(err) =
                shortcut::update_post_process_prompt_content(&t_ctx, t_id.clone(), text.to_string())
            {
                t_ctx.report_error("update_post_process_prompt_content", err);
            }
        });

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_min_content_height(100);
        scrolled.set_max_content_height(280);
        scrolled.set_child(Some(&text_view));
        scrolled.add_css_class("card");
        template_box.append(&scrolled);

        template_row.set_child(Some(&template_box));
        row.add_row(&template_row);

        rows.lock()
            .unwrap_or_else(|e| e.into_inner())
            .add(group, &row);
    }

    // Add Custom Prompt Button
    let add_row = libadwaita::ActionRow::new();
    add_row.set_title("Create New Custom Prompt Template");
    add_row.set_subtitle("Add a custom prompt instruction with dynamic variables like ${output}");
    let add_icon = gtk4::Image::from_icon_name("list-add-symbolic");
    add_row.add_prefix(&add_icon);

    let add_btn = gtk4::Button::new();
    let add_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let add_btn_icon = gtk4::Image::from_icon_name("list-add-symbolic");
    add_box.append(&add_btn_icon);
    add_box.append(&gtk4::Label::new(Some("Add Prompt")));
    add_btn.set_child(Some(&add_box));
    add_btn.set_valign(gtk4::Align::Center);
    add_btn.add_css_class("suggested-action");

    let add_ctx = ctx.clone();
    add_btn.connect_clicked(move |_| {
        if let Err(err) = shortcut::add_post_process_prompt(
            &add_ctx,
            "New Prompt".to_string(),
            "Please edit this transcript:\n${output}".to_string(),
        ) {
            add_ctx.report_error("add_post_process_prompt", err);
        }
    });
    add_row.add_suffix(&add_btn);
    rows.lock()
        .unwrap_or_else(|e| e.into_inner())
        .add(group, &add_row);
}
