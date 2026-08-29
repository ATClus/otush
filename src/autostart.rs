//! Launch-at-login (autostart) handling.
//!
//! Linux/GNOME: writes an XDG autostart desktop entry at
//! `~/.config/autostart/otush.desktop`. Errors are logged rather than
//! returned: the preference is re-applied on every launch, so a transient
//! failure self-heals and must not block startup.

use crate::context::AppContext;

const AUTOSTART_DIR: &str = "autostart";
const DESKTOP_FILE: &str = "otush.desktop";

fn autostart_file_path() -> Option<std::path::PathBuf> {
    let config_dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(dirs::config_dir)?;
    Some(config_dir.join(AUTOSTART_DIR).join(DESKTOP_FILE))
}

/// Apply the user's autostart preference by writing (or removing) the XDG
/// autostart desktop entry.
pub fn apply_autostart(_ctx: &AppContext, enabled: bool) {
    let Some(path) = autostart_file_path() else {
        log::warn!("Could not resolve the autostart directory");
        return;
    };

    let result = if enabled {
        // The entry launches the current executable; the app's own
        // `start_hidden` setting decides window visibility at login.
        let exe = std::env::current_exe().unwrap_or_default();
        let content = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Otush\n\
             Comment=Offline speech-to-text\n\
             Exec={}\n\
             X-GNOME-Autostart-enabled=true\n\
             Hidden=false\n",
            exe.display()
        );
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&path, content)
    } else {
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    };

    if let Err(e) = result {
        log::warn!(
            "Failed to apply autostart setting (enabled={}): {}",
            enabled,
            e
        );
    }
}
