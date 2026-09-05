//! Main window: navigation shell (sidebar + content stack) hosting the
//! workspace and settings pages, with a toast overlay and hide-to-tray behavior.

use crate::context::AppContext;
use gtk4::prelude::*;
use libadwaita::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SectionCategory {
    Workspace,
    Preferences,
}

/// Sidebar section metadata.
struct Section {
    id: &'static str,
    title: &'static str,
    icon: &'static str,
    category: SectionCategory,
}

const SECTIONS: &[Section] = &[
    // ========================================================================
    // WORKSPACE & CAPTURE
    // ========================================================================
    Section {
        id: "history",
        title: "History",
        icon: "document-open-recent-symbolic",
        category: SectionCategory::Workspace,
    },
    Section {
        id: "notes",
        title: "Notes & Ideas",
        icon: "text-editor-symbolic",
        category: SectionCategory::Workspace,
    },
    Section {
        id: "todos",
        title: "Tasks & Todos",
        icon: "checkbox-checked-symbolic",
        category: SectionCategory::Workspace,
    },
    Section {
        id: "docs",
        title: "Documents & OCR",
        icon: "x-office-document-symbolic",
        category: SectionCategory::Workspace,
    },
    Section {
        id: "research",
        title: "Search & Research",
        icon: "system-search-symbolic",
        category: SectionCategory::Workspace,
    },
    // ========================================================================
    // PREFERENCES & SYSTEM
    // ========================================================================
    Section {
        id: "general",
        title: "General",
        icon: "preferences-other-symbolic",
        category: SectionCategory::Preferences,
    },
    Section {
        id: "audio",
        title: "Audio & Voice",
        icon: "audio-input-microphone-symbolic",
        category: SectionCategory::Preferences,
    },
    Section {
        id: "models",
        title: "Speech Recognition",
        icon: "audio-speakers-symbolic",
        category: SectionCategory::Preferences,
    },
    Section {
        id: "post_processing",
        title: "AI & Prompts",
        icon: "starred-symbolic",
        category: SectionCategory::Preferences,
    },
    Section {
        id: "providers",
        title: "Cloud Providers",
        icon: "network-server-symbolic",
        category: SectionCategory::Preferences,
    },
    Section {
        id: "advanced",
        title: "Advanced & About",
        icon: "preferences-system-symbolic",
        category: SectionCategory::Preferences,
    },
];

/// Builds the main window. Returns the window and its toast overlay (used by
/// the event-bus subscriber). The caller keeps the window alive for the
/// process lifetime.
pub fn build_main_window(
    app: &libadwaita::Application,
    ctx: &AppContext,
) -> (libadwaita::ApplicationWindow, libadwaita::ToastOverlay) {
    let window = libadwaita::ApplicationWindow::new(app);
    window.set_title(Some("Otush"));
    window.set_default_size(840, 580);
    window.set_size_request(520, 400);

    // Toast overlay wraps the entire split view.
    let toast_overlay = libadwaita::ToastOverlay::new();
    window.set_content(Some(&toast_overlay));

    // Navigation split view (sidebar + content), native GNOME HIG navigation.
    let split = libadwaita::NavigationSplitView::new();
    split.set_min_sidebar_width(220.0);
    split.set_max_sidebar_width(280.0);

    // --- SIDEBAR PANE ---
    let sidebar_toolbar = libadwaita::ToolbarView::new();
    let sidebar_header = libadwaita::HeaderBar::new();
    let sidebar_title = libadwaita::WindowTitle::new("Otush", "");
    sidebar_header.set_title_widget(Some(&sidebar_title));
    sidebar_toolbar.add_top_bar(&sidebar_header);

    let sidebar_scroll = gtk4::ScrolledWindow::new();
    sidebar_scroll.set_hscrollbar_policy(gtk4::PolicyType::Never);
    sidebar_scroll.set_vscrollbar_policy(gtk4::PolicyType::Automatic);

    let sidebar_list = gtk4::ListBox::new();
    sidebar_list.add_css_class("navigation-sidebar");
    sidebar_list.set_selection_mode(gtk4::SelectionMode::Single);
    sidebar_scroll.set_child(Some(&sidebar_list));
    sidebar_toolbar.set_content(Some(&sidebar_scroll));

    let sidebar_page = libadwaita::NavigationPage::new(&sidebar_toolbar, "Otush");
    split.set_sidebar(Some(&sidebar_page));

    // --- CONTENT PANE ---
    let content_toolbar = libadwaita::ToolbarView::new();
    let content_header = libadwaita::HeaderBar::new();
    let window_title = libadwaita::WindowTitle::new("General", "Otush");
    content_header.set_title_widget(Some(&window_title));
    content_toolbar.add_top_bar(&content_header);

    // First-run banner: guide new users to pick a model.
    if !ctx.settings().onboarding_completed {
        let banner = libadwaita::Banner::new(
            "Welcome to Otush — select a speech model below to get started.",
        );
        banner.set_button_label(Some("Get Started"));
        let banner_ctx = ctx.clone();
        banner.connect_button_clicked(move |banner| {
            let mut settings = banner_ctx.settings();
            settings.onboarding_completed = true;
            banner_ctx.write_settings(&settings);
            banner.set_revealed(false);
        });
        content_toolbar.add_top_bar(&banner);
    }

    // Content: a crossfading stack of pages keyed by section id.
    let content_stack = gtk4::Stack::new();
    content_stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
    content_stack.set_hexpand(true);
    content_stack.set_vexpand(true);
    content_toolbar.set_content(Some(&content_stack));

    let content_page = libadwaita::NavigationPage::new(&content_toolbar, "Settings");
    split.set_content(Some(&content_page));

    toast_overlay.set_child(Some(&split));

    // Populate sidebar categories and rows
    let mut current_category: Option<SectionCategory> = None;
    let mut first_selectable_row: Option<gtk4::ListBoxRow> = None;

    for section in SECTIONS {
        if current_category != Some(section.category) {
            current_category = Some(section.category);
            let cat_name = match section.category {
                SectionCategory::Workspace => "Workspace",
                SectionCategory::Preferences => "Preferences",
            };
            let cat_row = create_category_header(cat_name);
            sidebar_list.append(&cat_row);
        }

        let row = create_section_row(section);
        sidebar_list.append(&row);

        if first_selectable_row.is_none() {
            first_selectable_row = Some(row.clone());
        }

        let page = crate::ui::pages::build_page(section.id, ctx);
        content_stack.add_named(&page, Some(section.id));
    }

    // Wire sidebar selection → visible content page & update header title
    let content_stack_weak = content_stack.downgrade();
    let window_title_weak = window_title.downgrade();
    let split_weak = split.downgrade();

    sidebar_list.connect_row_selected(move |_list, row| {
        if let Some(row) = row {
            let widget_name = row.widget_name();
            if widget_name.is_empty() {
                return;
            }
            if let Some(stack) = content_stack_weak.upgrade() {
                stack.set_visible_child_name(&widget_name);
            }
            if let Some(w_title) = window_title_weak.upgrade() {
                if let Some(sec) = SECTIONS.iter().find(|s| s.id == widget_name.as_str()) {
                    w_title.set_title(sec.title);
                }
            }
            if let Some(split) = split_weak.upgrade() {
                if split.is_collapsed() {
                    split.set_show_content(true);
                }
            }
        }
    });

    // Select the first section by default.
    if let Some(first) = first_selectable_row {
        sidebar_list.select_row(Some(&first));
        if let Some(first_sec) = SECTIONS.first() {
            window_title.set_title(first_sec.title);
        }
    }

    // Close → hide to tray; the process (and tray) keeps running. The tray's
    // Quit action terminates the app.
    window.connect_close_request(|window| {
        window.hide();
        glib::Propagation::Stop
    });

    (window, toast_overlay)
}

fn create_category_header(label_text: &str) -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::new();
    row.set_activatable(false);
    row.set_selectable(false);

    let label = gtk4::Label::new(Some(label_text));
    label.add_css_class("caption");
    label.add_css_class("dim-label");
    label.add_css_class("heading");
    label.set_xalign(0.0);
    label.set_margin_start(12);
    label.set_margin_end(12);
    label.set_margin_top(14);
    label.set_margin_bottom(6);

    row.set_child(Some(&label));
    row
}

fn create_section_row(section: &Section) -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::new();
    row.set_activatable(true);
    row.set_widget_name(section.id);

    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    hbox.set_margin_start(12);
    hbox.set_margin_end(12);
    hbox.set_margin_top(8);
    hbox.set_margin_bottom(8);

    let icon = gtk4::Image::from_icon_name(section.icon);
    icon.set_pixel_size(16);
    hbox.append(&icon);

    let label = gtk4::Label::new(Some(section.title));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    hbox.append(&label);

    row.set_child(Some(&hbox));
    row
}
