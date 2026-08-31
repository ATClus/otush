//! Settings pages, one per sidebar section.

pub mod advanced;
pub mod audio;
pub mod general;
pub mod history;
pub mod models;
pub mod providers_llm;

use crate::context::AppContext;
use libadwaita::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;

// Rows added to a PreferencesGroup, keyed by the group's widget name, so a
// rebuild can remove exactly the rows it owns (AdwPreferencesGroup wraps rows
// internally; generic child iteration hits the internal box instead).
thread_local! {
    static GROUP_ROWS: RefCell<HashMap<String, Vec<gtk4::Widget>>> = RefCell::new(HashMap::new());
}

/// Remove the rows previously tracked for `group` (see [`track_row`]).
pub fn clear_group_rows(group: &libadwaita::PreferencesGroup) {
    let key = group.widget_name().to_string();
    let rows = GROUP_ROWS.with(|m| m.borrow_mut().remove(&key).unwrap_or_default());
    for row in rows {
        group.remove(&row);
    }
}

/// Remember a row added to `group` so the next rebuild can remove it.
pub fn track_row(group: &libadwaita::PreferencesGroup, row: &impl IsA<gtk4::Widget>) {
    let key = group.widget_name().to_string();
    GROUP_ROWS.with(|m| {
        m.borrow_mut()
            .entry(key)
            .or_default()
            .push(row.clone().upcast())
    });
}

/// Build the content widget for a sidebar section id.
pub fn build_page(id: &str, ctx: &AppContext) -> gtk4::Widget {
    match id {
        "general" => general::build(ctx),
        "audio" => audio::build(ctx),
        "models" | "transcription" | "providers" => models::build(ctx),
        "post_processing" | "prompts" => providers_llm::build(ctx),
        "history" => history::build(ctx),
        "advanced" | "debug" | "about" => advanced::build(ctx),
        other => placeholder(other, ""),
    }
}

/// A preferences page that just shows a title and a note (interim state).
fn placeholder(title: &str, note: &str) -> gtk4::Widget {
    let page = libadwaita::PreferencesPage::new();
    page.set_title(title);
    let group = libadwaita::PreferencesGroup::new();
    group.set_title(title);
    let row = libadwaita::ActionRow::new();
    row.set_title(if note.is_empty() { title } else { note });
    row.set_activatable(false);
    group.add(&row);
    page.add(&group);
    page.upcast::<gtk4::Widget>()
}
