#![allow(dead_code)]
use crate::context::AppContext;
use crate::settings::{get_settings, write_settings, ModelUnloadTimeout};
use serde::Serialize;

#[derive(Serialize)]
pub struct ModelLoadStatus {
    is_loaded: bool,
    current_model: Option<String>,
}

pub fn set_model_unload_timeout(ctx: &AppContext, timeout: ModelUnloadTimeout) {
    let mut settings = get_settings(ctx);
    settings.model_unload_timeout = timeout;
    write_settings(ctx, settings);
}

pub fn get_model_load_status(ctx: &AppContext) -> Result<ModelLoadStatus, String> {
    Ok(ModelLoadStatus {
        is_loaded: ctx.transcription.is_model_loaded(),
        current_model: ctx.transcription.get_current_model(),
    })
}

pub fn unload_model_manually(ctx: &AppContext) -> Result<(), String> {
    ctx.transcription
        .unload_model()
        .map_err(|e| format!("Failed to unload model: {}", e))
}
