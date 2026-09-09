//! Launch-at-login (autostart) handling.
//!
//! Linux/GNOME: writes an XDG autostart desktop entry at
//! `~/.config/autostart/otush.desktop`. Errors are logged rather than
//! returned: the preference is re-applied on every launch, so a transient
//! failure self-heals and must not block startup.

use crate::context::AppContext;

const AUTOSTART_DIR: &str = "autostart";
/// Canonical autostart entry: matches the application ID so GNOME Tweaks,
/// `gnome-session-properties`, and the `.deb`-installed desktop entry all
/// refer to the same file.
const DESKTOP_FILE: &str = "com.clusterat.otush.desktop";
/// Pre-rebrand name (pre-1.x). Migrated automatically; never written again.
const LEGACY_DESKTOP_FILE: &str = "otush.desktop";

fn autostart_file_path() -> Option<std::path::PathBuf> {
    let config_dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(dirs::config_dir)?;
    Some(config_dir.join(AUTOSTART_DIR).join(DESKTOP_FILE))
}

fn legacy_autostart_file_path() -> Option<std::path::PathBuf> {
    let config_dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(dirs::config_dir)?;
    Some(config_dir.join(AUTOSTART_DIR).join(LEGACY_DESKTOP_FILE))
}

/// Read the effective autostart state from the file, honoring the XDG keys
/// the desktop reads: missing file or `Hidden=true` means off, anything
/// else means on. `X-GNOME-Autostart-enabled` is GNOME-specific; absence
/// does not mean disabled.
fn read_autostart_file_state(path: &std::path::Path) -> Option<bool> {
    let content = std::fs::read_to_string(path).ok()?;
    for line in content.lines() {
        let line = line.trim();
        if line.eq_ignore_ascii_case("Hidden=true") {
            return Some(false);
        }
    }
    Some(true)
}

/// Reconcile the autostart entry with the in-app setting at startup.
///
/// Called on every launch (before the GTK shell). Handles all four cases:
/// - setting on + file present: refresh the entry (repairs `Exec` after a
///   `.deb` upgrade moves the binary).
/// - setting on + file missing: recreate it. This is the reported bug: the
///   user enables autostart in GNOME Tweaks/System Settings, which writes
///   `Hidden=false` (or its own copy of the entry); a later `.deb` upgrade
///   refreshes `/usr/share/applications` and GNOME resets the override —
///   the in-app setting was already off, so nothing recreated the file.
///   Now the in-app setting is the source of truth and always wins.
/// - setting off + file present: remove it (external tool re-enabled it).
/// - setting off + file missing: nothing to do.
///
/// Also migrates the pre-rebrand `otush.desktop` to the canonical
/// `com.clusterat.otush.desktop` (same name as the application ID and the
/// `.deb`-installed entry), so Tweaks and the app manage one file.
pub fn ensure_autostart_consistency(ctx: &AppContext) {
    // One-way migration first: legacy file wins over absence, then goes away.
    if let (Some(legacy), Some(current)) = (legacy_autostart_file_path(), autostart_file_path()) {
        if legacy.exists() && !current.exists() {
            if let Some(parent) = current.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::rename(&legacy, &current).is_err() {
                let _ = std::fs::copy(&legacy, &current);
                let _ = std::fs::remove_file(&legacy);
            }
            log::info!("Migrated autostart entry to {}", current.display());
        } else if legacy.exists() {
            let _ = std::fs::remove_file(&legacy);
        }
    }

    let wanted = ctx.settings().autostart_enabled;
    let path = match autostart_file_path() {
        Some(path) => path,
        None => {
            log::warn!("Could not resolve the autostart directory");
            return;
        }
    };
    let present = path.exists() || std::fs::symlink_metadata(&path).is_ok();
    // `symlink_metadata` catches broken symlinks (`exists` follows links):
    // GNOME Tweaks (and older app versions) may leave the entry as a link
    // into `~/.local/share/applications/`, whose target
    // `ensure_desktop_entry_registered` legitimately deletes on system
    // installs. A broken link reads as missing/disabled, so it is recreated
    // as a regular file below instead of being written through.
    let file_enabled = present && read_autostart_file_state(&path).unwrap_or(false);
    match reconcile_decision(wanted, present, file_enabled) {
        ReconcileAction::Refresh => apply_autostart(ctx, true),
        ReconcileAction::Recreate => {
            log::info!("Recreating missing autostart entry (setting is on)");
            apply_autostart(ctx, true);
        }
        ReconcileAction::Remove => {
            log::info!("Removing external autostart entry (setting is off)");
            apply_autostart(ctx, false);
        }
        ReconcileAction::Nothing => {}
    }
}

/// Startup reconciliation outcome. Pure decision table over (setting,
/// file present, file enabled) — unit-tested below without touching disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReconcileAction {
    /// Setting on + healthy file: rewrite so `Exec` tracks the binary.
    Refresh,
    /// Setting on + missing/disabled file: recreate.
    Recreate,
    /// Setting off + enabled file: remove.
    Remove,
    /// Setting off + no file: nothing to do.
    Nothing,
}

fn reconcile_decision(wanted: bool, present: bool, file_enabled: bool) -> ReconcileAction {
    match (wanted, present, file_enabled) {
        (true, true, true) => ReconcileAction::Refresh,
        (true, _, _) => ReconcileAction::Recreate,
        (false, true, true) => ReconcileAction::Remove,
        (false, _, _) => ReconcileAction::Nothing,
    }
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
             Comment=Native AI Voice & Productivity Suite for GNOME\n\
             Exec={} --start-hidden\n\
             X-GNOME-Autostart-enabled=true\n\
             Hidden=false\n",
            exe.display()
        );
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Never write through a symlink: `std::fs::write` follows links, so
        // a stale link into `~/.local/share/applications/` (whose target
        // `ensure_desktop_entry_registered` deletes on system installs)
        // would resurrect the target as an autostart entry and re-break on
        // the next launch. Replace any link with a regular file instead.
        if std::fs::symlink_metadata(&path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            log::info!(
                "Replacing symlinked autostart entry at {} with a regular file",
                path.display()
            );
            let _ = std::fs::remove_file(&path);
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
         GenericName=AI Voice & Productivity Suite\n\
         Comment=Native speech-to-text, meeting intelligence, notes, tasks, OCR, and AI research for GNOME\n\
         Exec={}\n\
         Path={}\n\
         Icon=com.clusterat.otush\n\
         Terminal=false\n\
         Categories=Utility;AudioVideo;Accessibility;Office;\n\
         Keywords=speech;text;transcription;dictation;whisper;voice;ai;llm;notes;todos;ocr;research;gnome;wayland;productivity;\n\
         StartupNotify=true\n\
         StartupWMClass=com.clusterat.otush\n\
         X-GNOME-UsesNotifications=true\n",
        exe.display(),
        exe_dir.display()
    );

    let _ = std::fs::create_dir_all(&apps_dir);
    let _ = std::fs::write(&desktop_file, content);

    // Install the application icons into ~/.local/share/icons/hicolor
    let resource_dir = crate::resources::resource_dir();

    // 1. Scalable vector SVG
    let svg_dir = data_dir.join("icons/hicolor/scalable/apps");
    if std::fs::create_dir_all(&svg_dir).is_ok() {
        let bundled_svg = resource_dir.join("otush-icon.svg");
        if bundled_svg.exists() {
            let _ = std::fs::copy(&bundled_svg, svg_dir.join("com.clusterat.otush.svg"));
        }
    }

    // 2. High-res 512x512 PNG
    let p512_dir = data_dir.join("icons/hicolor/512x512/apps");
    if std::fs::create_dir_all(&p512_dir).is_ok() {
        let bundled_512 = resource_dir.join("otush.png");
        if bundled_512.exists() {
            let _ = std::fs::copy(&bundled_512, p512_dir.join("com.clusterat.otush.png"));
        }
    }

    // 3. Standard 128x128 PNG
    let p128_dir = data_dir.join("icons/hicolor/128x128/apps");
    if std::fs::create_dir_all(&p128_dir).is_ok() {
        let bundled_128 = resource_dir.join("otush-128.png");
        if bundled_128.exists() {
            let _ = std::fs::copy(&bundled_128, p128_dir.join("com.clusterat.otush.png"));
        }
    }

    // Refresh icon cache
    let hicolor_dir = data_dir.join("icons/hicolor");
    let _ = std::process::Command::new("gtk-update-icon-cache")
        .arg("-f")
        .arg("-t")
        .arg(&hicolor_dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    // Refresh user desktop database so GNOME immediately recognizes the application ID
    let _ = std::process::Command::new("update-desktop-database")
        .arg(&apps_dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn decision_table_covers_all_states() {
        // Setting on: healthy file refreshes, anything else recreates.
        assert_eq!(
            reconcile_decision(true, true, true),
            ReconcileAction::Refresh
        );
        assert_eq!(
            reconcile_decision(true, false, false),
            ReconcileAction::Recreate
        );
        assert_eq!(
            reconcile_decision(true, true, false),
            ReconcileAction::Recreate
        );
        // Setting off: enabled file is removed, otherwise nothing.
        assert_eq!(
            reconcile_decision(false, true, true),
            ReconcileAction::Remove
        );
        assert_eq!(
            reconcile_decision(false, false, false),
            ReconcileAction::Nothing
        );
        assert_eq!(
            reconcile_decision(false, true, false),
            ReconcileAction::Nothing
        );
    }

    #[test]
    fn hidden_true_reads_as_disabled() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("entry.desktop");
        let mut file = std::fs::File::create(&path).expect("create");
        writeln!(file, "[Desktop Entry]\nHidden=true").expect("write");
        assert_eq!(read_autostart_file_state(&path), Some(false));
    }

    #[test]
    fn plain_entry_reads_as_enabled() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("entry.desktop");
        let mut file = std::fs::File::create(&path).expect("create");
        writeln!(file, "[Desktop Entry]\nHidden=false").expect("write");
        assert_eq!(read_autostart_file_state(&path), Some(true));
    }

    #[test]
    fn missing_file_reads_as_none() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert_eq!(
            read_autostart_file_state(&dir.path().join("nope.desktop")),
            None
        );
    }

    #[test]
    fn broken_symlink_reads_as_not_present() {
        // `Path::exists` follows links, so a broken symlink must be probed
        // via `symlink_metadata` — otherwise the reconciler mistakes it for
        // a missing file and writes *through* it, resurrecting the target.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("entry.desktop");
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.path().join("gone.desktop"), &path).expect("symlink");
        assert!(!path.exists());
        assert!(std::fs::symlink_metadata(&path).is_ok());
        assert_eq!(read_autostart_file_state(&path), None);
    }
}
