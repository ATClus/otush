//! Keyboard implementation selection (split from `shortcut/mod.rs`; same behavior, same paths).

use super::portal;
use crate::settings::KeyboardImplementation;

pub(crate) fn validate_shortcut_for_implementation(
    raw: &str,
    _implementation: KeyboardImplementation,
) -> Result<(), String> {
    portal::validate_shortcut(raw)
}
