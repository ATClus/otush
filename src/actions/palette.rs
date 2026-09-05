//! Palette/utility actions plus the global `ACTION_MAP`. (split from `actions.rs`; same behavior).

use super::transcribe::{TranscribeAction, TranscribeMode};
use super::ShortcutAction;
use crate::context::AppContext;
use crate::utils::cancel_current_operation;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

pub(crate) struct CancelAction;

impl ShortcutAction for CancelAction {
    fn start(&self, ctx: &AppContext, _binding_id: &str, _shortcut_str: &str) {
        cancel_current_operation(ctx);
    }

    fn stop(&self, _ctx: &AppContext, _binding_id: &str, _shortcut_str: &str) {
        // Nothing to do on stop for cancel
    }
}

// Test Action

pub(crate) struct TestAction;

impl ShortcutAction for TestAction {
    fn start(&self, _ctx: &AppContext, binding_id: &str, shortcut_str: &str) {
        log::info!(
            "Shortcut ID '{}': Started - {} (App: Otush)",
            binding_id,
            shortcut_str
        );
    }

    fn stop(&self, _ctx: &AppContext, binding_id: &str, shortcut_str: &str) {
        log::info!(
            "Shortcut ID '{}': Stopped - {} (App: Otush)",
            binding_id,
            shortcut_str
        );
    }
}

#[derive(Debug)]
pub struct TransformSelectionAction;

impl ShortcutAction for TransformSelectionAction {
    fn start(&self, ctx: &AppContext, _binding_id: &str, _shortcut_str: &str) {
        log::info!("TransformSelectionAction triggered");
        crate::ui::prompt_palette::show_prompt_palette(ctx);
    }

    fn stop(&self, _ctx: &AppContext, _binding_id: &str, _shortcut_str: &str) {}
}

#[derive(Debug)]
pub struct ShowHistoryAction;

impl ShortcutAction for ShowHistoryAction {
    fn start(&self, ctx: &AppContext, _binding_id: &str, _shortcut_str: &str) {
        log::info!("ShowHistoryAction triggered");
        crate::ui::history_palette::toggle_history_palette(ctx);
    }

    fn stop(&self, _ctx: &AppContext, _binding_id: &str, _shortcut_str: &str) {}
}

#[derive(Debug)]
pub struct SearchOverlayAction;

impl ShortcutAction for SearchOverlayAction {
    fn start(&self, ctx: &AppContext, _binding_id: &str, _shortcut_str: &str) {
        log::info!("SearchOverlayAction triggered");
        crate::ui::chat_overlay::show_search_mode(ctx);
    }

    fn stop(&self, _ctx: &AppContext, _binding_id: &str, _shortcut_str: &str) {}
}

#[derive(Debug)]
pub struct AgentChatAction;

impl ShortcutAction for AgentChatAction {
    fn start(&self, ctx: &AppContext, _binding_id: &str, _shortcut_str: &str) {
        log::info!("AgentChatAction triggered");
        crate::ui::chat_overlay::show_agent_chat(ctx);
    }

    fn stop(&self, _ctx: &AppContext, _binding_id: &str, _shortcut_str: &str) {}
}

#[derive(Debug)]
pub struct QuickNoteAction;

impl ShortcutAction for QuickNoteAction {
    fn start(&self, ctx: &AppContext, _binding_id: &str, _shortcut_str: &str) {
        log::info!("QuickNoteAction triggered");
        crate::ui::notes_palette::show_notes_palette(ctx);
    }

    fn stop(&self, _ctx: &AppContext, _binding_id: &str, _shortcut_str: &str) {}
}

#[derive(Debug)]
pub struct TodoPaletteAction;

impl ShortcutAction for TodoPaletteAction {
    fn start(&self, ctx: &AppContext, _binding_id: &str, _shortcut_str: &str) {
        log::info!("TodoPaletteAction triggered");
        crate::ui::todo_palette::show_todo_palette(ctx);
    }

    fn stop(&self, _ctx: &AppContext, _binding_id: &str, _shortcut_str: &str) {}
}

#[derive(Debug)]
pub struct DocParserAction;

impl ShortcutAction for DocParserAction {
    fn start(&self, ctx: &AppContext, _binding_id: &str, _shortcut_str: &str) {
        log::info!("DocParserAction triggered");
        crate::ui::doc_parser::show_doc_parser(ctx);
    }

    fn stop(&self, _ctx: &AppContext, _binding_id: &str, _shortcut_str: &str) {}
}

// Static Action Map

pub static ACTION_MAP: LazyLock<HashMap<String, Arc<dyn ShortcutAction>>> = LazyLock::new(|| {
    let mut map = HashMap::new();
    map.insert(
        "transcribe".to_string(),
        Arc::new(TranscribeAction {
            mode: TranscribeMode::Standard,
        }) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "transcribe_with_post_process".to_string(),
        Arc::new(TranscribeAction {
            mode: TranscribeMode::PostProcess,
        }) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "transcribe_meeting".to_string(),
        Arc::new(TranscribeAction {
            mode: TranscribeMode::Meeting,
        }) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "transform_selection".to_string(),
        Arc::new(TransformSelectionAction) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "show_history".to_string(),
        Arc::new(ShowHistoryAction) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "search_overlay".to_string(),
        Arc::new(SearchOverlayAction) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "agent_chat".to_string(),
        Arc::new(AgentChatAction) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "quick_note".to_string(),
        Arc::new(QuickNoteAction) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "todo_palette".to_string(),
        Arc::new(TodoPaletteAction) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "doc_parser".to_string(),
        Arc::new(DocParserAction) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "cancel".to_string(),
        Arc::new(CancelAction) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "test".to_string(),
        Arc::new(TestAction) as Arc<dyn ShortcutAction>,
    );
    map
});
