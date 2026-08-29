//! Post-processing settings page: provider, API key, base URL, model and
//! prompt management for the LLM post-processing pipeline.

use crate::context::AppContext;
use crate::shortcut;
use libadwaita::prelude::*;

/// Build the Post-Processing preferences page.
pub fn build(ctx: &AppContext) -> gtk4::Widget {
    let page = libadwaita::PreferencesPage::new();
    page.set_title("Post-Processing");

    // --- Enable toggle ---
    let enable_group = libadwaita::PreferencesGroup::new();
    enable_group.set_title("Post-Processing");
    let toggle = libadwaita::SwitchRow::new();
    toggle.set_title("Enable post-processing");
    toggle.set_subtitle("Clean up transcripts with an AI model");
    toggle.set_active(ctx.settings().post_process_enabled);
    let toggle_ctx = ctx.clone();
    toggle.connect_active_notify(move |row| {
        let _ = shortcut::change_post_process_enabled_setting(&toggle_ctx, row.is_active());
    });
    enable_group.add(&toggle);
    page.add(&enable_group);

    let settings = ctx.settings();

    // --- Provider group ---
    let provider_group = libadwaita::PreferencesGroup::new();
    provider_group.set_title("Provider");

    let providers = settings.post_process_providers.clone();
    let provider_ids: Vec<String> = providers.iter().map(|p| p.id.clone()).collect();
    let provider_labels: Vec<String> = providers
        .iter()
        .map(|p| {
            if p.id == "custom" {
                "Custom (e.g. Ollama)".to_string()
            } else {
                p.label.clone()
            }
        })
        .collect();

    let provider_row = libadwaita::ComboRow::new();
    provider_row.set_title("Provider");
    let model = gtk4::StringList::new(
        &provider_labels
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>(),
    );
    provider_row.set_model(Some(&model));
    let current_provider = settings.post_process_provider_id.clone();
    if let Some(i) = provider_ids.iter().position(|id| *id == current_provider) {
        provider_row.set_selected(i as u32);
    }
    let ctx_for_provider = ctx.clone();
    let provider_ids_for_select = provider_ids.clone();
    provider_row.connect_selected_notify(move |row| {
        if let Some(id) = provider_ids_for_select.get(row.selected() as usize) {
            let _ = shortcut::set_post_process_provider(&ctx_for_provider, id.to_string());
        }
    });
    provider_group.add(&provider_row);

    // API key
    let api_key_row = libadwaita::PasswordEntryRow::new();
    api_key_row.set_title("API Key");
    let key = settings
        .post_process_api_keys
        .get(&current_provider)
        .cloned()
        .unwrap_or_default();
    api_key_row.set_text(&key);
    let key_ctx = ctx.clone();
    let key_provider = current_provider.clone();
    api_key_row.connect_changed(move |row| {
        let _ = shortcut::change_post_process_api_key_setting(
            &key_ctx,
            key_provider.clone(),
            row.text().to_string(),
        );
    });
    provider_group.add(&api_key_row);

    // Model (fetch on demand)
    let model_row = libadwaita::ComboRow::new();
    model_row.set_title("Model");
    model_row.set_subtitle("Model used for post-processing");
    let current_model = settings
        .post_process_models
        .get(&current_provider)
        .cloned()
        .unwrap_or_default();
    model_row.set_subtitle(&if current_model.is_empty() {
        "Select a model".to_string()
    } else {
        current_model.clone()
    });
    provider_group.add(&model_row);
    populate_models(ctx, &model_row, &current_provider, &current_model);

    // Base URL (custom provider only)
    let base_url_row = libadwaita::EntryRow::new();
    base_url_row.set_title("Base URL");
    if current_provider == "custom" {
        if let Some(provider) = settings.post_process_provider(&current_provider) {
            base_url_row.set_text(&provider.base_url);
        }
        let url_ctx = ctx.clone();
        let url_provider = current_provider.clone();
        base_url_row.connect_changed(move |row| {
            let _ = shortcut::change_post_process_base_url_setting(
                &url_ctx,
                url_provider.clone(),
                row.text().to_string(),
            );
        });
        provider_group.add(&base_url_row);
    }

    page.add(&provider_group);

    // --- Prompts group ---
    let prompts_group = libadwaita::PreferencesGroup::new();
    prompts_group.set_title("Prompts");

    let selected_prompt_id = settings.post_process_selected_prompt_id.clone();
    for prompt in &settings.post_process_prompts {
        let row = libadwaita::ActionRow::new();
        row.set_title(&prompt.name);
        row.set_subtitle(&glib::markup_escape_text(
            &prompt.prompt.chars().take(80).collect::<String>(),
        ));
        if Some(&prompt.id) == selected_prompt_id.as_ref() {
            row.set_activatable(true);
            row.add_suffix(&check_badge());
            let row_ctx = ctx.clone();
            let id = prompt.id.clone();
            row.connect_activated(move |_| {
                let _ = shortcut::set_post_process_selected_prompt(&row_ctx, id.clone());
            });
        } else {
            row.set_activatable(true);
            let row_ctx = ctx.clone();
            let id = prompt.id.clone();
            row.connect_activated(move |_| {
                let _ = shortcut::set_post_process_selected_prompt(&row_ctx, id.clone());
            });
        }
        prompts_group.add(&row);
    }

    // Add-prompt button (appends the default template as a new prompt).
    let add_button = gtk4::Button::with_label("Add Prompt");
    add_button.add_css_class("suggested-action");
    let add_ctx = ctx.clone();
    add_button.connect_clicked(move |_| {
        let _ = shortcut::add_post_process_prompt(
            &add_ctx,
            "New Prompt".to_string(),
            "Clean up the transcript.".to_string(),
        );
    });
    let add_row = libadwaita::ActionRow::new();
    add_row.set_title("Add a new prompt");
    add_row.add_suffix(&add_button);
    prompts_group.add(&add_row);

    page.add(&prompts_group);

    page.upcast::<gtk4::Widget>()
}

fn check_badge() -> gtk4::Widget {
    let label = gtk4::Label::new(Some("✓"));
    label.upcast::<gtk4::Widget>()
}

/// Fetch the provider's model list asynchronously and populate the combo.
fn populate_models(
    ctx: &AppContext,
    row: &libadwaita::ComboRow,
    provider_id: &str,
    current_model: &str,
) {
    let ctx = ctx.clone();
    let row_weak = glib::SendWeakRef::from(row.downgrade());
    let provider_id = provider_id.to_string();
    let current_model = current_model.to_string();
    crate::runtime::spawn(async move {
        let result = crate::shortcut::fetch_post_process_models(&ctx, provider_id.clone()).await;
        let row_weak = row_weak.clone();
        let ctx = ctx.clone();
        let provider_id = provider_id.clone();
        glib::MainContext::default().invoke(move || {
            let Some(row) = row_weak.into_weak_ref().upgrade() else {
                return;
            };
            match result {
                Ok(models) if !models.is_empty() => {
                    let names: Vec<&str> = models.iter().map(|s| s.as_str()).collect();
                    let model = gtk4::StringList::new(&names);
                    row.set_model(Some(&model));
                    if let Some(i) = names.iter().position(|n| *n == current_model) {
                        row.set_selected(i as u32);
                    }
                    let row_for_select = row.clone();
                    let ctx_for_select = ctx.clone();
                    let provider_for_select = provider_id.clone();
                    row_for_select.connect_selected_notify(move |row| {
                        if let Some(item) = row.selected_item() {
                            let name = item
                                .downcast_ref::<gtk4::StringObject>()
                                .map(|s| s.string().to_string())
                                .unwrap_or_default();
                            let _ = shortcut::change_post_process_model_setting(
                                &ctx_for_select,
                                provider_for_select.clone(),
                                name,
                            );
                        }
                    });
                }
                _ => {
                    // Model list unavailable (no API key etc.) — leave the row as-is.
                }
            }
        });
    });
}
