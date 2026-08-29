//! About page: version, license and links, plus a What's New summary.

use crate::context::AppContext;
use libadwaita::prelude::*;

/// Build the About preferences page.
pub fn build(ctx: &AppContext) -> gtk4::Widget {
    let page = libadwaita::PreferencesPage::new();
    page.set_title("About");

    let group = libadwaita::PreferencesGroup::new();
    group.set_title("Otush");

    let version = crate::updater::current_version();
    let version_row = libadwaita::ActionRow::new();
    version_row.set_title("Version");
    version_row.set_subtitle(&version);
    version_row.set_activatable(false);
    group.add(&version_row);

    let name_row = libadwaita::ActionRow::new();
    name_row.set_title("A free, open source speech-to-text application");
    name_row.set_subtitle("Offline Whisper transcription with AI post-processing");
    name_row.set_activatable(false);
    group.add(&name_row);

    let about_button = gtk4::Button::with_label("About Otush");
    let ctx = ctx.clone();
    about_button.connect_clicked(move |_| show_about(&ctx));
    let about_row = libadwaita::ActionRow::new();
    about_row.set_title("About");
    about_row.add_suffix(&about_button);
    group.add(&about_row);

    let whatsnew_button = gtk4::Button::with_label("View");
    whatsnew_button.connect_clicked(|_| {
        let dialog = libadwaita::MessageDialog::new(
            None::<&gtk4::Window>,
            Some("What's New"),
            Some(
                "This native GNOME build of Otush is a work in progress — \
                 release notes will be shown here.",
            ),
        );
        dialog.add_response("close", "Close");
        dialog.set_default_response(Some("close"));
        dialog.connect_response(None, |dialog: &libadwaita::MessageDialog, _: &str| {
            dialog.close()
        });
        dialog.present();
    });
    let whatsnew_row = libadwaita::ActionRow::new();
    whatsnew_row.set_title("What's New");
    whatsnew_row.add_suffix(&whatsnew_button);
    group.add(&whatsnew_row);

    page.add(&group);

    // --- Links ---
    let links_group = libadwaita::PreferencesGroup::new();
    links_group.set_title("Links");

    let github_row = libadwaita::ActionRow::new();
    github_row.set_title("GitHub");
    github_row.set_subtitle("github.com/ATClus/otush");
    github_row.set_activatable(true);
    github_row.connect_activated(|_| {
        let _ = opener::open("https://github.com/ATClus/otush");
    });
    links_group.add(&github_row);

    page.add(&links_group);

    page.upcast::<gtk4::Widget>()
}

fn show_about(_ctx: &AppContext) {
    let version = crate::updater::current_version();
    let about = libadwaita::AboutWindow::new();
    about.set_application_name("Otush");
    about.set_version(&version);
    about.set_developer_name("Otush contributors");
    about.set_copyright("© Otush contributors");
    about.set_license_type(gtk4::License::MitX11);
    about.set_website("https://github.com/ATClus/otush");
    about.set_comments("A free, open source, offline speech-to-text application for GNOME.");
    about.set_translator_credits("translator-credits");
    about.present();
}
