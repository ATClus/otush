//! Unified action dispatcher for global shortcuts and floating palettes.
//!
//! Split into modules by concern; this facade re-exports the public surface
//! so all existing `crate::actions::X` paths keep working:
//! [`transcribe`] (transcription action + text pipeline),
//! [`palette`] (palette/utility actions + `ACTION_MAP`),
//! [`support`] (cancellation primitives). The [`ShortcutAction`] trait and
//! the public action structs stay here.

pub mod palette;
pub mod support;
pub mod transcribe;

use crate::context::AppContext;

pub use palette::ACTION_MAP;
pub use transcribe::{
    post_process_text_with_prompt, post_process_text_with_prompt_and_context,
    post_process_transcription, process_transcription_output, resolve_effective_language,
};

/// Action triggered by a global shortcut or floating palette entry.
pub trait ShortcutAction: Send + Sync {
    fn start(&self, ctx: &AppContext, binding_id: &str, shortcut_str: &str);
    fn stop(&self, ctx: &AppContext, binding_id: &str, shortcut_str: &str);
}

#[cfg(test)]
mod tests {
    use super::support::complete_unless_cancelled;
    use super::transcribe::{
        is_blank_transcription, should_use_streaming_overlay, strip_think_block,
    };
    use crate::settings::OverlayStyle;
    use std::future;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn blank_transcription_is_detected() {
        assert!(is_blank_transcription(""));
        assert!(is_blank_transcription("   "));
        assert!(is_blank_transcription("\t\n  \r\n"));
    }

    #[test]
    fn non_blank_transcription_is_kept() {
        assert!(!is_blank_transcription("hello"));
        assert!(!is_blank_transcription("  hello  "));
    }

    #[test]
    fn completed_operation_returns_its_output() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let result = runtime.block_on(complete_unless_cancelled(future::ready("done"), || false));

        assert_eq!(result, Some("done"));
    }

    #[test]
    fn pending_operation_stops_after_cancellation() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancelled_for_thread = Arc::clone(&cancelled);
        let cancel_thread = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            cancelled_for_thread.store(true, Ordering::Release);
        });

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let result = runtime.block_on(complete_unless_cancelled(future::pending::<()>(), || {
            cancelled.load(Ordering::Acquire)
        }));

        cancel_thread.join().unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn leading_think_block_is_stripped() {
        assert_eq!(
            strip_think_block("<think>pondering...</think>Cleaned text."),
            "Cleaned text."
        );
        assert_eq!(
            strip_think_block("  \n<think>multi\nline</think>\n  Cleaned text."),
            "Cleaned text."
        );
    }

    #[test]
    fn content_without_think_block_is_unchanged() {
        assert_eq!(strip_think_block("Cleaned text."), "Cleaned text.");
        assert_eq!(
            strip_think_block("Mentions <think> mid-sentence."),
            "Mentions <think> mid-sentence."
        );
        // Unclosed block: leave untouched rather than guess
        assert_eq!(
            strip_think_block("<think>never closed"),
            "<think>never closed"
        );
    }

    #[test]
    fn live_overlay_uses_streaming_states_only_for_streaming_models() {
        assert!(should_use_streaming_overlay(OverlayStyle::Live, true));
        assert!(!should_use_streaming_overlay(OverlayStyle::Live, false));
        assert!(!should_use_streaming_overlay(OverlayStyle::Minimal, true));
        assert!(!should_use_streaming_overlay(OverlayStyle::None, true));
    }

    #[test]
    fn action_map_registers_transcribe_meeting() {
        assert!(super::ACTION_MAP.contains_key("transcribe"));
        assert!(super::ACTION_MAP.contains_key("transcribe_with_post_process"));
        assert!(super::ACTION_MAP.contains_key("transcribe_meeting"));
        assert!(super::ACTION_MAP.contains_key("transform_selection"));
        assert!(super::ACTION_MAP.contains_key("show_history"));
        assert!(super::ACTION_MAP.contains_key("search_overlay"));
        assert!(super::ACTION_MAP.contains_key("agent_chat"));
        assert!(super::ACTION_MAP.contains_key("quick_note"));
        assert!(super::ACTION_MAP.contains_key("todo_palette"));
        assert!(super::ACTION_MAP.contains_key("doc_parser"));
        assert!(super::ACTION_MAP.contains_key("cancel"));
    }
}
