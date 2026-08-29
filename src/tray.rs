//! System tray icon and menu (StatusNotifierItem via ksni).
//!
//! The tray is driven by a desired-state snapshot ([`TraySnapshot`]) that
//! callers update through [`set_tray_state`], [`refresh_tray_icon`] and
//! [`update_tray_menu`]. Every such call records intent and pushes the change
//! to the running SNI service via `Handle::update` (which re-reads the icon
//! and menu); the first call spawns the service. Menu callbacks dispatch to
//! the app context on worker threads so the menu never blocks.
//!
//! On GNOME, the icon requires the "AppIndicator and KStatusNotifierItem
//! Support" shell extension (shipped by default on Ubuntu's GNOME session).

use crate::context::{AppContext, AppEvent};
use crate::managers::history::HistoryEntry;
use crate::settings;
use crate::tray_i18n::get_tray_translations;
use image::GenericImageView;
use log::{error, info, warn};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayIconState {
    Idle,
    Recording,
    Transcribing,
}

impl TrayIconState {
    /// Recording and Transcribing share the same menu ("Cancel" instead of the
    /// model submenu), so only the idle/busy distinction matters for the menu.
    fn is_busy(self) -> bool {
        self != TrayIconState::Idle
    }
}

/// Tray icon theme. Linux always uses the colored (pink) icon set; the
/// light/dark variants are kept for other platforms' themes.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppTheme {
    Dark,
    Light,
    Colored,
}

/// Gets the current app theme; Linux always uses the colored theme.
pub fn get_current_theme(_ctx: &AppContext) -> AppTheme {
    AppTheme::Colored
}

/// Gets the appropriate icon path (relative to the resource dir) for the
/// given theme and state.
pub fn get_icon_path(theme: AppTheme, state: TrayIconState, warning: bool) -> &'static str {
    if warning && state == TrayIconState::Idle {
        return match theme {
            AppTheme::Dark => "tray_idle_warning.png",
            AppTheme::Light => "tray_idle_warning_dark.png",
            // Linux never sets the warning flag (Secure Input is macOS-only),
            // but fall back to the normal icon just in case.
            AppTheme::Colored => "otush.png",
        };
    }
    match (theme, state) {
        // Dark theme uses light icons
        (AppTheme::Dark, TrayIconState::Idle) => "tray_idle.png",
        (AppTheme::Dark, TrayIconState::Recording) => "tray_recording.png",
        (AppTheme::Dark, TrayIconState::Transcribing) => "tray_transcribing.png",
        // Light theme uses dark icons
        (AppTheme::Light, TrayIconState::Idle) => "tray_idle_dark.png",
        (AppTheme::Light, TrayIconState::Recording) => "tray_recording_dark.png",
        (AppTheme::Light, TrayIconState::Transcribing) => "tray_transcribing_dark.png",
        // Colored theme uses pink icons (for Linux)
        (AppTheme::Colored, TrayIconState::Idle) => "otush.png",
        (AppTheme::Colored, TrayIconState::Recording) => "recording.png",
        (AppTheme::Colored, TrayIconState::Transcribing) => "transcribing.png",
    }
}

pub fn tray_tooltip() -> String {
    if cfg!(debug_assertions) {
        format!("Otush v{} (Dev)", env!("CARGO_PKG_VERSION"))
    } else {
        format!("Otush v{}", env!("CARGO_PKG_VERSION"))
    }
}

/// Desired tray state, shared with the SNI service thread.
#[derive(Debug, Clone)]
struct TraySnapshot {
    icon_state: TrayIconState,
    visible: bool,
}

static TRAY_SNAPSHOT: OnceLock<Arc<Mutex<TraySnapshot>>> = OnceLock::new();
static TRAY_HANDLE: Mutex<Option<ksni::blocking::Handle<OtushTray>>> = Mutex::new(None);

/// The SNI service. ksni serves `icon_pixmap`/`menu` on demand (each DBus
/// request); the shared snapshot is mutated by the app and pushed via
/// `Handle::update`.
pub struct OtushTray {
    ctx: AppContext,
    snapshot: Arc<Mutex<TraySnapshot>>,
}

impl ksni::Tray for OtushTray {
    fn id(&self) -> String {
        "otush".to_string()
    }

    fn title(&self) -> String {
        tray_tooltip()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let snapshot = self.snapshot.lock().unwrap_or_else(|e| e.into_inner());
        if !snapshot.visible {
            return vec![];
        }
        let path = get_icon_path(get_current_theme(&self.ctx), snapshot.icon_state, false);
        match load_pixmap(crate::resources::resource(path)) {
            Some(icon) => vec![icon],
            None => {
                warn!("Failed to load tray icon from {}", path);
                vec![]
            }
        }
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        let snapshot = self.snapshot.lock().unwrap_or_else(|e| e.into_inner());
        build_menu(&self.ctx, snapshot.icon_state)
    }
}

/// Decode a PNG into an ARGB32 `ksni::Icon` (network byte order).
fn load_pixmap(path: std::path::PathBuf) -> Option<ksni::Icon> {
    let img = image::open(&path).ok()?;
    let (width, height) = img.dimensions();
    let mut data = img.into_rgba8().into_vec();
    // ksni expects ARGB32; the image crate yields RGBA.
    for pixel in data.as_chunks_mut::<4>().0 {
        pixel.rotate_right(1);
    }
    Some(ksni::Icon {
        width: width as i32,
        height: height as i32,
        data,
    })
}

fn snapshot_handle() -> Arc<Mutex<TraySnapshot>> {
    Arc::clone(TRAY_SNAPSHOT.get_or_init(|| {
        Arc::new(Mutex::new(TraySnapshot {
            icon_state: TrayIconState::Idle,
            visible: true,
        }))
    }))
}

/// Spawn the SNI service if it is not running, or push the current snapshot
/// to it if it is. Every public setter funnels through here.
///
/// The actual ksni work always runs on the GTK main thread: the blocking
/// StatusNotifierItem API builds its own runtime and panics with "cannot
/// start a runtime from within a runtime" when invoked from a Tokio worker
/// (e.g. `set_tray_state` called inside an async transcription task).
pub fn sync_tray(ctx: &AppContext) {
    let ctx = ctx.clone();
    glib::MainContext::default().invoke(move || {
        let mut guard = TRAY_HANDLE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(handle) = guard.as_ref() {
            let _ = handle.update(|_| {});
            return;
        }
        let tray = OtushTray {
            ctx: ctx.clone(),
            snapshot: snapshot_handle(),
        };
        use ksni::blocking::TrayMethods;
        match tray.spawn() {
            Ok(handle) => {
                info!("Tray icon created (StatusNotifierItem)");
                *guard = Some(handle);
            }
            Err(e) => {
                warn!("Failed to create tray icon: {}", e);
            }
        }
        let _ = ctx;
    });
}

/// Record a new icon state and refresh the tray.
pub fn set_tray_state(ctx: &AppContext, state: TrayIconState) {
    if let Some(snapshot) = TRAY_SNAPSHOT.get() {
        snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .icon_state = state;
    }
    sync_tray(ctx);
}

/// Refresh the tray icon (e.g. after a theme change).
#[allow(dead_code)]
pub fn refresh_tray_icon(ctx: &AppContext) {
    sync_tray(ctx);
}

/// Rebuild the tray menu (e.g. after model/settings changes).
pub fn update_tray_menu(ctx: &AppContext) {
    sync_tray(ctx);
}

/// Show or hide the tray icon.
pub fn set_tray_visibility(ctx: &AppContext, visible: bool) {
    if let Some(snapshot) = TRAY_SNAPSHOT.get() {
        snapshot.lock().unwrap_or_else(|e| e.into_inner()).visible = visible;
    }
    sync_tray(ctx);
}

fn last_transcript_text(entry: &HistoryEntry) -> &str {
    entry
        .post_processed_text
        .as_deref()
        .unwrap_or(&entry.transcription_text)
}

/// Copy the most recent completed transcription to the clipboard.
pub fn copy_last_transcript(ctx: &AppContext) {
    let entry = match ctx.history.get_latest_completed_entry() {
        Ok(Some(entry)) => entry,
        Ok(None) => {
            warn!("No completed transcription history entries available for tray copy.");
            return;
        }
        Err(err) => {
            error!(
                "Failed to fetch last completed transcription entry: {}",
                err
            );
            return;
        }
    };

    let text = last_transcript_text(&entry);
    if text.trim().is_empty() {
        warn!("Last completed transcription is empty; skipping tray copy.");
        return;
    }

    if let Err(err) = crate::clipboard::write_clipboard_text(ctx, text) {
        error!("Failed to copy last transcript to clipboard: {}", err);
        return;
    }

    info!("Copied last transcript to clipboard via tray.");
}

/// Build the tray menu for the current state. Called by ksni on demand.
fn build_menu(ctx: &AppContext, icon_state: TrayIconState) -> Vec<ksni::MenuItem<OtushTray>> {
    use ksni::menu::{MenuItem, StandardItem, SubMenu};

    let settings = settings::get_settings(ctx);
    let strings = get_tray_translations(Some(settings.app_language.clone()));

    let mut items: Vec<MenuItem<OtushTray>> = Vec::new();

    // Settings
    items.push(
        StandardItem {
            label: strings.settings,
            activate: Box::new({
                let ctx = ctx.clone();
                move |_| {
                    let _ = crate::commands::show_main_window_command(&ctx);
                }
            }),
            ..Default::default()
        }
        .into(),
    );

    // Check for updates
    items.push(
        StandardItem {
            label: strings.check_updates,
            activate: Box::new({
                let ctx = ctx.clone();
                move |_| {
                    ctx.bus.send(AppEvent::CheckForUpdates);
                }
            }),
            ..Default::default()
        }
        .into(),
    );

    // Copy last transcript
    items.push(
        StandardItem {
            label: strings.copy_last_transcript,
            activate: Box::new({
                let ctx = ctx.clone();
                move |_| copy_last_transcript(&ctx)
            }),
            ..Default::default()
        }
        .into(),
    );

    items.push(MenuItem::Separator);

    if icon_state.is_busy() {
        // Cancel (busy states)
        items.push(
            StandardItem {
                label: strings.cancel,
                activate: Box::new({
                    let ctx = ctx.clone();
                    move |_| {
                        let ctx = ctx.clone();
                        std::thread::spawn(move || crate::utils::cancel_current_operation(&ctx));
                    }
                }),
                ..Default::default()
            }
            .into(),
        );
    } else {
        // Model submenu
        let models: Vec<(String, String)> = ctx
            .model
            .get_available_models()
            .into_iter()
            .filter(|m| m.is_downloaded)
            .map(|m| (m.id, m.name))
            .collect();
        let selected = settings.selected_model;
        if !models.is_empty() {
            let submenu: Vec<MenuItem<OtushTray>> = models
                .into_iter()
                .map(|(id, name)| {
                    let ctx = ctx.clone();
                    let selected = selected.clone();
                    StandardItem {
                        label: name,
                        activate: Box::new(move |_| {
                            let ctx = ctx.clone();
                            let id = id.clone();
                            let selected = selected.clone();
                            std::thread::spawn(move || {
                                if id != selected {
                                    if let Err(e) =
                                        crate::commands::models::switch_active_model(&ctx, &id)
                                    {
                                        error!("Failed to switch model via tray: {}", e);
                                    }
                                    update_tray_menu(&ctx);
                                }
                            });
                        }),
                        ..Default::default()
                    }
                    .into()
                })
                .collect();
            items.push(
                SubMenu {
                    label: strings.model,
                    submenu,
                    ..Default::default()
                }
                .into(),
            );
        }

        // Unload model
        items.push(
            StandardItem {
                label: strings.unload_model,
                enabled: ctx.transcription.is_model_loaded(),
                activate: Box::new({
                    let ctx = ctx.clone();
                    move |_| {
                        let ctx = ctx.clone();
                        std::thread::spawn(move || match ctx.transcription.unload_model() {
                            Ok(()) => info!("Model unloaded via tray."),
                            Err(e) => error!("Failed to unload model via tray: {}", e),
                        });
                    }
                }),
                ..Default::default()
            }
            .into(),
        );
    }

    items.push(MenuItem::Separator);

    // Quit
    items.push(
        StandardItem {
            label: strings.quit,
            activate: Box::new(|_| std::process::exit(0)),
            ..Default::default()
        }
        .into(),
    );

    items
}

#[cfg(test)]
mod tests {
    use super::{get_icon_path, AppTheme, TrayIconState};

    #[test]
    fn colored_theme_uses_otush_icon_when_idle() {
        assert_eq!(
            get_icon_path(AppTheme::Colored, TrayIconState::Idle, false),
            "otush.png"
        );
        assert_eq!(
            get_icon_path(AppTheme::Colored, TrayIconState::Recording, false),
            "recording.png"
        );
        assert_eq!(
            get_icon_path(AppTheme::Colored, TrayIconState::Transcribing, false),
            "transcribing.png"
        );
    }
}
