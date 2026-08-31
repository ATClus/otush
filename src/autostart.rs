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
        // The entry launches the current executable in the background (hidden to tray).
        let exe = std::env::current_exe().unwrap_or_default();
        let content = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Otush\n\
             Comment=Offline speech-to-text\n\
             Exec={} --start-hidden\n\
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

/// Ensure `com.clusterat.otush.desktop` exists in `~/.local/share/applications/`.
/// XDG Desktop Portal requires a registered desktop file matching the application ID
/// (`com.clusterat.otush`) to grant GlobalShortcuts on GNOME/Wayland.
pub fn ensure_desktop_entry_registered() {
    let Some(data_dir) = dirs::data_dir() else {
        return;
    };
    let apps_dir = data_dir.join("applications");
    let desktop_file = apps_dir.join("com.clusterat.otush.desktop");

    let exe = std::env::current_exe().unwrap_or_default();
    let is_system_install = exe.starts_with("/usr/bin")
        || exe.starts_with("/usr/local/bin")
        || exe.starts_with("/app/bin");

    let system_desktop =
        std::path::Path::new("/usr/share/applications/com.clusterat.otush.desktop");
    if is_system_install && system_desktop.exists() {
        // If a user desktop file exists in ~/.local/share/applications/, it shadows the
        // system desktop file. Remove it so the system-managed desktop file is used.
        if desktop_file.exists() {
            let _ = std::fs::remove_file(&desktop_file);
            let _ = std::process::Command::new("update-desktop-database")
                .arg(&apps_dir)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
        return;
    }

    let exe_dir = exe.parent().unwrap_or_else(|| std::path::Path::new(""));
    let content = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Otush\n\
         GenericName=Speech to Text\n\
         Comment=A free, open source, offline speech-to-text application\n\
         Exec={}\n\
         Path={}\n\
         Icon=com.clusterat.otush\n\
         Terminal=false\n\
         Categories=Utility;AudioVideo;Accessibility;\n\
         Keywords=speech;text;transcription;dictation;whisper;voice;\n\
         StartupNotify=true\n\
         StartupWMClass=com.clusterat.otush\n\
         X-GNOME-UsesNotifications=true\n",
        exe.display(),
        exe_dir.display()
    );

    let _ = std::fs::create_dir_all(&apps_dir);
    let _ = std::fs::write(&desktop_file, content);

    // Also install the application icon into ~/.local/share/icons if not present
    let icons_dir = data_dir.join("icons/hicolor/128x128/apps");
    let icon_file = icons_dir.join("com.clusterat.otush.png");
    if !icon_file.exists() {
        let _ = std::fs::create_dir_all(&icons_dir);
        let bundled_icon = crate::resources::resource_dir().join("otush-128.png");
        if bundled_icon.exists() {
            let _ = std::fs::copy(&bundled_icon, &icon_file);
        }
    }

    // Refresh user desktop database so GNOME immediately recognizes the application ID
    let _ = std::process::Command::new("update-desktop-database")
        .arg(&apps_dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}
