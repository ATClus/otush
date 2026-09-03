//! Resolution of bundled resource files (sounds, tray icons, VAD model,
//! default settings).

use std::path::PathBuf;

/// Directory containing bundled resources.
///
/// - Development: `resources/` next to the crate manifest (embedded at compile
///   time via `CARGO_MANIFEST_DIR`).
/// - Installed (deb/flatpak): `/usr/share/otush/resources` (or a `resources`
///   dir next to the executable, which also covers portable-style installs).
pub fn resource_dir() -> PathBuf {
    // Dev layout wins whenever the manifest-adjacent resources exist at
    // runtime. Stale copies that end up next to build artifacts (e.g. an old
    // `target/debug/resources` from a previous project generation) must never
    // shadow the real resource set.
    #[cfg(debug_assertions)]
    {
        let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources");
        if dev.is_dir() {
            return dev;
        }
    }
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
    // Last resort: a bare `resources` relative to the CWD.
    PathBuf::from("resources")
}
