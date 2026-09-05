//! Tasks & Todo List management page.

use crate::context::AppContext;
use gdk4::prelude::*;
use gtk4::prelude::*;
use libadwaita::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TodoFilter {
    All,
    Active,
    Completed,
}

/// Build the Todos preferences page.
pub fn build(ctx: &AppContext) -> gtk4::Widget {
    let page = libadwaita::PreferencesPage::new();
    page.set_title("Tasks &amp; Todos");
    page.set_icon_name(Some("checkbox-checked-symbolic"));

    // Filter state
    let active_filter = Rc::new(RefCell::new(TodoFilter::All));

    // ========================================================================
    // 1. Filter & Quick Capture Group (Top - identical to history and notes)
    // ========================================================================
    let filter_group = libadwaita::PreferencesGroup::new();
    filter_group.set_title("Filter &amp; Add Task");
    filter_group.set_hexpand(true);

    // New task input row
    let input_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    input_box.set_margin_top(4);
    input_box.set_margin_bottom(6);
    input_box.set_hexpand(true);

    let task_entry = gtk4::Entry::new();
    task_entry.set_hexpand(true);
    task_entry.set_placeholder_text(Some("Add a new task… (Press Enter)"));
    input_box.append(&task_entry);

    let add_btn = gtk4::Button::from_icon_name("list-add-symbolic");
    add_btn.set_tooltip_text(Some("Add Task"));
    add_btn.add_css_class("suggested-action");
    input_box.append(&add_btn);
    filter_group.add(&input_box);

    // Filter controls and Clear Completed bar (placed on top, above the list)
    let controls_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    controls_box.set_margin_top(2);
    controls_box.set_margin_bottom(6);
    controls_box.set_hexpand(true);

    let filter_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    filter_row.add_css_class("linked");
    filter_row.set_halign(gtk4::Align::Start);

    let all_btn = gtk4::ToggleButton::new();
    all_btn.set_active(true);
    let all_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let all_icon = gtk4::Image::from_icon_name("view-grid-symbolic");
    all_icon.set_pixel_size(16);
    all_box.append(&all_icon);
    let all_lbl = gtk4::Label::new(Some("All"));
    all_box.append(&all_lbl);
    all_btn.set_child(Some(&all_box));
    filter_row.append(&all_btn);

    let active_btn = gtk4::ToggleButton::new();
    active_btn.set_group(Some(&all_btn));
    let active_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let active_icon = gtk4::Image::from_icon_name("radio-checked-symbolic");
    active_icon.set_pixel_size(16);
    active_box.append(&active_icon);
    let active_lbl = gtk4::Label::new(Some("Active"));
    active_box.append(&active_lbl);
    active_btn.set_child(Some(&active_box));
    filter_row.append(&active_btn);

    let completed_btn = gtk4::ToggleButton::new();
    completed_btn.set_group(Some(&all_btn));
    let comp_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let comp_icon = gtk4::Image::from_icon_name("emblem-ok-symbolic");
    comp_icon.set_pixel_size(16);
    comp_box.append(&comp_icon);
    let comp_lbl = gtk4::Label::new(Some("Completed"));
    comp_box.append(&comp_lbl);
    completed_btn.set_child(Some(&comp_box));
    filter_row.append(&completed_btn);

    controls_box.append(&filter_row);

    // Clear completed button
    let clear_completed_btn = gtk4::Button::new();
    let clear_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let clear_icon = gtk4::Image::from_icon_name("edit-clear-all-symbolic");
    clear_icon.set_pixel_size(16);
    clear_box.append(&clear_icon);
    let clear_lbl = gtk4::Label::new(Some("Clear Completed"));
    clear_box.append(&clear_lbl);
    clear_completed_btn.set_child(Some(&clear_box));
    clear_completed_btn.add_css_class("flat");
    clear_completed_btn.set_hexpand(true);
    clear_completed_btn.set_halign(gtk4::Align::End);
    controls_box.append(&clear_completed_btn);

    filter_group.add(&controls_box);

    // Quick Tasks Palette Action Row
    let palette_row = libadwaita::ActionRow::new();
    palette_row.set_title("Quick Tasks Palette");
    palette_row.set_subtitle(
        "Capture todos and checklist items instantly from anywhere (Shortcut: Ctrl+Alt+T)",
    );
    palette_row.set_activatable(true);

    let palette_icon = gtk4::Image::from_icon_name("window-new-symbolic");
    palette_row.add_prefix(&palette_icon);

    let palette_btn = gtk4::Button::from_icon_name("window-new-symbolic");
    palette_btn.set_tooltip_text(Some("Open Quick Tasks Palette"));
    palette_btn.set_valign(gtk4::Align::Center);
    palette_btn.add_css_class("flat");
    palette_row.add_suffix(&palette_btn);

    let ctx_palette = ctx.clone();
    palette_row.connect_activated(move |_| {
        crate::ui::todo_palette::show_todo_palette(&ctx_palette);
    });
    let ctx_palette_btn = ctx.clone();
    palette_btn.connect_clicked(move |_| {
        crate::ui::todo_palette::show_todo_palette(&ctx_palette_btn);
    });

    filter_group.add(&palette_row);
    page.add(&filter_group);

    // ========================================================================
    // 2. Tasks & Checklist Group (Center - purely rows)
    // ========================================================================
    let list_group = libadwaita::PreferencesGroup::new();
    list_group.set_widget_name("todos-list");
    list_group.set_title("Tasks &amp; Checklist");
    list_group.set_description(Some(
        "Manage everyday action items and voice-transcribed tasks.",
    ));
    list_group.set_hexpand(true);
    page.add(&list_group);

    // Refresh function directly rendering into list_group (identical to history and notes)
    let rows = Arc::new(Mutex::new(crate::ui::pages::PageGroup::new()));
    let refresh_tasks = {
        let ctx = ctx.clone();
        let group = list_group.clone();
        let filter = active_filter.clone();
        let rows = rows.clone();

        move || {
            rows.lock().unwrap_or_else(|e| e.into_inner()).clear(&group);

            let current_filter = *filter.borrow();

            if let Ok(todos) = ctx.history.list_todos(true) {
                let filtered_todos: Vec<_> = todos
                    .into_iter()
                    .filter(|t| match current_filter {
                        TodoFilter::All => true,
                        TodoFilter::Active => !t.completed,
                        TodoFilter::Completed => t.completed,
                    })
                    .collect();

                if filtered_todos.is_empty() {
                    let empty_row = libadwaita::ActionRow::new();
                    let (title, subtitle) = match current_filter {
                        TodoFilter::Completed => (
                            "No completed tasks",
                            "Complete tasks by checking their box to see them here.",
                        ),
                        TodoFilter::Active => (
                            "No active tasks",
                            "All tasks are complete! Add a new task above or press Ctrl+Alt+T.",
                        ),
                        TodoFilter::All => (
                            "No tasks found",
                            "Add a new task above or press Ctrl+Alt+T to capture tasks anytime.",
                        ),
                    };
                    empty_row.set_title(title);
                    empty_row.set_subtitle(subtitle);
                    let empty_icon = gtk4::Image::from_icon_name("checkbox-checked-symbolic");
                    empty_row.add_prefix(&empty_icon);
                    empty_row.set_activatable(false);
                    rows.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .add(&group, &empty_row);
                    return;
                }

                for todo in filtered_todos {
                    let row = libadwaita::ActionRow::new();
                    row.set_use_markup(false);
                    row.set_title(&todo.task);

                    // Formatted timestamp subtitle
                    let date_str = if todo.completed && todo.completed_at.is_some() {
                        todo.completed_at
                            .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
                            .map(|t| {
                                let local = t.with_timezone(&chrono::Local);
                                format!("Completed on {}", local.format("%Y-%m-%d %H:%M"))
                            })
                            .unwrap_or_else(|| "Completed".to_string())
                    } else {
                        chrono::DateTime::from_timestamp(todo.created_at, 0)
                            .map(|t| {
                                let local = t.with_timezone(&chrono::Local);
                                format!("Added {}", local.format("%Y-%m-%d %H:%M"))
                            })
                            .unwrap_or_else(|| "Active".to_string())
                    };

                    let full_subtitle = if let Some(ref due) = todo.due_date {
                        format!("{} • Due: {}", date_str, due)
                    } else {
                        date_str
                    };
                    row.set_subtitle(&full_subtitle);

                    // CheckButton prefix
                    let check = gtk4::CheckButton::new();
                    check.set_active(todo.completed);
                    check.set_valign(gtk4::Align::Center);

                    if todo.completed {
                        row.add_css_class("dim-label");
                    }

                    let ctx_toggle = ctx.clone();
                    let todo_id = todo.id;
                    let row_weak = glib::SendWeakRef::from(row.downgrade());
                    check.connect_toggled(move |cb| {
                        let _ = ctx_toggle.history.toggle_todo(todo_id);
                        if let Some(r) = row_weak.clone().into_weak_ref().upgrade() {
                            if cb.is_active() {
                                r.add_css_class("dim-label");
                            } else {
                                r.remove_css_class("dim-label");
                            }
                        }
                    });
                    row.add_prefix(&check);

                    // Copy task text button with visual confirmation
                    let copy_btn = create_copy_button(&ctx, &todo.task);
                    row.add_suffix(&copy_btn);

                    // Delete button
                    let del_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
                    del_btn.set_tooltip_text(Some("Delete task"));
                    del_btn.add_css_class("flat");
                    del_btn.set_valign(gtk4::Align::Center);
                    let ctx_del = ctx.clone();
                    let del_id = todo.id;
                    let group_weak = glib::SendWeakRef::from(group.downgrade());
                    let row_del_weak = glib::SendWeakRef::from(row.downgrade());
                    del_btn.connect_clicked(move |_| {
                        let _ = ctx_del.history.delete_todo(del_id);
                        if let (Some(g), Some(r)) = (
                            group_weak.clone().into_weak_ref().upgrade(),
                            row_del_weak.clone().into_weak_ref().upgrade(),
                        ) {
                            g.remove(&r);
                        }
                    });
                    row.add_suffix(&del_btn);

                    rows.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .add(&group, &row);
                }
            }
        }
    };

    refresh_tasks();

    // Wire segmented filter buttons
    let ref_all = refresh_tasks.clone();
    let filter_all = active_filter.clone();
    all_btn.connect_toggled(move |btn| {
        if btn.is_active() {
            *filter_all.borrow_mut() = TodoFilter::All;
            ref_all();
        }
    });

    let ref_act = refresh_tasks.clone();
    let filter_act = active_filter.clone();
    active_btn.connect_toggled(move |btn| {
        if btn.is_active() {
            *filter_act.borrow_mut() = TodoFilter::Active;
            ref_act();
        }
    });

    let ref_comp = refresh_tasks.clone();
    let filter_comp = active_filter.clone();
    completed_btn.connect_toggled(move |btn| {
        if btn.is_active() {
            *filter_comp.borrow_mut() = TodoFilter::Completed;
            ref_comp();
        }
    });

    // Add task logic
    let add_task = {
        let ctx = ctx.clone();
        let entry = task_entry.clone();
        let refresh = refresh_tasks.clone();
        move || {
            let text = entry.text().to_string();
            if !text.trim().is_empty() && ctx.history.save_todo(text, 0, None).is_ok() {
                entry.set_text("");
                refresh();
            }
        }
    };

    let add_click = add_task.clone();
    add_btn.connect_clicked(move |_| {
        add_click();
    });

    task_entry.connect_activate(move |_| {
        add_task();
    });

    // Clear completed logic
    let clear_action = {
        let ctx = ctx.clone();
        let refresh = refresh_tasks.clone();
        move || {
            let _ = ctx.history.clear_completed_todos();
            refresh();
        }
    };

    clear_completed_btn.connect_clicked(move |_| {
        clear_action();
    });

    page.upcast::<gtk4::Widget>()
}

fn create_copy_button(ctx: &AppContext, text: &str) -> gtk4::Button {
    let btn = gtk4::Button::from_icon_name("edit-copy-symbolic");
    btn.set_tooltip_text(Some("Copy task text"));
    btn.set_valign(gtk4::Align::Center);
    btn.add_css_class("flat");

    if text.trim().is_empty() {
        btn.set_sensitive(false);
        btn.set_tooltip_text(Some("Task is empty"));
        return btn;
    }

    let ctx = ctx.clone();
    let text = text.to_string();
    let btn_weak = glib::SendWeakRef::from(btn.downgrade());
    btn.connect_clicked(move |_| {
        let _ = crate::clipboard::write_clipboard_text(&ctx, &text);
        if let Some(display) = gdk4::Display::default() {
            display.clipboard().set_text(&text);
        }
        if let Some(btn) = btn_weak.clone().into_weak_ref().upgrade() {
            btn.set_icon_name("object-select-symbolic");
            btn.set_tooltip_text(Some("Copied!"));
            let btn_reset = glib::SendWeakRef::from(btn.downgrade());
            glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
                if let Some(btn) = btn_reset.into_weak_ref().upgrade() {
                    btn.set_icon_name("edit-copy-symbolic");
                    btn.set_tooltip_text(Some("Copy task text"));
                }
            });
        }
    });
    btn
}
