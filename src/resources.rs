//! Resolution of bundled resource files (sounds, tray icons, VAD model,
//! default settings).

use std::path::PathBuf;

/// Directory containing bundled resources.
///
/// - Development: `resources/` next to the crate manifest (embedded at compile
///   time via `CARGO_MANIFEST_DIR`).
/// - Installed (deb): `/usr/share/otush/resources` (or a `resources` dir next
///   to the executable, which also covers portable-style installs).
pub fn resource_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        let exe_dir = exe.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        for candidate in [
            exe_dir.join("resources"),
            exe_dir.join("../share/otush/resources"),
            exe_dir.join("../../share/otush/resources"),
        ] {
            if candidate.is_dir() {
                return candidate;
            }
        }
    }
    // Dev layout. Falls back to a bare `resources` relative to CWD.
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources");
    if dev.is_dir() {
        dev
    } else {
        PathBuf::from("resources")
    }
}

/// Convenience: resolve a resource file path relative to the resource dir.
pub fn resource(path: &str) -> PathBuf {
    resource_dir().join(path)
}
