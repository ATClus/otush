//! Todo Checklist & Task palette (`Ctrl+Alt+T`).
//!
//! Provides instant task capture and checklist toggle stored in SQLite.
//! Adheres strictly to GTK4 + Libadwaita / GNOME HIG design principles.

use crate::context::AppContext;
use gdk4::prelude::*;
use gtk4::prelude::*;
use libadwaita::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

static TODO_WINDOW: LazyLock<Mutex<Option<glib::SendWeakRef<libadwaita::Window>>>> =
    LazyLock::new(|| Mutex::new(None));

static LAST_TODO_TOGGLE: LazyLock<Mutex<Option<Instant>>> = LazyLock::new(|| Mutex::new(None));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TodoFilter {
    All,
    Active,
    Completed,
}

/// Toggle or show the quick todo palette modal.
pub fn show_todo_palette(ctx: &AppContext) {
    toggle_todo_palette(ctx);
}

/// Toggle display of the quick todo palette modal.
pub fn toggle_todo_palette(ctx: &AppContext) {
    let now = Instant::now();
    if let Ok(mut last) = LAST_TODO_TOGGLE.lock() {
        if let Some(prev) = *last {
            if now.duration_since(prev) < Duration::from_millis(300) {
                return;
            }
        }
        *last = Some(now);
    }

    let ctx = ctx.clone();
    glib::MainContext::default().invoke(move || {
        let existing_win = {
            let mut guard = match TODO_WINDOW.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard.take().and_then(|w| w.into_weak_ref().upgrade())
        };

        if let Some(win) = existing_win {
            if !win.in_destruction() {
                if win.is_visible() {
                    win.close();
                } else {
                    win.present();
                    if let Ok(mut guard) = TODO_WINDOW.lock() {
                        *guard = Some(glib::SendWeakRef::from(win.downgrade()));
                    }
                }
                return;
            }
        }

        build_and_present_todo_palette(&ctx);
    });
}

fn format_task_title(task: &str, completed: bool) -> String {
    let escaped = glib::markup_escape_text(task);
    if completed {
        format!("<s>{escaped}</s>")
    } else {
        escaped.to_string()
    }
}

fn update_header_stats(
    ctx: &AppContext,
    win_title: &libadwaita::WindowTitle,
    clear_btn: &gtk4::Button,
) -> (usize, usize, usize) {
    let todos = ctx.history.list_todos(true).unwrap_or_default();
    let total = todos.len();
    let completed = todos.iter().filter(|t| t.completed).count();
    let active = total.saturating_sub(completed);

    let subtitle = if total == 0 {
        "0 tasks".to_string()
    } else if active == 0 {
        format!("All {total} tasks completed")
    } else {
        format!("{active} active • {completed} completed")
    };

    win_title.set_subtitle(&subtitle);
    clear_btn.set_sensitive(completed > 0);
    if completed > 0 {
        clear_btn.set_tooltip_text(Some(&format!("Clear {completed} completed task(s)")));
    } else {
        clear_btn.set_tooltip_text(Some("No completed tasks to clear"));
    }

    (total, active, completed)
}

fn build_and_present_todo_palette(ctx: &AppContext) {
    let window = libadwaita::Window::new();
    window.set_title(Some("Tasks & Checklist"));
    window.set_default_size(560, 480);
    window.set_modal(true);
    window.set_resizable(true);
    window.set_deletable(true);
    window.add_css_class("dialog");

    {
        let mut guard = match TODO_WINDOW.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        *guard = Some(glib::SendWeakRef::from(window.downgrade()));
    }

    let toast_overlay = libadwaita::ToastOverlay::new();
    let main_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    toast_overlay.set_child(Some(&main_box));
    window.set_content(Some(&toast_overlay));

    // Header bar
    let header_bar = libadwaita::HeaderBar::new();
    header_bar.set_show_end_title_buttons(true);

    let win_title = libadwaita::WindowTitle::new("Tasks & Checklist", "Loading tasks…");
    header_bar.set_title_widget(Some(&win_title));

    // Clear completed button in header
    let clear_btn = gtk4::Button::from_icon_name("edit-clear-all-symbolic");
    clear_btn.set_tooltip_text(Some("Clear completed tasks"));
    clear_btn.add_css_class("flat");
    header_bar.pack_start(&clear_btn);

    main_box.append(&header_bar);

    // Filter state
    let active_filter = Rc::new(RefCell::new(TodoFilter::All));

    // Controls container
    let controls_box = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
    controls_box.set_margin_start(16);
    controls_box.set_margin_end(16);
    controls_box.set_margin_top(12);
    controls_box.set_margin_bottom(8);
    main_box.append(&controls_box);

    // Task capture row
    let input_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let task_entry = gtk4::Entry::new();
    task_entry.set_hexpand(true);
    task_entry.set_placeholder_text(Some("Add a new task… (Press Enter)"));
    task_entry.set_icon_from_icon_name(
        gtk4::EntryIconPosition::Primary,
        Some("checkbox-checked-symbolic"),
    );
    input_row.append(&task_entry);

    let add_btn = gtk4::Button::from_icon_name("list-add-symbolic");
    add_btn.set_tooltip_text(Some("Add Task"));
    add_btn.add_css_class("suggested-action");
    input_row.append(&add_btn);
    controls_box.append(&input_row);

    // Segmented filter control
    let filter_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    filter_row.add_css_class("linked");
    filter_row.set_halign(gtk4::Align::Center);

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

    let act_btn = gtk4::ToggleButton::new();
    act_btn.set_group(Some(&all_btn));
    let act_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let act_icon = gtk4::Image::from_icon_name("radio-checked-symbolic");
    act_icon.set_pixel_size(16);
    act_box.append(&act_icon);
    let act_lbl = gtk4::Label::new(Some("Active"));
    act_box.append(&act_lbl);
    act_btn.set_child(Some(&act_box));
    filter_row.append(&act_btn);

    let comp_btn = gtk4::ToggleButton::new();
    comp_btn.set_group(Some(&all_btn));
    let comp_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let comp_icon = gtk4::Image::from_icon_name("emblem-ok-symbolic");
    comp_icon.set_pixel_size(16);
    comp_box.append(&comp_icon);
    let comp_lbl = gtk4::Label::new(Some("Completed"));
    comp_box.append(&comp_lbl);
    comp_btn.set_child(Some(&comp_box));
    filter_row.append(&comp_btn);

    controls_box.append(&filter_row);

    // Multi-State View (Stack)
    let stack = gtk4::Stack::new();
    stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
    stack.set_transition_duration(200);
    stack.set_vexpand(true);
    stack.set_hexpand(true);

    // State 1: Entries List inside ScrolledWindow
    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_hexpand(true);
    scrolled.set_min_content_height(260);

    let list_box = gtk4::ListBox::new();
    list_box.set_selection_mode(gtk4::SelectionMode::None);
    list_box.add_css_class("boxed-list");
    list_box.set_margin_start(16);
    list_box.set_margin_end(16);
    list_box.set_margin_bottom(16);
    scrolled.set_child(Some(&list_box));
    stack.add_named(&scrolled, Some("entries"));

    // State 2: StatusPage (Empty)
    let status_page = libadwaita::StatusPage::new();
    status_page.set_icon_name(Some("checkbox-checked-symbolic"));
    status_page.set_title("No Tasks");
    status_page.set_description(Some("Type a task above and press Enter to get started."));
    stack.add_named(&status_page, Some("empty"));

    main_box.append(&stack);

    // Helper to refresh task list
    let refresh_tasks = {
        let ctx = ctx.clone();
        let list = list_box.clone();
        let stack = stack.clone();
        let status_page = status_page.clone();
        let active_filter = active_filter.clone();
        let win_title = win_title.clone();
        let clear_btn = clear_btn.clone();
        let toast_overlay = toast_overlay.clone();

        Rc::new(move || {
            let (_, _, _) = update_header_stats(&ctx, &win_title, &clear_btn);

            // Remove existing rows
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }

            let current_filter = *active_filter.borrow();

            if let Ok(todos) = ctx.history.list_todos(true) {
                let filtered: Vec<_> = todos
                    .into_iter()
                    .filter(|t| match current_filter {
                        TodoFilter::All => true,
                        TodoFilter::Active => !t.completed,
                        TodoFilter::Completed => t.completed,
                    })
                    .collect();

                if filtered.is_empty() {
                    match current_filter {
                        TodoFilter::All => {
                            status_page.set_icon_name(Some("checkbox-checked-symbolic"));
                            status_page.set_title("No Tasks Yet");
                            status_page.set_description(Some(
                                "Type a task above and press Enter to begin your checklist.",
                            ));
                        }
                        TodoFilter::Active => {
                            status_page.set_icon_name(Some("emblem-ok-symbolic"));
                            status_page.set_title("All Caught Up!");
                            status_page.set_description(Some(
                                "No active tasks pending. Enjoy your day or add a new one above!",
                            ));
                        }
                        TodoFilter::Completed => {
                            status_page.set_icon_name(Some("checkbox-checked-symbolic"));
                            status_page.set_title("No Completed Tasks");
                            status_page.set_description(Some(
                                "Completed tasks will be recorded here as you finish them.",
                            ));
                        }
                    }
                    stack.set_visible_child_name("empty");
                    return;
                }

                stack.set_visible_child_name("entries");

                for todo in filtered {
                    let row = libadwaita::ActionRow::new();
                    row.set_use_markup(true);
                    row.set_title(&format_task_title(&todo.task, todo.completed));

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

                    let check = gtk4::CheckButton::new();
                    check.set_active(todo.completed);
                    check.set_valign(gtk4::Align::Center);

                    if todo.completed {
                        row.add_css_class("dim-label");
                    }

                    let ctx_toggle = ctx.clone();
                    let todo_id = todo.id;
                    let task_text = todo.task.clone();
                    let row_weak = glib::SendWeakRef::from(row.downgrade());
                    let win_title_toggle = win_title.clone();
                    let clear_btn_toggle = clear_btn.clone();

                    check.connect_toggled(move |cb| {
                        let is_checked = cb.is_active();
                        let _ = ctx_toggle.history.toggle_todo(todo_id);
                        if let Some(r) = row_weak.clone().into_weak_ref().upgrade() {
                            r.set_title(&format_task_title(&task_text, is_checked));
                            if is_checked {
                                r.add_css_class("dim-label");
                            } else {
                                r.remove_css_class("dim-label");
                            }
                        }
                        update_header_stats(&ctx_toggle, &win_title_toggle, &clear_btn_toggle);
                    });

                    row.add_prefix(&check);
                    row.set_activatable_widget(Some(&check));

                    // Copy button
                    let copy_btn = create_copy_button(&ctx, &todo.task, &toast_overlay);
                    row.add_suffix(&copy_btn);

                    // Delete button
                    let del_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
                    del_btn.set_tooltip_text(Some("Delete task"));
                    del_btn.add_css_class("flat");
                    del_btn.set_valign(gtk4::Align::Center);

                    let ctx_del = ctx.clone();
                    let del_id = todo.id;
                    let list_weak = glib::SendWeakRef::from(list.downgrade());
                    let row_del_weak = glib::SendWeakRef::from(row.downgrade());
                    let stack_del = stack.clone();
                    let toast_del = toast_overlay.clone();
                    let win_title_del = win_title.clone();
                    let clear_btn_del = clear_btn.clone();

                    del_btn.connect_clicked(move |_| {
                        let _ = ctx_del.history.delete_todo(del_id);
                        if let (Some(l), Some(r)) = (
                            list_weak.clone().into_weak_ref().upgrade(),
                            row_del_weak.clone().into_weak_ref().upgrade(),
                        ) {
                            l.remove(&r);
                            if l.first_child().is_none() {
                                stack_del.set_visible_child_name("empty");
                            }
                        }
                        update_header_stats(&ctx_del, &win_title_del, &clear_btn_del);
                        toast_del.add_toast(libadwaita::Toast::new("Task deleted"));
                    });
                    row.add_suffix(&del_btn);

                    list.append(&row);
                }
            }
        })
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
    act_btn.connect_toggled(move |btn| {
        if btn.is_active() {
            *filter_act.borrow_mut() = TodoFilter::Active;
            ref_act();
        }
    });

    let ref_comp = refresh_tasks.clone();
    let filter_comp = active_filter.clone();
    comp_btn.connect_toggled(move |btn| {
        if btn.is_active() {
            *filter_comp.borrow_mut() = TodoFilter::Completed;
            ref_comp();
        }
    });

    // Add task action
    let add_task = {
        let ctx = ctx.clone();
        let entry = task_entry.clone();
        let refresh = refresh_tasks.clone();
        let toast = toast_overlay.clone();
        move || {
            let text = entry.text().to_string();
            if !text.trim().is_empty() {
                if let Ok(_todo) = ctx.history.save_todo(text, 0, None) {
                    entry.set_text("");
                    toast.add_toast(libadwaita::Toast::new("Task added"));
                    refresh();
                }
            }
        }
    };

    let add_task_click = add_task.clone();
    add_btn.connect_clicked(move |_| {
        add_task_click();
    });

    task_entry.connect_activate(move |_| {
        add_task();
    });

    // Clear completed action
    let clear_completed = {
        let ctx = ctx.clone();
        let refresh = refresh_tasks.clone();
        let toast = toast_overlay.clone();
        move || {
            if let Ok(count) = ctx.history.clear_completed_todos() {
                if count > 0 {
                    toast.add_toast(libadwaita::Toast::new(&format!(
                        "Cleared {count} completed task(s)"
                    )));
                    refresh();
                }
            }
        }
    };

    clear_btn.connect_clicked(move |_| {
        clear_completed();
    });

    // Close on Escape
    let key_controller = gtk4::EventControllerKey::new();
    let win_weak = glib::SendWeakRef::from(window.downgrade());
    key_controller.connect_key_pressed(move |_, key, _, _| {
        if key == gdk4::Key::Escape {
            if let Some(w) = win_weak.clone().into_weak_ref().upgrade() {
                w.close();
                return glib::Propagation::Stop;
            }
        }
        glib::Propagation::Proceed
    });
    window.add_controller(key_controller);

    window.connect_destroy(|_| {
        if let Ok(mut guard) = TODO_WINDOW.lock() {
            *guard = None;
        }
    });

    task_entry.grab_focus();
    window.present();
}

fn create_copy_button(
    ctx: &AppContext,
    text: &str,
    toast_overlay: &libadwaita::ToastOverlay,
) -> gtk4::Button {
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
    let toast_weak = glib::SendWeakRef::from(toast_overlay.downgrade());

    btn.connect_clicked(move |_| {
        let _ = crate::clipboard::write_clipboard_text(&ctx, &text);
        if let Some(display) = gdk4::Display::default() {
            display.clipboard().set_text(&text);
        }
        if let Some(btn) = btn_weak.clone().into_weak_ref().upgrade() {
            btn.set_icon_name("object-select-symbolic");
            btn.set_tooltip_text(Some("Copied!"));
            let btn_reset = glib::SendWeakRef::from(btn.downgrade());
            glib::timeout_add_local_once(Duration::from_millis(1500), move || {
                if let Some(btn) = btn_reset.into_weak_ref().upgrade() {
                    btn.set_icon_name("edit-copy-symbolic");
                    btn.set_tooltip_text(Some("Copy task text"));
                }
            });
        }
        if let Some(toast) = toast_weak.clone().into_weak_ref().upgrade() {
            toast.add_toast(libadwaita::Toast::new("Task copied to clipboard"));
        }
    });
    btn
}
