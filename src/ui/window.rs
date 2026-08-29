//! Main window: navigation shell (sidebar + content stack) hosting the
//! settings pages, with a toast overlay and hide-to-tray behavior.

use crate::context::AppContext;
use libadwaita::prelude::*;

/// Sidebar section metadata.
struct Section {
    id: &'static str,
    title: &'static str,
}

const SECTIONS: &[Section] = &[
    Section {
        id: "general",
        title: "General",
    },
    Section {
        id: "models",
        title: "Models",
    },
    Section {
        id: "post_processing",
        title: "Post-Processing",
    },
    Section {
        id: "history",
        title: "History",
    },
    Section {
        id: "advanced",
        title: "Advanced",
    },
    Section {
        id: "debug",
        title: "Debug",
    },
    Section {
        id: "about",
        title: "About",
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
    window.set_default_size(880, 620);

    // Toast overlay wraps everything.
    let toast_overlay = libadwaita::ToastOverlay::new();
    window.set_content(Some(&toast_overlay));

    // Toolbar view: header bar on top, content below.
    let toolbar = libadwaita::ToolbarView::new();
    let header = libadwaita::HeaderBar::new();
    toolbar.add_top_bar(&header);
    toast_overlay.set_child(Some(&toolbar));

    // First-run banner: guide new users to pick a model.
    if !ctx.settings().onboarding_completed {
        let banner =
            libadwaita::Banner::new("Welcome to Otush — pick a model below to get started.");
        banner.set_button_label(Some("Get Started"));
        let banner_ctx = ctx.clone();
        banner.connect_button_clicked(move |banner| {
            let mut settings = banner_ctx.settings();
            settings.onboarding_completed = true;
            banner_ctx.write_settings(&settings);
            banner.set_revealed(false);
        });
        toolbar.add_top_bar(&banner);
    }

    // Sidebar: one row per section.
    let sidebar_list = gtk4::ListBox::new();
    sidebar_list.add_css_class("navigation-sidebar");
    sidebar_list.set_selection_mode(gtk4::SelectionMode::Single);

    // Content: a stack of pages keyed by section id.
    let content_stack = gtk4::Stack::new();
    content_stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
    content_stack.set_hexpand(true);
    content_stack.set_vexpand(true);

    for section in SECTIONS {
        let row = libadwaita::ActionRow::new();
        row.set_title(section.title);
        row.set_activatable(true);
        row.set_widget_name(section.id);
        sidebar_list.append(&row);

        let page = crate::ui::pages::build_page(section.id, ctx);
        content_stack.add_named(&page, Some(section.id));
    }

    // Wire sidebar selection → visible content page.
    let content_stack_weak = content_stack.downgrade();
    sidebar_list.connect_row_selected(move |_list, row| {
        if let Some(row) = row {
            if let Some(stack) = content_stack_weak.upgrade() {
                stack.set_visible_child_name(&row.widget_name());
            }
        }
    });

    // Select the first section by default.
    if let Some(first) = sidebar_list.first_child() {
        if let Ok(row) = first.downcast::<gtk4::ListBoxRow>() {
            sidebar_list.select_row(Some(&row));
        }
    }

    // Navigation split view (sidebar + content), the GNOME navigation idiom.
    let split = libadwaita::NavigationSplitView::new();
    let sidebar_page = libadwaita::NavigationPage::new(&sidebar_list, "Otush");
    let content_page = libadwaita::NavigationPage::new(&content_stack, "Settings");
    split.set_sidebar(Some(&sidebar_page));
    split.set_content(Some(&content_page));
    toolbar.set_content(Some(&split));

    // Close → hide to tray; the process (and tray) keeps running. The tray's
    // Quit action terminates the app.
    window.connect_close_request(|window| {
        window.hide();
        glib::Propagation::Stop
    });

    (window, toast_overlay)
}
