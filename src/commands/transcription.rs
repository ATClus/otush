//! Transcription engine control commands.

#![allow(dead_code)]
use crate::context::AppContext;
use crate::settings::{get_settings, write_settings, ModelUnloadTimeout};
use serde::Serialize;

/// Information about whether a model is currently in memory.
#[derive(Serialize)]
pub struct ModelLoadStatus {
    pub is_loaded: bool,
    pub current_model: Option<String>,
}

/// Set and persist the model idle unload timeout policy.
pub fn set_model_unload_timeout(ctx: &AppContext, timeout: ModelUnloadTimeout) {
    let mut settings = get_settings(ctx);
    settings.model_unload_timeout = timeout;
    write_settings(ctx, settings);
}

/// Query whether a model is loaded in memory and which model ID it is.
pub fn get_model_load_status(ctx: &AppContext) -> Result<ModelLoadStatus, String> {
    Ok(ModelLoadStatus {
        is_loaded: ctx.transcription.is_model_loaded(),
        current_model: ctx.transcription.get_current_model(),
    })
}

/// Manually unload the currently resident model to free RAM/VRAM.
pub fn unload_model_manually(ctx: &AppContext) -> Result<(), String> {
    ctx.transcription
        .unload_model()
        .map_err(|e| format!("Failed to unload model: {}", e))
}
