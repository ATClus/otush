//! GTK4/libadwaita application shell — the native GNOME UI.

use crate::context::{AppContext, AppEvent};
use crate::CliArgs;
use libadwaita::prelude::*;
use log::info;
use std::sync::OnceLock;

/// The running application context, bound at startup (used by tray, CLI and
/// signal paths that live outside the widget tree).
static APP_CTX: OnceLock<AppContext> = OnceLock::new();

/// Weak handle to the main window (GTK objects are not `Send`, so the static
/// stores a sendable weak ref; access is marshaled onto the main thread).
static MAIN_WINDOW: OnceLock<glib::SendWeakRef<libadwaita::ApplicationWindow>> = OnceLock::new();

/// Raise the main settings window (from tray / CLI / second instance).
pub fn show_main_window() {
    let Some(send_weak) = MAIN_WINDOW.get() else {
        info!("show_main_window: window not created yet");
        return;
    };
    let send_weak = send_weak.clone();
    glib::MainContext::default().invoke(move || {
        let weak = send_weak.into_weak_ref();
        if let Some(window) = weak.upgrade() {
            window.present();
        }
    });
}

/// Apply the persisted appearance setting to libadwaita's color scheme.
fn apply_theme(theme: crate::settings::Theme) {
    let manager = libadwaita::StyleManager::default();
    match theme {
        crate::settings::Theme::System => {
            manager.set_color_scheme(libadwaita::ColorScheme::Default)
        }
        crate::settings::Theme::Light => {
            manager.set_color_scheme(libadwaita::ColorScheme::ForceLight)
        }
        crate::settings::Theme::Dark => {
            manager.set_color_scheme(libadwaita::ColorScheme::ForceDark)
        }
    }
}

/// Wire backend events to shell-level UI effects (toasts, theme).
fn subscribe_bus(ctx: &AppContext, toasts: &libadwaita::ToastOverlay) {
    let ctx_owned = ctx.clone();
    let toasts = glib::SendWeakRef::from(toasts.downgrade());
    ctx.bus.subscribe(move |event| {
        let ctx = ctx_owned.clone();
        let toasts = toasts.clone();
        glib::MainContext::default().invoke(move || {
            let weak = toasts.into_weak_ref();
            let Some(toasts) = weak.upgrade() else {
                return;
            };
            match event {
                AppEvent::CheckForUpdates => {
                    toasts.add_toast(libadwaita::Toast::new("Checking for updates…"));
                    crate::updater::check_for_updates(&toasts);
                }
                AppEvent::PasteError => {
                    toasts.add_toast(libadwaita::Toast::new("Failed to paste the transcription"));
                }
                AppEvent::TranscriptionError(err) => {
                    toasts.add_toast(libadwaita::Toast::new(&format!(
                        "Transcription failed: {err}"
                    )));
                }
                AppEvent::RecordingError(e) => {
                    let message = match e.error_type.as_str() {
                        "microphone_permission_denied" => {
                            "Microphone permission was denied".to_string()
                        }
                        "no_input_device" => "No input device found".to_string(),
                        _ => format!("Recording failed: {}", e.detail.unwrap_or_default()),
                    };
                    toasts.add_toast(libadwaita::Toast::new(&message));
                }
                AppEvent::TextCopiedToClipboard { message } => {
                    let toast = libadwaita::Toast::new(&message);
                    toast.set_button_label(Some("Open History"));
                    let toast_ctx = ctx.clone();
                    toast.connect_button_clicked(move |_| {
                        crate::ui::history_palette::show_history_palette(&toast_ctx);
                    });
                    toasts.add_toast(toast);
                }
                AppEvent::ThemeChanged(theme) => apply_theme(theme),
                AppEvent::CommandFailed { context, message } => {
                    toasts.add_toast(libadwaita::Toast::new(&format!("{context}: {message}")));
                }
                _ => {}
            }
        });
    });
}

/// Run the GNOME shell. Called from `lib.rs::run` for non-headless launches.
pub fn run_gtk(ctx: AppContext, cli_args: &CliArgs) {
    let app = libadwaita::Application::new(
        Some("com.clusterat.otush"),
        gio::ApplicationFlags::HANDLES_COMMAND_LINE,
    );

    let _ = APP_CTX.set(ctx);

    let no_tray = cli_args.no_tray;
    let start_hidden_cli = cli_args.start_hidden;
    app.connect_activate(move |app| {
        let Some(ctx) = APP_CTX.get() else {
            return;
        };
        let is_first_init = MAIN_WINDOW.get().is_none();
        if is_first_init {
            let (window, toasts) = crate::ui::window::build_main_window(app, ctx);
            subscribe_bus(ctx, &toasts);
            let _ = MAIN_WINDOW.set(glib::SendWeakRef::from(window.downgrade()));

            // Recording overlay (Wayland layer surface).
            crate::overlay::init_overlay(ctx, ctx.settings().overlay_position);

            // Post-startup initialization (mirrors the pre-port behavior after
            // onboarding): input system + global shortcuts.
            let _ = crate::commands::initialize_enigo();
            let _ = crate::commands::initialize_shortcuts(ctx);

            // Tray (SNI). Hidden if --no-tray or the setting disables it.
            if !no_tray {
                crate::tray::sync_tray(ctx);
                if !ctx.settings().show_tray_icon {
                    crate::tray::set_tray_visibility(ctx, false);
                }
            }
        }
        // Respect start_hidden on the first show; later activates always show.
        let start_hidden = start_hidden_cli || ctx.settings().start_hidden;
        if !start_hidden || !is_first_init {
            show_main_window();
        }
    });

    // A second instance's command line is forwarded here (single-instance
    // behavior is native to GApplication). Remote-control flags act on the
    // running app; a plain relaunch raises the window.
    app.connect_command_line(|app, cmdline| {
        let args: Vec<String> = cmdline
            .arguments()
            .into_iter()
            .skip(1)
            .map(|a| a.into_string().unwrap_or_default())
            .collect();
        let mut handled = false;

        if let Some(ctx) = APP_CTX.get() {
            let is_remote_control = args.iter().any(|a| {
                a == "--toggle-transcription"
                    || a == "--toggle-post-process"
                    || a == "--toggle-meeting"
                    || a == "--transform-selection"
                    || a == "--toggle-history"
                    || a == "--cancel"
            });

            // Ensure app subsystems (window shell, overlay, enigo, tray) are
            // initialized on first launch before processing remote-control commands.
            if is_remote_control && MAIN_WINDOW.get().is_none() {
                app.activate();
            }

            if args.iter().any(|a| a == "--toggle-transcription") {
                crate::signal_handle::send_transcription_input(ctx, "transcribe", "CLI");
                handled = true;
            }
            if args.iter().any(|a| a == "--toggle-post-process") {
                crate::signal_handle::send_transcription_input(
                    ctx,
                    "transcribe_with_post_process",
                    "CLI",
                );
                handled = true;
            }
            if args.iter().any(|a| a == "--toggle-meeting") {
                crate::signal_handle::send_transcription_input(ctx, "transcribe_meeting", "CLI");
                handled = true;
            }
            if args.iter().any(|a| a == "--transform-selection") {
                crate::ui::prompt_palette::show_prompt_palette(ctx);
                handled = true;
            }
            if args.iter().any(|a| a == "--toggle-history") {
                crate::ui::history_palette::toggle_history_palette(ctx);
                handled = true;
            }
            if args.iter().any(|a| a == "--cancel") {
                crate::utils::cancel_current_operation(ctx);
                handled = true;
            }
            if args.iter().any(|a| a == "--start-hidden") && MAIN_WINDOW.get().is_some() {
                handled = true;
            }
        }

        if !handled {
            app.activate();
        }
        glib::ExitCode::from(0)
    });

    // Pass the real argv through so a first launch with a remote-control flag
    // reaches the command-line handler (GApplication forwards these to the
    // primary instance).
    let argv: Vec<String> = std::env::args().collect();
    app.run_with_args(&argv);
}
