//! Update checking — GitHub releases.
//!
//! Checks the GitHub releases API for a newer version than the running one
//! and surfaces the result as a toast. Distribution-managed updates (Flatpak /
//! GNOME Software) take over on those installs; this path covers the raw
//! binary / deb installs.

use libadwaita::prelude::*;

/// Version of the running binary (debug builds note the dev suffix).
pub fn current_version() -> String {
    if cfg!(debug_assertions) {
        format!("{} (Dev)", env!("CARGO_PKG_VERSION"))
    } else {
        env!("CARGO_PKG_VERSION").to_string()
    }
}

/// Compare two dotted numeric versions; `true` when `candidate` is newer.
fn is_newer(candidate: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.split('.')
            .filter_map(|part| {
                part.chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .parse()
                    .ok()
            })
            .collect()
    };
    let cand = parse(candidate);
    let cur = parse(current);
    for (c, k) in cand.iter().zip(cur.iter()) {
        if c != k {
            return c > k;
        }
    }
    cand.len() > cur.len()
}

/// Query the GitHub releases API for the latest release tag.
async fn latest_release() -> Option<String> {
    let client = reqwest::Client::new();
    let resp = client
        .get("https://api.github.com/repos/ATClus/otush/releases/latest")
        .header("User-Agent", format!("otush/{}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: serde_json::Value = resp.json().await.ok()?;
    json.get("tag_name")?
        .as_str()?
        .trim_start_matches('v')
        .to_string()
        .into()
}

/// Check for a newer release and notify via the window's toast overlay.
/// Runs the network request on the Tokio runtime and marshals the toast back
/// to the GTK main thread.
pub fn check_for_updates(toasts: &libadwaita::ToastOverlay) {
    let toasts = glib::SendWeakRef::from(toasts.downgrade());
    crate::runtime::spawn(async move {
        let result = match latest_release().await {
            Some(latest) if is_newer(&latest, env!("CARGO_PKG_VERSION")) => Some(format!(
                "Version {latest} is available — check the GitHub releases page."
            )),
            _ => None,
        };
        let toasts = toasts.clone();
        glib::MainContext::default().invoke(move || {
            let weak = toasts.into_weak_ref();
            let Some(toasts) = weak.upgrade() else {
                return;
            };
            match result {
                Some(message) => toasts.add_toast(libadwaita::Toast::new(&message)),
                None => toasts.add_toast(libadwaita::Toast::new("You're up to date.")),
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn version_comparison() {
        assert!(is_newer("0.10.0", "0.9.6"));
        assert!(is_newer("1.0.0", "0.9.6"));
        assert!(is_newer("0.9.10", "0.9.6"));
        assert!(!is_newer("0.9.6", "0.9.6"));
        assert!(!is_newer("0.9.5", "0.9.6"));
        assert!(!is_newer("0.8.0", "0.9.6"));
    }
}
