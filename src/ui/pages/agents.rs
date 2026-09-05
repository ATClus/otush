//! Agents settings page: manage AI chat agents.
//!
//! Each agent binds to an LLM provider from Cloud Providers with an optional
//! model override, a system prompt, a web-tool allow-list, and RAG options.
//! Editing goes through `commands::agents` (validated writes); the page
//! refreshes on `SettingsChanged { "agents" }` via `PageGroup`.

use crate::context::{AppContext, AppEvent};
use crate::settings::{AgentConfig, AGENT_TOOL_NAMES};
use gtk4::prelude::*;
use libadwaita::prelude::*;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

/// Build the Agents preferences page.
pub fn build(ctx: &AppContext) -> gtk4::Widget {
    let page = libadwaita::PreferencesPage::new();
    page.set_title("Agents");
    page.set_icon_name(Some("chat-symbolic"));

    let list_group = libadwaita::PreferencesGroup::new();
    list_group.set_title("AI Chat Agents");
    list_group.set_description(Some(
        "Agents power the Chat overlay: each binds to an LLM provider with its own model, tools, and local knowledge.",
    ));
    list_group.set_hexpand(true);
    page.add(&list_group);

    let rows = Arc::new(Mutex::new(crate::ui::pages::PageGroup::new()));
    let expanded: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

    refresh_agents_group(ctx, &list_group, &expanded, &rows);

    // Live refresh.
    let group_weak = glib::SendWeakRef::from(list_group.downgrade());
    let ctx_bus = ctx.clone();
    let rows_bus = rows.clone();
    let exp_bus = expanded.clone();
    ctx.bus.subscribe(move |event| {
        let ctx = ctx_bus.clone();
        let group_weak = group_weak.clone();
        let rows = rows_bus.clone();
        let exp = exp_bus.clone();
        glib::MainContext::default().invoke(move || {
            if let AppEvent::SettingsChanged { setting, .. } = event {
                if setting == "agents"
                    || setting == "selected_agent_id"
                    || setting == "post_process_provider_models"
                {
                    if let Some(group) = group_weak.into_weak_ref().upgrade() {
                        refresh_agents_group(&ctx, &group, &exp, &rows);
                    }
                }
            }
        });
    });

    build_rag_group(ctx, &page);

    page.upcast::<gtk4::Widget>()
}

fn refresh_agents_group(
    ctx: &AppContext,
    group: &libadwaita::PreferencesGroup,
    expanded: &Arc<Mutex<HashSet<String>>>,
    rows: &Arc<Mutex<crate::ui::pages::PageGroup>>,
) {
    let mut guard = match rows.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard.clear(group);
    drop(guard);

    let settings = crate::settings::get_settings(ctx);
    let agents = settings.agents.clone();

    for agent in &agents {
        let is_expanded = expanded
            .lock()
            .map(|set| set.contains(&agent.id))
            .unwrap_or(false);

        let row = libadwaita::ExpanderRow::new();
        let provider_label = settings
            .post_process_provider(&agent.provider_id)
            .map(|p| p.label.clone())
            .unwrap_or_else(|| agent.provider_id.clone());
        let model = agent
            .model_override
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .map(str::to_string)
            .or_else(|| {
                settings
                    .post_process_models
                    .get(&agent.provider_id)
                    .cloned()
                    .filter(|m| !m.trim().is_empty())
            })
            .unwrap_or_else(|| "no model".to_string());
        row.set_title(&format!("{} — {provider_label} / {model}", agent.name));
        row.set_subtitle(if agent.enabled {
            agent.description.as_str()
        } else {
            "Disabled"
        });
        row.set_expanded(is_expanded);

        // Enabled switch.
        let enabled_switch = gtk4::Switch::new();
        enabled_switch.set_valign(gtk4::Align::Center);
        enabled_switch.set_active(agent.enabled);
        let ctx_switch = ctx.clone();
        let agent_switch = agent.clone();
        enabled_switch.connect_state_set(move |_, active| {
            let mut updated = agent_switch.clone();
            updated.enabled = active;
            if let Err(e) = crate::commands::agents::update_agent(&ctx_switch, updated) {
                ctx_switch.report_error("update_agent", e);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        row.add_suffix(&enabled_switch);

        // Select button.
        let select_btn = gtk4::Button::with_label("Use");
        select_btn.set_valign(gtk4::Align::Center);
        select_btn.add_css_class("flat");
        let is_selected = settings.selected_agent_id.as_deref() == Some(agent.id.as_str());
        select_btn.set_sensitive(!is_selected && agent.enabled);
        let ctx_select = ctx.clone();
        let agent_id = agent.id.clone();
        select_btn.connect_clicked(move |_| {
            if let Err(e) = crate::commands::agents::select_agent(&ctx_select, &agent_id) {
                ctx_select.report_error("select_agent", e);
            }
        });
        row.add_suffix(&select_btn);

        // Delete button (keep at least one agent).
        if agents.len() > 1 {
            let delete_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
            delete_btn.set_valign(gtk4::Align::Center);
            delete_btn.add_css_class("flat");
            delete_btn.set_tooltip_text(Some("Delete agent"));
            let ctx_delete = ctx.clone();
            let agent_id = agent.id.clone();
            delete_btn.connect_clicked(move |_| {
                if let Err(e) = crate::commands::agents::delete_agent(&ctx_delete, &agent_id) {
                    ctx_delete.report_error("delete_agent", e);
                }
            });
            row.add_suffix(&delete_btn);
        }

        build_agent_editor(ctx, &row, agent, expanded);

        if let Ok(mut guard) = rows.lock() {
            guard.add(group, &row);
        }
    }

    // Add-agent row.
    let add_row = libadwaita::ActionRow::new();
    add_row.set_title("New agent");
    add_row.set_subtitle("Create a persona bound to an LLM provider");
    add_row.set_activatable(true);
    let add_icon = gtk4::Image::from_icon_name("list-add-symbolic");
    add_row.add_prefix(&add_icon);
    let ctx_add = ctx.clone();
    add_row.connect_activated(move |_| {
        let count = crate::settings::get_settings(&ctx_add).agents.len();
        let agent = AgentConfig {
            id: format!("agent-{}", count + 1),
            name: format!("Agent {}", count + 1),
            description: String::new(),
            enabled: true,
            provider_id: "openai".to_string(),
            model_override: None,
            system_prompt: "You are a helpful assistant. Answer concisely in Markdown.".to_string(),
            enabled_tools: AGENT_TOOL_NAMES.iter().map(|s| s.to_string()).collect(),
            max_tool_steps: 6,
            tool_budget_per_tool: 3,
            rag_enabled: true,
            rag_top_k: 4,
        };
        if let Err(e) = crate::commands::agents::create_agent(&ctx_add, agent) {
            ctx_add.report_error("create_agent", e);
        }
    });
}

/// Static RAG knowledge group (never rebuilt: no PageGroup tracking).
fn build_rag_group(ctx: &AppContext, page: &libadwaita::PreferencesPage) {
    // RAG knowledge group: index parsed documents into the local store.
    let rag_group = libadwaita::PreferencesGroup::new();
    rag_group.set_title("Local Knowledge (RAG)");
    rag_group.set_description(Some(
        "Index parsed documents so agents can cite them. Retrieval is local full-text search (FTS5).",
    ));
    rag_group.set_hexpand(true);

    let index_row = libadwaita::ActionRow::new();
    index_row.set_title("Index all parsed documents");
    index_row.set_subtitle("Rebuild the local search index from Documents & OCR");
    index_row.set_activatable(true);
    let index_icon = gtk4::Image::from_icon_name("system-search-symbolic");
    index_row.add_prefix(&index_icon);
    let ctx_index = ctx.clone();
    index_row.connect_activated(move |_| {
        let docs = match ctx_index.history.list_docs() {
            Ok(docs) => docs,
            Err(e) => {
                ctx_index.report_error("rag_index", anyhow::anyhow!(e));
                return;
            }
        };
        let ids: Vec<i64> = docs.iter().map(|d| d.id).collect();
        match crate::commands::agents::index_suite_docs(&ctx_index, &ids) {
            Ok(count) => {
                ctx_index.bus.send(AppEvent::SettingsChanged {
                    setting: "rag_index".to_string(),
                    value: serde_json::json!({ "indexed": count }),
                });
            }
            Err(e) => ctx_index.report_error("rag_index", e),
        }
    });
    rag_group.add(&index_row);

    let clear_row = libadwaita::ActionRow::new();
    clear_row.set_title("Clear local index");
    clear_row.set_subtitle("Remove all indexed passages");
    clear_row.set_activatable(true);
    let ctx_clear = ctx.clone();
    clear_row.connect_activated(move |_| {
        if let Err(e) = crate::commands::agents::clear_rag_index(&ctx_clear) {
            ctx_clear.report_error("rag_index", e);
        }
    });
    rag_group.add(&clear_row);

    page.add(&rag_group);
}

/// Per-agent editor rows inside the expander.
fn build_agent_editor(
    ctx: &AppContext,
    row: &libadwaita::ExpanderRow,
    agent: &AgentConfig,
    expanded: &Arc<Mutex<HashSet<String>>>,
) {
    let edit = std::rc::Rc::new(std::cell::RefCell::new(agent.clone()));
    let agent_id = agent.id.clone();
    let expanded = expanded.clone();
    row.connect_expanded_notify(move |row| {
        if let Ok(mut set) = expanded.lock() {
            if row.is_expanded() {
                set.insert(agent_id.clone());
            } else {
                set.remove(&agent_id);
            }
        }
    });

    // Provider combo.
    let settings = crate::settings::get_settings(ctx);
    let provider_names: Vec<String> = settings
        .post_process_providers
        .iter()
        .map(|p| format!("{} ({})", p.label, p.id))
        .collect();
    let provider_combo = libadwaita::ComboRow::new();
    provider_combo.set_title("LLM provider");
    let provider_store = gtk4::StringList::new(
        &provider_names
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>(),
    );
    provider_combo.set_model(Some(&provider_store));
    if let Some(pos) = settings
        .post_process_providers
        .iter()
        .position(|p| p.id == agent.provider_id)
    {
        provider_combo.set_selected(pos as u32);
    }
    let ctx_provider = ctx.clone();
    let edit_provider = edit.clone();
    provider_combo.connect_selected_notify(move |combo| {
        let idx = combo.selected() as usize;
        let providers = crate::settings::get_settings(&ctx_provider).post_process_providers;
        if let Some(provider) = providers.get(idx) {
            let updated = {
                let mut draft = edit_provider.borrow_mut();
                draft.provider_id = provider.id.clone();
                draft.clone()
            };
            if let Err(e) = crate::commands::agents::update_agent(&ctx_provider, updated) {
                ctx_provider.report_error("update_agent", e);
            }
        }
    });
    row.add_row(&provider_combo);

    // Model override entry.
    let model_row = libadwaita::EntryRow::new();
    model_row.set_title("Model override (empty = provider default)");
    model_row.set_text(agent.model_override.as_deref().unwrap_or(""));
    let edit_model = edit.clone();
    model_row.connect_changed(move |entry| {
        let text = entry.text().to_string();
        edit_model.borrow_mut().model_override = if text.trim().is_empty() {
            None
        } else {
            Some(text)
        };
    });
    let ctx_model_apply = ctx.clone();
    let edit_model_apply = edit.clone();
    let apply_btn = gtk4::Button::with_label("Apply");
    apply_btn.set_valign(gtk4::Align::Center);
    apply_btn.connect_clicked(move |_| {
        let updated = edit_model_apply.borrow().clone();
        if let Err(e) = crate::commands::agents::update_agent(&ctx_model_apply, updated) {
            ctx_model_apply.report_error("update_agent", e);
        }
    });
    model_row.add_suffix(&apply_btn);
    row.add_row(&model_row);

    // System prompt editor.
    let prompt_row = libadwaita::EntryRow::new();
    prompt_row.set_title("System prompt");
    prompt_row.set_text(&agent.system_prompt);
    let edit_prompt = edit.clone();
    prompt_row.connect_changed(move |entry| {
        edit_prompt.borrow_mut().system_prompt = entry.text().to_string();
    });
    let ctx_prompt_apply = ctx.clone();
    let edit_prompt_apply = edit.clone();
    let prompt_apply = gtk4::Button::with_label("Apply");
    prompt_apply.set_valign(gtk4::Align::Center);
    prompt_apply.connect_clicked(move |_| {
        let updated = edit_prompt_apply.borrow().clone();
        if let Err(e) = crate::commands::agents::update_agent(&ctx_prompt_apply, updated) {
            ctx_prompt_apply.report_error("update_agent", e);
        }
    });
    prompt_row.add_suffix(&prompt_apply);
    row.add_row(&prompt_row);

    // Tool toggles.
    for tool_name in AGENT_TOOL_NAMES {
        let tool_row = libadwaita::SwitchRow::new();
        tool_row.set_title(tool_name);
        let enabled =
            agent.enabled_tools.is_empty() || agent.enabled_tools.contains(&tool_name.to_string());
        tool_row.set_active(enabled);
        let ctx_tool = ctx.clone();
        let edit_tool = edit.clone();
        let tool_owned = tool_name.to_string();
        tool_row.connect_active_notify(move |row| {
            let updated = {
                let mut draft = edit_tool.borrow_mut();
                if row.is_active() {
                    if !draft.enabled_tools.contains(&tool_owned) {
                        draft.enabled_tools.push(tool_owned.clone());
                    }
                } else {
                    draft.enabled_tools.retain(|t| t != &tool_owned);
                }
                draft.clone()
            };
            if let Err(e) = crate::commands::agents::update_agent(&ctx_tool, updated) {
                ctx_tool.report_error("update_agent", e);
            }
        });
        row.add_row(&tool_row);
    }

    // Max steps spin.
    let steps_adj = gtk4::Adjustment::new(
        f64::from(agent.effective_max_steps()),
        1.0,
        12.0,
        1.0,
        1.0,
        0.0,
    );
    let steps_row = libadwaita::SpinRow::new(Some(&steps_adj), 1.0, 0);
    steps_row.set_title("Max tool steps");
    let ctx_steps = ctx.clone();
    let edit_steps = edit.clone();
    steps_row.connect_changed(move |row| {
        let updated = {
            let mut draft = edit_steps.borrow_mut();
            draft.max_tool_steps = row.value() as u32;
            draft.clone()
        };
        if let Err(e) = crate::commands::agents::update_agent(&ctx_steps, updated) {
            ctx_steps.report_error("update_agent", e);
        }
    });
    row.add_row(&steps_row);

    // Per-tool call budget spin.
    let budget_adj = gtk4::Adjustment::new(
        f64::from(agent.effective_tool_budget()),
        1.0,
        10.0,
        1.0,
        1.0,
        0.0,
    );
    let budget_row = libadwaita::SpinRow::new(Some(&budget_adj), 1.0, 0);
    budget_row.set_title("Calls per tool");
    budget_row.set_subtitle("Max calls to the same tool inside one answer");
    let ctx_budget = ctx.clone();
    let edit_budget = edit.clone();
    budget_row.connect_changed(move |row| {
        let updated = {
            let mut draft = edit_budget.borrow_mut();
            draft.tool_budget_per_tool = row.value() as u32;
            draft.clone()
        };
        if let Err(e) = crate::commands::agents::update_agent(&ctx_budget, updated) {
            ctx_budget.report_error("update_agent", e);
        }
    });
    row.add_row(&budget_row);

    // RAG toggles.
    let rag_row = libadwaita::SwitchRow::new();
    rag_row.set_title("Use local documents (RAG)");
    rag_row.set_active(agent.rag_enabled);
    let ctx_rag = ctx.clone();
    let edit_rag = edit.clone();
    rag_row.connect_active_notify(move |row| {
        let updated = {
            let mut draft = edit_rag.borrow_mut();
            draft.rag_enabled = row.is_active();
            draft.clone()
        };
        if let Err(e) = crate::commands::agents::update_agent(&ctx_rag, updated) {
            ctx_rag.report_error("update_agent", e);
        }
    });
    row.add_row(&rag_row);

    let topk_adj =
        gtk4::Adjustment::new(f64::from(agent.effective_top_k()), 1.0, 10.0, 1.0, 1.0, 0.0);
    let topk_row = libadwaita::SpinRow::new(Some(&topk_adj), 1.0, 0);
    topk_row.set_title("RAG passages (top-k)");
    let ctx_topk = ctx.clone();
    let edit_topk = edit.clone();
    topk_row.connect_changed(move |row| {
        let updated = {
            let mut draft = edit_topk.borrow_mut();
            draft.rag_top_k = row.value() as u32;
            draft.clone()
        };
        if let Err(e) = crate::commands::agents::update_agent(&ctx_topk, updated) {
            ctx_topk.report_error("update_agent", e);
        }
    });
    row.add_row(&topk_row);
}
