//! Unified Providers page: consolidating Speech-to-Text (STT),
//! General LLMs & Reasoning, and Web & Docs search/extraction into a single page.

use crate::context::{AppContext, AppEvent};
use crate::settings;
use crate::shortcut;
use libadwaita::prelude::*;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

/// Build the unified Providers preferences page.
pub fn build(ctx: &AppContext) -> gtk4::Widget {
    let page = libadwaita::PreferencesPage::new();
    page.set_title("Cloud Providers");
    page.set_icon_name(Some("network-server-symbolic"));

    // --- 1. General LLM & Reasoning Providers ---
    let llm_group = libadwaita::PreferencesGroup::new();
    llm_group.set_widget_name("providers_llm_group");
    llm_group.set_title("General LLM &amp; Reasoning");
    llm_group.set_description(Some(
        "AI reasoning, post-processing, and multimodal vision models (Anthropic, OpenAI, Google, DeepSeek, Z.AI, Meta, Moonshot AI, Mistral, Ollama, Local SLM). Evaluated in priority order.",
    ));
    llm_group.set_hexpand(true);
    page.add(&llm_group);

    // --- 2. Speech-to-Text (STT) Providers ---
    let stt_group = libadwaita::PreferencesGroup::new();
    stt_group.set_widget_name("providers_stt_group");
    stt_group.set_title("Speech-to-Text (STT) Cloud");
    stt_group.set_description(Some(
        "Cloud transcription engines (Deepgram, Google, OpenAI, Gladia, AssemblyAI, Groq, Custom). Evaluated in priority order when cloud mode is preferred.",
    ));
    stt_group.set_hexpand(true);
    page.add(&stt_group);

    // --- 3. Web & Docs Intelligence Providers ---
    let web_group = libadwaita::PreferencesGroup::new();
    web_group.set_widget_name("providers_web_group");
    web_group.set_title("Web &amp; Document Intelligence");
    web_group.set_description(Some(
        "Search, crawling, scraping, and document OCR / parsing APIs (Tavily, Firecrawl).",
    ));
    web_group.set_hexpand(true);
    page.add(&web_group);

    let expanded_llm: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let expanded_stt: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let expanded_web: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

    // Owned row lists, one per rebuilt group (see `PageGroup`).
    let llm_rows = Arc::new(Mutex::new(crate::ui::pages::PageGroup::new()));
    let stt_rows = Arc::new(Mutex::new(crate::ui::pages::PageGroup::new()));
    let web_rows = Arc::new(Mutex::new(crate::ui::pages::PageGroup::new()));

    // Initial render
    refresh_llm_providers_group(ctx, &llm_group, &expanded_llm, &llm_rows);
    refresh_stt_providers_group(ctx, &stt_group, &expanded_stt, &stt_rows);
    refresh_web_providers_group(ctx, &web_group, &expanded_web, &web_rows);

    // Live refresh on bus events
    let llm_weak = glib::SendWeakRef::from(llm_group.downgrade());
    let stt_weak = glib::SendWeakRef::from(stt_group.downgrade());
    let web_weak = glib::SendWeakRef::from(web_group.downgrade());
    let ctx_bus = ctx.clone();
    let llm_rows_bus = llm_rows.clone();
    let stt_rows_bus = stt_rows.clone();
    let web_rows_bus = web_rows.clone();

    ctx.bus.subscribe(move |event| {
        let ctx = ctx_bus.clone();
        let llm_weak = llm_weak.clone();
        let stt_weak = stt_weak.clone();
        let web_weak = web_weak.clone();
        let exp_llm = expanded_llm.clone();
        let exp_stt = expanded_stt.clone();
        let exp_web = expanded_web.clone();
        let llm_rows = llm_rows_bus.clone();
        let stt_rows = stt_rows_bus.clone();
        let web_rows = web_rows_bus.clone();

        glib::MainContext::default().invoke(move || {
            if let AppEvent::SettingsChanged { setting, .. } = event {
                if setting == "post_process_providers_reordered"
                    || setting == "post_process_provider_models"
                {
                    if let Some(grp) = llm_weak.into_weak_ref().upgrade() {
                        refresh_llm_providers_group(&ctx, &grp, &exp_llm, &llm_rows);
                    }
                } else if setting == "transcription_providers_reordered"
                    || setting == "transcription_provider_model"
                    || setting == "local_transcription_enabled"
                {
                    if let Some(grp) = stt_weak.into_weak_ref().upgrade() {
                        refresh_stt_providers_group(&ctx, &grp, &exp_stt, &stt_rows);
                    }
                } else if setting == "web_providers_reordered"
                    || setting == "web_provider_api_key"
                    || setting == "web_provider_base_url"
                {
                    if let Some(grp) = web_weak.into_weak_ref().upgrade() {
                        refresh_web_providers_group(&ctx, &grp, &exp_web, &web_rows);
                    }
                }
            }
        });
    });

    page.upcast::<gtk4::Widget>()
}

// ============================================================================
// 1. General LLM & Reasoning Section
// ============================================================================

fn refresh_llm_providers_group(
    ctx: &AppContext,
    group: &libadwaita::PreferencesGroup,
    expanded_ids: &Arc<Mutex<HashSet<String>>>,
    rows: &Arc<Mutex<crate::ui::pages::PageGroup>>,
) {
    rows.lock().unwrap_or_else(|e| e.into_inner()).clear(group);

    let settings = settings::get_settings(ctx);
    let total_providers = settings.post_process_providers.len();

    for (idx, provider) in settings.post_process_providers.iter().enumerate() {
        let row = libadwaita::ExpanderRow::new();
        row.set_title(&format!("#{}: {}", idx + 1, provider.label));
        let prov_icon = gtk4::Image::from_icon_name("network-server-symbolic");
        row.add_prefix(&prov_icon);

        let current_model = settings
            .post_process_models
            .get(&provider.id)
            .cloned()
            .unwrap_or_default();

        let model_display = if current_model.is_empty() {
            "(default)".to_string()
        } else {
            current_model.clone()
        };

        let subtitle = if provider.allow_base_url_edit {
            format!("Model: {} • Endpoint: {}", model_display, provider.base_url)
        } else {
            format!("Model: {}", model_display)
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
                if let Err(err) =
                    shortcut::move_post_process_provider_priority(&up_ctx, &up_pid, true)
                {
                    up_ctx.report_error("move_post_process_provider_priority", err);
                }
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
                if let Err(err) =
                    shortcut::move_post_process_provider_priority(&down_ctx, &down_pid, false)
                {
                    down_ctx.report_error("move_post_process_provider_priority", err);
                }
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
            if let Err(err) = shortcut::toggle_post_process_provider_enabled(
                &en_ctx,
                en_id.clone(),
                sw.is_active(),
            ) {
                en_ctx.report_error("toggle_post_process_provider_enabled", err);
            }
        });
        row.add_suffix(&enable_switch);

        // 1. API Key Row
        let api_key_row = libadwaita::PasswordEntryRow::new();
        api_key_row.set_title("API Key");
        let key_icon = gtk4::Image::from_icon_name("dialog-password-symbolic");
        api_key_row.add_prefix(&key_icon);
        let api_key = settings
            .post_process_api_keys
            .get(&provider.id)
            .cloned()
            .unwrap_or_default();
        api_key_row.set_text(&api_key);

        let key_ctx = ctx.clone();
        let key_id = provider.id.clone();
        api_key_row.connect_changed(move |r| {
            if let Err(err) = shortcut::change_post_process_api_key_setting(
                &key_ctx,
                key_id.clone(),
                r.text().to_string(),
            ) {
                key_ctx.report_error("change_post_process_api_key_setting", err);
            }
        });
        row.add_row(&api_key_row);

        // 2. Dynamic Model Input Row with "Fetch Models" capability
        let model_row = libadwaita::EntryRow::new();
        model_row.set_title("Model ID");
        let model_icon = gtk4::Image::from_icon_name("application-x-executable-symbolic");
        model_row.add_prefix(&model_icon);
        model_row.set_text(&current_model);

        // Add "Fetch Available Models" button suffix
        let fetch_btn = gtk4::Button::from_icon_name("view-refresh-symbolic");
        fetch_btn.set_valign(gtk4::Align::Center);
        fetch_btn.add_css_class("flat");
        fetch_btn.set_tooltip_text(Some(
            "Query provider API to list current models (no hardcoded models)",
        ));

        let f_ctx = ctx.clone();
        let f_pid = provider.id.clone();
        let f_model_row_weak = glib::SendWeakRef::from(model_row.downgrade());
        let f_btn_weak = glib::SendWeakRef::from(fetch_btn.downgrade());

        fetch_btn.connect_clicked(move |_| {
            if let Some(btn) = f_btn_weak.clone().into_weak_ref().upgrade() {
                btn.set_sensitive(false);
            }
            let task_ctx = f_ctx.clone();
            let task_pid = f_pid.clone();
            let task_row = f_model_row_weak.clone();
            let task_btn = f_btn_weak.clone();

            crate::runtime::spawn(async move {
                let models_res = shortcut::fetch_llm_provider_models(&task_ctx, task_pid.clone()).await;
                glib::MainContext::default().invoke(move || {
                    if let Some(btn) = task_btn.into_weak_ref().upgrade() {
                        btn.set_sensitive(true);
                    }
                    if let Some(row) = task_row.into_weak_ref().upgrade() {
                        match models_res {
                            Ok(models) if !models.is_empty() => {
                                // If the current model is empty, set to the first fetched model
                                if row.text().is_empty() {
                                    if let Some(first) = models.first() {
                                        row.set_text(first);
                                        if let Err(err) = shortcut::change_post_process_model_setting(
                                            &task_ctx,
                                            task_pid.clone(),
                                            first.clone(),
                                        ) {
                                            task_ctx.report_error("change_post_process_model_setting", err);
                                        }
                                    }
                                }
                                let tip = format!("Available models ({} found):\n{}", models.len(), models.join(", "));
                                row.set_tooltip_text(Some(&tip));
                            }
                            Ok(_) => {
                                row.set_tooltip_text(Some("Provider returned empty models list. You can enter any custom model name."));
                            }
                            Err(e) => {
                                row.set_tooltip_text(Some(&format!("Could not fetch models automatically ({e}). You can enter any custom model name manually.")));
                            }
                        }
                    }
                });
            });
        });
        model_row.add_suffix(&fetch_btn);

        let model_ctx = ctx.clone();
        let model_id = provider.id.clone();
        model_row.connect_changed(move |r| {
            if let Err(err) = shortcut::change_post_process_model_setting(
                &model_ctx,
                model_id.clone(),
                r.text().to_string(),
            ) {
                model_ctx.report_error("change_post_process_model_setting", err);
            }
        });
        row.add_row(&model_row);

        // 3. Base URL Row (if editable)
        if provider.allow_base_url_edit {
            let url_row = libadwaita::EntryRow::new();
            url_row.set_title("Base URL Endpoint");
            let url_icon = gtk4::Image::from_icon_name("network-server-symbolic");
            url_row.add_prefix(&url_icon);
            url_row.set_text(&provider.base_url);

            let url_ctx = ctx.clone();
            let url_id = provider.id.clone();
            url_row.connect_changed(move |r| {
                if let Err(err) = shortcut::change_post_process_base_url_setting(
                    &url_ctx,
                    url_id.clone(),
                    r.text().to_string(),
                ) {
                    url_ctx.report_error("change_post_process_base_url_setting", err);
                }
            });
            row.add_row(&url_row);
        }

        // 4. Request Timeout Row (SpinRow)
        let timeout_adj = gtk4::Adjustment::new(
            provider.timeout_seconds.max(15) as f64,
            15.0,
            600.0,
            15.0,
            30.0,
            0.0,
        );
        let timeout_row = libadwaita::SpinRow::new(Some(&timeout_adj), 15.0, 0);
        timeout_row.set_title("Request Timeout (seconds)");
        timeout_row.set_subtitle("Default 120s. Increase for reasoning or vision tasks.");
        let time_icon = gtk4::Image::from_icon_name("preferences-system-time-symbolic");
        timeout_row.add_prefix(&time_icon);
        timeout_row.set_snap_to_ticks(true);
        timeout_row.set_numeric(true);

        let timeout_ctx = ctx.clone();
        let timeout_id = provider.id.clone();
        timeout_adj.connect_value_changed(move |adj| {
            let val = adj.value().round() as u32;
            if let Err(err) =
                shortcut::change_post_process_timeout_setting(&timeout_ctx, timeout_id.clone(), val)
            {
                timeout_ctx.report_error("change_post_process_timeout_setting", err);
            }
        });
        row.add_row(&timeout_row);

        // 5. Reasoning / Thinking Effort Row (ComboRow)
        let reasoning_row = libadwaita::ComboRow::new();
        reasoning_row.set_title("Reasoning & Thinking Effort");
        let reasoning_subtitle = match provider.id.as_str() {
            "meta" => "Meta Muse Spark reasoning depth (minimal, low, medium, high, xhigh)",
            "anthropic" => "Anthropic Claude extended thinking budget tokens",
            "deepseek" => "DeepSeek-R1 reasoning engine mode",
            "openai" => "OpenAI o-series reasoning effort (low, medium, high)",
            _ => "Internal reasoning tokens and thinking depth for supported models",
        };
        reasoning_row.set_subtitle(reasoning_subtitle);
        let r_icon = gtk4::Image::from_icon_name("system-run-symbolic");
        reasoning_row.add_prefix(&r_icon);

        let reasoning_model = gtk4::StringList::new(&[
            "Disabled / Model Default",
            "Minimal (Fastest Thinking - Meta)",
            "Low (Brief Thinking)",
            "Medium (Balanced)",
            "High (Deep Thinking)",
            "Maximum / XHigh (Exhaustive - Meta)",
        ]);
        reasoning_row.set_model(Some(&reasoning_model));
        reasoning_row.set_selected(provider.reasoning.effort.to_index());

        let r_ctx = ctx.clone();
        let r_pid = provider.id.clone();
        let r_budget = provider.reasoning.budget_tokens;
        reasoning_row.connect_selected_notify(move |combo| {
            let effort = settings::ReasoningEffort::from_index(combo.selected());
            if let Err(err) = shortcut::set_post_process_provider_reasoning(
                &r_ctx,
                r_pid.clone(),
                effort,
                r_budget,
            ) {
                r_ctx.report_error("set_post_process_provider_reasoning", err);
            }
        });
        row.add_row(&reasoning_row);

        // 6. Test Connection Row
        let test_row = libadwaita::ActionRow::new();
        test_row.set_title("Connection Test");
        test_row.set_subtitle("Send a short test query to verify API key and model availability");
        let test_icon = gtk4::Image::from_icon_name("network-transmit-receive-symbolic");
        test_row.add_prefix(&test_icon);

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
                r.set_subtitle("Connecting to provider endpoint…");
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

        rows.lock()
            .unwrap_or_else(|e| e.into_inner())
            .add(group, &row);
    }
}

// ============================================================================
// 2. Speech to Text (STT) Section
// ============================================================================

fn refresh_stt_providers_group(
    ctx: &AppContext,
    group: &libadwaita::PreferencesGroup,
    expanded_ids: &Arc<Mutex<HashSet<String>>>,
    rows: &Arc<Mutex<crate::ui::pages::PageGroup>>,
) {
    rows.lock().unwrap_or_else(|e| e.into_inner()).clear(group);

    let settings = settings::get_settings(ctx);

    // 1. Local Offline STT row
    let local_row = libadwaita::ActionRow::new();
    local_row.set_title("Local Speech Engine (Whisper / Parakeet)");
    local_row.set_subtitle(if settings.local_transcription_enabled {
        "Offline local models active — running on-device inference"
    } else {
        "Offline local models disabled — falling back to cloud providers below"
    });
    let local_icon = gtk4::Image::from_icon_name("computer-symbolic");
    local_row.add_prefix(&local_icon);

    let local_switch = gtk4::Switch::new();
    local_switch.set_active(settings.local_transcription_enabled);
    local_switch.set_valign(gtk4::Align::Center);
    local_switch.set_tooltip_text(Some(
        "Toggle between local offline models and cloud providers",
    ));
    let sw_ctx = ctx.clone();
    local_switch.connect_active_notify(move |sw| {
        if let Err(err) = shortcut::toggle_local_transcription_setting(&sw_ctx, sw.is_active()) {
            sw_ctx.report_error("toggle_local_transcription_setting", err);
        }
    });
    local_row.add_suffix(&local_switch);
    rows.lock()
        .unwrap_or_else(|e| e.into_inner())
        .add(group, &local_row);

    let total_providers = settings.transcription_providers.len();

    for (idx, provider) in settings.transcription_providers.iter().enumerate() {
        let row = libadwaita::ExpanderRow::new();
        row.set_title(&format!("#{}: {}", idx + 1, provider.label));
        let prov_icon = gtk4::Image::from_icon_name("audio-speakers-symbolic");
        row.add_prefix(&prov_icon);

        let current_model = settings
            .transcription_models
            .get(&provider.id)
            .cloned()
            .unwrap_or_else(|| provider.model.clone());

        let subtitle = if provider.allow_base_url_edit {
            format!("Model: {} • Endpoint: {}", current_model, provider.base_url)
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
            up_btn.set_tooltip_text(Some("Increase STT fallback priority"));
            let up_ctx = ctx.clone();
            let up_pid = provider.id.clone();
            up_btn.connect_clicked(move |_| {
                if let Err(err) =
                    shortcut::move_transcription_provider_priority(&up_ctx, &up_pid, true)
                {
                    up_ctx.report_error("move_transcription_provider_priority", err);
                }
            });
            row.add_suffix(&up_btn);
        }

        // Priority Move Down button
        if idx + 1 < total_providers {
            let down_btn = gtk4::Button::from_icon_name("go-down-symbolic");
            down_btn.set_valign(gtk4::Align::Center);
            down_btn.add_css_class("flat");
            down_btn.set_tooltip_text(Some("Decrease STT fallback priority"));
            let down_ctx = ctx.clone();
            let down_pid = provider.id.clone();
            down_btn.connect_clicked(move |_| {
                if let Err(err) =
                    shortcut::move_transcription_provider_priority(&down_ctx, &down_pid, false)
                {
                    down_ctx.report_error("move_transcription_provider_priority", err);
                }
            });
            row.add_suffix(&down_btn);
        }

        // Enable / Disable switch
        let enable_switch = gtk4::Switch::new();
        enable_switch.set_active(provider.enabled);
        enable_switch.set_valign(gtk4::Align::Center);
        enable_switch.set_tooltip_text(Some("Enable/disable provider in STT fallback chain"));
        let en_ctx = ctx.clone();
        let en_id = provider.id.clone();
        enable_switch.connect_active_notify(move |sw| {
            if let Err(err) = shortcut::toggle_transcription_provider_enabled(
                &en_ctx,
                en_id.clone(),
                sw.is_active(),
            ) {
                en_ctx.report_error("toggle_transcription_provider_enabled", err);
            }
        });
        row.add_suffix(&enable_switch);

        // 1. API Key Row
        let api_key_row = libadwaita::PasswordEntryRow::new();
        api_key_row.set_title("API Key");
        let key_icon = gtk4::Image::from_icon_name("dialog-password-symbolic");
        api_key_row.add_prefix(&key_icon);
        let api_key = settings
            .transcription_api_keys
            .get(&provider.id)
            .cloned()
            .unwrap_or_default();
        api_key_row.set_text(&api_key);

        let key_ctx = ctx.clone();
        let key_id = provider.id.clone();
        api_key_row.connect_changed(move |r| {
            if let Err(err) = shortcut::change_transcription_api_key_setting(
                &key_ctx,
                key_id.clone(),
                r.text().to_string(),
            ) {
                key_ctx.report_error("change_transcription_api_key_setting", err);
            }
        });
        row.add_row(&api_key_row);

        // 2. Model Row (freeform entry)
        let model_row = libadwaita::EntryRow::new();
        model_row.set_title("Model ID");
        let model_icon = gtk4::Image::from_icon_name("audio-input-microphone-symbolic");
        model_row.add_prefix(&model_icon);
        model_row.set_text(&current_model);

        let model_ctx = ctx.clone();
        let model_id = provider.id.clone();
        model_row.connect_changed(move |r| {
            if let Err(err) = shortcut::change_transcription_model_setting(
                &model_ctx,
                model_id.clone(),
                r.text().to_string(),
            ) {
                model_ctx.report_error("change_transcription_model_setting", err);
            }
        });
        row.add_row(&model_row);

        // 3. Base URL Row (if editable)
        if provider.allow_base_url_edit {
            let url_row = libadwaita::EntryRow::new();
            url_row.set_title("Base URL");
            let url_icon = gtk4::Image::from_icon_name("network-server-symbolic");
            url_row.add_prefix(&url_icon);
            url_row.set_text(&provider.base_url);

            let url_ctx = ctx.clone();
            let url_id = provider.id.clone();
            url_row.connect_changed(move |r| {
                if let Err(err) = shortcut::change_transcription_base_url_setting(
                    &url_ctx,
                    url_id.clone(),
                    r.text().to_string(),
                ) {
                    url_ctx.report_error("change_transcription_base_url_setting", err);
                }
            });
            row.add_row(&url_row);
        }

        // 4. Request Timeout Row (SpinRow)
        let timeout_adj = gtk4::Adjustment::new(
            provider.timeout_seconds.max(5) as f64,
            5.0,
            120.0,
            5.0,
            15.0,
            0.0,
        );
        let timeout_row = libadwaita::SpinRow::new(Some(&timeout_adj), 5.0, 0);
        timeout_row.set_title("Request Timeout (seconds)");
        timeout_row.set_subtitle("Default 15-25s. Increase for long recordings.");
        let time_icon = gtk4::Image::from_icon_name("preferences-system-time-symbolic");
        timeout_row.add_prefix(&time_icon);
        timeout_row.set_snap_to_ticks(true);
        timeout_row.set_numeric(true);

        let timeout_ctx = ctx.clone();
        let timeout_id = provider.id.clone();
        timeout_adj.connect_value_changed(move |adj| {
            let val = adj.value().round() as u32;
            if let Err(err) = shortcut::change_transcription_timeout_setting(
                &timeout_ctx,
                timeout_id.clone(),
                val,
            ) {
                timeout_ctx.report_error("change_transcription_timeout_setting", err);
            }
        });
        row.add_row(&timeout_row);

        // 5. Deepgram specific options if deepgram
        if provider.id == "deepgram" {
            let dg_cfg = provider.deepgram.clone().unwrap_or_default();
            let smart_format_row = libadwaita::SwitchRow::new();
            smart_format_row.set_title("Smart Formatting");
            smart_format_row.set_subtitle("Apply automatic punctuation and casing");
            let sf_icon = gtk4::Image::from_icon_name("format-text-symbolic");
            smart_format_row.add_prefix(&sf_icon);
            smart_format_row.set_active(dg_cfg.smart_format);

            let sf_ctx = ctx.clone();
            smart_format_row.connect_active_notify(move |sw| {
                let active = sw.is_active();
                if let Err(err) = shortcut::update_deepgram_config(&sf_ctx, move |cfg| {
                    cfg.smart_format = active;
                }) {
                    sf_ctx.report_error("update_deepgram_config", err);
                }
            });
            row.add_row(&smart_format_row);
        }

        // 6. Test Connection Row
        let test_row = libadwaita::ActionRow::new();
        test_row.set_title("Connection Test");
        test_row.set_subtitle("Transcribe synthetic test tone to verify API key and latency");
        let test_icon = gtk4::Image::from_icon_name("network-transmit-receive-symbolic");
        test_row.add_prefix(&test_icon);

        let test_btn = gtk4::Button::with_label("Test STT");
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
                r.set_subtitle("Sending test audio to STT endpoint…");
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
                        btn.set_label("Test STT");
                    }
                    if let Some(r) = t_row_weak.into_weak_ref().upgrade() {
                        match res {
                            Ok((_, ms)) => {
                                r.set_subtitle(&format!(
                                    "<span foreground=\"#2ec27e\">✓ Connected! Audio transcription latency: {}ms</span>",
                                    ms
                                ));
                            }
                            Err(e) => {
                                r.set_subtitle(&format!(
                                    "<span foreground=\"#e01b24\">✗ STT failed: {}</span>",
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

        rows.lock()
            .unwrap_or_else(|e| e.into_inner())
            .add(group, &row);
    }
}

// ============================================================================
// 3. Web & Docs Intelligence Section (Tavily, Firecrawl)
// ============================================================================

fn refresh_web_providers_group(
    ctx: &AppContext,
    group: &libadwaita::PreferencesGroup,
    expanded_ids: &Arc<Mutex<HashSet<String>>>,
    rows: &Arc<Mutex<crate::ui::pages::PageGroup>>,
) {
    rows.lock().unwrap_or_else(|e| e.into_inner()).clear(group);

    let settings = settings::get_settings(ctx);

    for provider in &settings.web_providers {
        let row = libadwaita::ExpanderRow::new();
        row.set_title(&provider.label);
        let prov_icon = gtk4::Image::from_icon_name("system-search-symbolic");
        row.add_prefix(&prov_icon);
        row.set_subtitle(&provider.base_url);

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

        // Enable / Disable switch
        let enable_switch = gtk4::Switch::new();
        enable_switch.set_active(provider.enabled);
        enable_switch.set_valign(gtk4::Align::Center);
        enable_switch.set_tooltip_text(Some(
            "Enable/disable provider for web search & document parsing",
        ));
        let en_ctx = ctx.clone();
        let en_id = provider.id.clone();
        enable_switch.connect_active_notify(move |sw| {
            if let Err(err) =
                shortcut::toggle_web_provider_enabled(&en_ctx, en_id.clone(), sw.is_active())
            {
                en_ctx.report_error("toggle_web_provider_enabled", err);
            }
        });
        row.add_suffix(&enable_switch);

        // 1. API Key Row
        let api_key_row = libadwaita::PasswordEntryRow::new();
        api_key_row.set_title("API Key");
        let key_icon = gtk4::Image::from_icon_name("dialog-password-symbolic");
        api_key_row.add_prefix(&key_icon);
        let api_key = settings
            .web_api_keys
            .get(&provider.id)
            .cloned()
            .unwrap_or_default();
        api_key_row.set_text(&api_key);

        let key_ctx = ctx.clone();
        let key_id = provider.id.clone();
        api_key_row.connect_changed(move |r| {
            if let Err(err) = shortcut::change_web_provider_api_key_setting(
                &key_ctx,
                key_id.clone(),
                r.text().to_string(),
            ) {
                key_ctx.report_error("change_web_provider_api_key_setting", err);
            }
        });
        row.add_row(&api_key_row);

        // 2. Base URL Row
        if provider.allow_base_url_edit {
            let url_row = libadwaita::EntryRow::new();
            url_row.set_title("Base URL");
            let url_icon = gtk4::Image::from_icon_name("network-server-symbolic");
            url_row.add_prefix(&url_icon);
            url_row.set_text(&provider.base_url);

            let url_ctx = ctx.clone();
            let url_id = provider.id.clone();
            url_row.connect_changed(move |r| {
                if let Err(err) = shortcut::change_web_provider_base_url_setting(
                    &url_ctx,
                    url_id.clone(),
                    r.text().to_string(),
                ) {
                    url_ctx.report_error("change_web_provider_base_url_setting", err);
                }
            });
            row.add_row(&url_row);
        }

        // 3. Request Timeout Row (SpinRow)
        let timeout_adj = gtk4::Adjustment::new(
            provider.timeout_seconds.max(15) as f64,
            15.0,
            300.0,
            15.0,
            30.0,
            0.0,
        );
        let timeout_row = libadwaita::SpinRow::new(Some(&timeout_adj), 15.0, 0);
        timeout_row.set_title("Request Timeout (seconds)");
        timeout_row.set_subtitle("Recommended 60s for deep web research or multi-page crawling.");
        let time_icon = gtk4::Image::from_icon_name("preferences-system-time-symbolic");
        timeout_row.add_prefix(&time_icon);
        timeout_row.set_snap_to_ticks(true);
        timeout_row.set_numeric(true);

        let timeout_ctx = ctx.clone();
        let timeout_id = provider.id.clone();
        timeout_adj.connect_value_changed(move |adj| {
            let val = adj.value().round() as u32;
            if let Err(err) =
                shortcut::change_web_provider_timeout_setting(&timeout_ctx, timeout_id.clone(), val)
            {
                timeout_ctx.report_error("change_web_provider_timeout_setting", err);
            }
        });
        row.add_row(&timeout_row);

        // 4. Test Connection Row
        let test_row = libadwaita::ActionRow::new();
        test_row.set_title("Connection Test");
        test_row.set_subtitle("Query endpoint to verify API key and measure latency");
        let test_icon = gtk4::Image::from_icon_name("network-transmit-receive-symbolic");
        test_row.add_prefix(&test_icon);

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
                r.set_subtitle("Testing connection to web service…");
            }

            let t_ctx = test_ctx.clone();
            let t_id = test_id.clone();
            let t_row_weak = test_row_weak.clone();
            let t_btn_weak = test_btn_weak.clone();

            crate::runtime::spawn(async move {
                let res = shortcut::test_web_provider_connection(&t_ctx, t_id).await;
                glib::MainContext::default().invoke(move || {
                    if let Some(btn) = t_btn_weak.into_weak_ref().upgrade() {
                        btn.set_sensitive(true);
                        btn.set_label("Test Connection");
                    }
                    if let Some(r) = t_row_weak.into_weak_ref().upgrade() {
                        match res {
                            Ok((sample, ms)) => {
                                r.set_subtitle(&format!(
                                    "<span foreground=\"#2ec27e\">✓ Connected ({}ms): \"{}\"</span>",
                                    ms,
                                    glib::markup_escape_text(&sample)
                                ));
                            }
                            Err(e) => {
                                r.set_subtitle(&format!(
                                    "<span foreground=\"#e01b24\">✗ Test failed: {}</span>",
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

        rows.lock()
            .unwrap_or_else(|e| e.into_inner())
            .add(group, &row);
    }
}
