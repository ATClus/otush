//! Settings pages, one per sidebar section.

pub mod advanced;
pub mod agents;
pub mod audio;
pub mod docs;
pub mod general;
pub mod history;
pub mod models;
pub mod notes;
pub mod providers;
pub mod providers_llm;
pub mod research;
pub mod todos;

use crate::context::AppContext;
use libadwaita::prelude::*;

/// Owned row list for one [`libadwaita::PreferencesGroup`].
///
/// `AdwPreferencesGroup` wraps rows internally, so generic child iteration
/// cannot find them for removal on rebuild. `PageGroup` keeps the rows it
/// added and removes exactly those on [`PageGroup::clear`] — no global map,
/// no string keys, no cross-group aliasing. Rows are held as
/// [`glib::SendWeakRef`] so the handle stays `Send + Sync` across the
/// event-bus boundary and never pins a removed row alive; the removal itself
/// always runs on the GTK main thread. Keep one per rebuilt group next to
/// the group itself (usually in an `Arc<Mutex<…>>` shared with the bus
/// callbacks that trigger rebuilds).
#[derive(Default)]
pub struct PageGroup {
    rows: Vec<glib::SendWeakRef<gtk4::Widget>>,
}

impl PageGroup {
    pub fn new() -> Self {
        Self::default()
    }

    /// Remove all previously added rows from `group`. Must run on the GTK
    /// main thread (it upgrades widget refs). Dead rows are skipped.
    pub fn clear(&mut self, group: &libadwaita::PreferencesGroup) {
        for row in self.rows.drain(..) {
            if let Some(widget) = row.into_weak_ref().upgrade() {
                group.remove(&widget);
            }
        }
    }

    /// Add `row` to `group` and remember it for the next [`PageGroup::clear`].
    pub fn add(&mut self, group: &libadwaita::PreferencesGroup, row: &impl IsA<gtk4::Widget>) {
        group.add(row);
        self.rows.push(glib::SendWeakRef::from(
            row.clone().upcast::<gtk4::Widget>().downgrade(),
        ));
    }
}

/// Build the content widget for a sidebar section id.
pub fn build_page(id: &str, ctx: &AppContext) -> gtk4::Widget {
    match id {
        "general" => general::build(ctx),
        "audio" => audio::build(ctx),
        "models" | "transcription" | "speech" => models::build(ctx),
        "post_processing" | "prompts" | "ai" => providers_llm::build(ctx),
        "providers" | "cloud" => providers::build(ctx),
        "agents" => agents::build(ctx),
        "notes" => notes::build(ctx),
        "todos" => todos::build(ctx),
        "docs" => docs::build(ctx),
        "research" => research::build(ctx),
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
