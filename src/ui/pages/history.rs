//! History settings page: recent transcriptions with saved/delete/retry
//! actions, refreshed through the event bus.

use crate::commands::history as history_cmds;
use crate::context::{AppContext, AppEvent};
use libadwaita::prelude::*;

/// Build the History preferences page.
pub fn build(ctx: &AppContext) -> gtk4::Widget {
    let page = libadwaita::PreferencesPage::new();
    page.set_title("History");

    // --- Retention group ---
    let retention_group = libadwaita::PreferencesGroup::new();
    retention_group.set_title("Retention");

    let limit_adjustment = gtk4::Adjustment::new(
        ctx.settings().history_limit as f64,
        1.0,
        100.0,
        1.0,
        10.0,
        0.0,
    );
    let limit_row = libadwaita::SpinRow::new(Some(&limit_adjustment), 0.0, 0);
    limit_row.set_title("History limit");
    limit_row.set_subtitle("Entries kept before the oldest are trimmed");
    limit_row.set_snap_to_ticks(true);
    limit_row.set_numeric(true);
    let limit_ctx = ctx.clone();
    limit_adjustment.connect_value_changed(move |adj| {
        let ctx = limit_ctx.clone();
        let value = adj.value() as usize;
        glib::spawn_future_local(async move {
            let _ = history_cmds::update_history_limit(&ctx, value).await;
        });
    });
    retention_group.add(&limit_row);
    page.add(&retention_group);

    // --- Entries group ---
    let entries_group = libadwaita::PreferencesGroup::new();
    entries_group.set_widget_name("history-list");
    entries_group.set_title("Recent Transcriptions");
    page.add(&entries_group);

    // Initial render + live refresh on history events.
    let ctx = ctx.clone();
    let group = entries_group.clone();
    refresh_entries(&ctx, &group);

    let group_for_events = glib::SendWeakRef::from(group.downgrade());
    let bus = ctx.bus.clone();
    bus.subscribe(move |event| {
        let ctx = ctx.clone();
        let group = group_for_events.clone();
        glib::MainContext::default().invoke(move || {
            let weak = group.into_weak_ref();
            let Some(group) = weak.upgrade() else {
                return;
            };
            if matches!(event, AppEvent::HistoryUpdated(_)) {
                refresh_entries(&ctx, &group);
            }
        });
    });

    page.upcast::<gtk4::Widget>()
}

fn refresh_entries(ctx: &AppContext, group: &libadwaita::PreferencesGroup) {
    crate::ui::pages::clear_group_rows(group);

    let ctx = ctx.clone();
    let group_weak = glib::SendWeakRef::from(group.downgrade());
    crate::runtime::spawn(async move {
        let result = history_cmds::get_history_entries(&ctx, None, Some(20)).await;
        let ctx = ctx.clone();
        let group_weak = group_weak.clone();
        glib::MainContext::default().invoke(move || {
            let Some(group) = group_weak.into_weak_ref().upgrade() else {
                return;
            };
            match result {
                Ok(paginated) => {
                    for entry in paginated.entries {
                        let row = libadwaita::ExpanderRow::new();
                        row.set_widget_name(&entry.id.to_string());
                        row.set_title(&glib::markup_escape_text(&entry.title));
                        let ts = chrono::DateTime::from_timestamp(entry.timestamp, 0)
                            .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                            .unwrap_or_else(|| "unknown time".to_string());
                        row.set_subtitle(&ts);
                        row.set_expanded(false);

                        let text = if entry.post_processed_text.is_some() {
                            entry.post_processed_text.clone().unwrap_or_default()
                        } else {
                            entry.transcription_text.clone()
                        };
                        let text_row = libadwaita::ActionRow::new();
                        text_row.set_title("Transcript");
                        text_row.set_subtitle(&glib::markup_escape_text(
                            &text.chars().take(160).collect::<String>(),
                        ));
                        text_row.set_activatable(false);
                        row.add_row(&text_row);

                        // Saved toggle.
                        let saved_row = libadwaita::SwitchRow::new();
                        saved_row.set_title("Keep this entry");
                        saved_row.set_active(entry.saved);
                        let save_ctx = ctx.clone();
                        let id = entry.id;
                        saved_row.connect_active_notify(move |_r| {
                            let ctx = save_ctx.clone();
                            let id = id;
                            crate::runtime::spawn(async move {
                                let _ = history_cmds::toggle_history_entry_saved(&ctx, id).await;
                            });
                        });
                        row.add_row(&saved_row);

                        // Retry.
                        let retry_button = gtk4::Button::with_label("Retry");
                        let retry_ctx = ctx.clone();
                        let retry_id = entry.id;
                        retry_button.connect_clicked(move |_| {
                            let ctx = retry_ctx.clone();
                            let id = retry_id;
                            crate::runtime::spawn(async move {
                                let _ =
                                    history_cmds::retry_history_entry_transcription(&ctx, id).await;
                            });
                        });
                        row.add_suffix(&retry_button);

                        // Delete.
                        let delete_button = gtk4::Button::with_label("Delete");
                        delete_button.add_css_class("destructive-action");
                        let delete_ctx = ctx.clone();
                        let delete_id = entry.id;
                        delete_button.connect_clicked(move |_| {
                            let ctx = delete_ctx.clone();
                            let id = delete_id;
                            crate::runtime::spawn(async move {
                                let _ = history_cmds::delete_history_entry(&ctx, id).await;
                            });
                        });
                        row.add_suffix(&delete_button);

                        group.add(&row);
                        crate::ui::pages::track_row(&group, &row);
                    }
                }
                Err(e) => {
                    let row = libadwaita::ActionRow::new();
                    row.set_title("Failed to load history");
                    row.set_subtitle(&e);
                    group.add(&row);
                    crate::ui::pages::track_row(&group, &row);
                }
            }
        });
    });
}
