//! Models settings page — model list with download / delete / set-active
//! controls, updated live through the event bus.

use crate::commands::models as model_cmds;
use crate::context::{AppContext, AppEvent};
use libadwaita::prelude::*;

/// Build the Models preferences page.
pub fn build(ctx: &AppContext) -> gtk4::Widget {
    let page = libadwaita::PreferencesPage::new();
    page.set_title("Models");

    let group = libadwaita::PreferencesGroup::new();
    group.set_widget_name("models-list");
    group.set_title("Available Models");
    group.set_description(Some(
        "Download a model to start transcribing; the recommended model is marked.",
    ));
    page.add(&group);

    // Initial render.
    rebuild_group(ctx, &group);

    // Live updates: rebuild on lifecycle events, patch progress bars on
    // download progress. All events are marshaled onto the GTK main loop, so
    // the widgets are only ever touched on the main thread.
    let ctx = ctx.clone();
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
            match event {
                AppEvent::ModelsUpdated
                | AppEvent::ModelDownloadComplete(_)
                | AppEvent::ModelDownloadCancelled(_)
                | AppEvent::ModelDownloadFailed { .. }
                | AppEvent::ModelDeleted(_)
                | AppEvent::ModelVerificationStarted(_)
                | AppEvent::ModelVerificationCompleted(_)
                | AppEvent::ModelExtractionStarted(_)
                | AppEvent::ModelExtractionCompleted(_)
                | AppEvent::ModelStateChanged(_) => rebuild_group(&ctx, &group),
                AppEvent::ModelDownloadProgress(progress) => {
                    update_progress(&group, &progress);
                }
                _ => {}
            }
        });
    });

    page.upcast::<gtk4::Widget>()
}

/// Rebuild the model list from the current manager state. Called on the main
/// thread (initial render and on lifecycle events).
fn rebuild_group(ctx: &AppContext, group: &libadwaita::PreferencesGroup) {
    crate::ui::pages::clear_group_rows(group);

    let settings = ctx.settings();
    let selected = settings.selected_model.clone();
    let loaded_model = ctx.transcription.get_current_model();
    let models = ctx.model.get_available_models();

    // Loaded-model status row (live via ModelStateChanged rebuilds).
    let status_row = libadwaita::ActionRow::new();
    status_row.set_widget_name("model-status");
    match &loaded_model {
        Some(id) => {
            status_row.set_title("Loaded model");
            status_row.set_subtitle(id);
            let unload_button = gtk4::Button::with_label("Unload");
            let unload_ctx = ctx.clone();
            unload_button.connect_clicked(move |_| {
                let _ = crate::commands::transcription::unload_model_manually(&unload_ctx);
            });
            status_row.add_suffix(&unload_button);
        }
        None => {
            status_row.set_title("No model loaded");
            status_row.set_subtitle("A model loads automatically when you start recording");
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

        // Status line.
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

        // Progress bar while downloading.
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

        // Actions.
        let ctx = ctx.clone();
        if model.is_downloaded {
            let is_loaded = loaded_model.as_deref() == Some(model.id.as_str());
            if !is_loaded {
                // Load (sets active + loads the model).
                let load_button = gtk4::Button::with_label("Load");
                load_button.add_css_class("suggested-action");
                let load_ctx = ctx.clone();
                let id = model.id.clone();
                load_button.connect_clicked(move |_| {
                    let ctx = load_ctx.clone();
                    let id = id.clone();
                    crate::runtime::spawn(async move {
                        let _ = model_cmds::set_active_model(&ctx, id).await;
                    });
                });
                row.add_suffix(&load_button);
            }
            // Delete.
            let delete_button = gtk4::Button::with_label("Delete");
            delete_button.add_css_class("destructive-action");
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
            // Download.
            let download_button = gtk4::Button::with_label("Download");
            download_button.add_css_class("suggested-action");
            let dl_ctx = ctx.clone();
            let id = model.id.clone();
            download_button.connect_clicked(move |_| {
                let ctx = dl_ctx.clone();
                let id = id.clone();
                crate::runtime::spawn(async move {
                    let _ = model_cmds::download_model(&ctx, id).await;
                });
            });
            row.add_suffix(&download_button);
        }

        group.add(&row);
        crate::ui::pages::track_row(group, &row);
    }
}

/// Patch the progress bar of the row matching `model_id`.
fn update_progress(
    group: &libadwaita::PreferencesGroup,
    progress: &crate::managers::model::DownloadProgress,
) {
    let Some(row) = find_row(group, &progress.model_id) else {
        // The row may not exist yet (e.g. event raced a rebuild) — full refresh.
        return;
    };
    let total = progress.total.max(1) as f64;
    let fraction = (progress.downloaded as f64 / total).clamp(0.0, 1.0);
    // The progress bar is the last added row child.
    let mut bar: Option<gtk4::ProgressBar> = None;
    let mut child = row.first_child();
    while let Some(widget) = child {
        if let Some(b) = widget.downcast_ref::<gtk4::ProgressBar>() {
            bar = Some(b.clone());
        }
        child = widget.next_sibling();
    }
    if let Some(bar) = bar {
        bar.set_fraction(fraction);
    }
}

fn find_row(
    group: &libadwaita::PreferencesGroup,
    model_id: &str,
) -> Option<libadwaita::ExpanderRow> {
    let mut child = group.first_child();
    while let Some(widget) = child {
        if let Some(row) = widget.downcast_ref::<libadwaita::ExpanderRow>() {
            if row.widget_name() == model_id {
                return Some(row.clone());
            }
        }
        child = widget.next_sibling();
    }
    None
}

/// A small pill badge label (e.g. "Recommended" / "Active").
fn badge(text: &str) -> gtk4::Widget {
    let label = gtk4::Label::new(Some(text));
    label.add_css_class("badge");
    label.upcast::<gtk4::Widget>()
}
