use crate::audio_feedback::{play_feedback_sound, play_feedback_sound_blocking, SoundType};
use crate::audio_toolkit::{is_microphone_access_denied, is_no_input_device_error, VadPolicy};
use crate::context::{AppContext, AppEvent};
use crate::managers::transcription::StreamWorkKind;
use crate::settings::{get_settings, AppSettings, OverlayStyle};
use crate::shortcut;
use crate::tray::{set_tray_state, TrayIconState};
use crate::utils::{
    self, show_processing_overlay, show_recording_overlay, show_transcribing_overlay,
};
use ferrous_opencc::{config::BuiltinConfig, OpenCC};
use log::{debug, error, info, warn};
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// Drop guard that notifies the [`TranscriptionCoordinator`] when the
/// transcription pipeline finishes — whether it completes normally or panics.
struct FinishGuard(AppContext);
impl Drop for FinishGuard {
    fn drop(&mut self) {
        self.0.coordinator.notify_processing_finished();
        // The pipeline just freed its large transient buffers (captured PCM,
        // WAV copy, engine scratch); hand the cached pages back to the OS so
        // they don't sit in malloc arenas until they get swapped out (#1792).
        crate::memory::trim_freed_memory();
    }
}

// Shortcut Action Trait
pub trait ShortcutAction: Send + Sync {
    fn start(&self, ctx: &AppContext, binding_id: &str, shortcut_str: &str);
    fn stop(&self, ctx: &AppContext, binding_id: &str, shortcut_str: &str);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranscribeMode {
    Standard,
    PostProcess,
    Meeting,
}

// Transcribe Action
pub struct TranscribeAction {
    pub mode: TranscribeMode,
}

/// Field name for structured output JSON schema
const TRANSCRIPTION_FIELD: &str = "transcription";

/// Strip invisible Unicode characters that some LLMs may insert
fn strip_invisible_chars(s: &str) -> String {
    s.replace(['\u{200B}', '\u{200C}', '\u{200D}', '\u{FEFF}'], "")
}

/// Strip a leading `<think>...</think>` block. Some endpoints can't disable
/// reasoning, and some local servers put the reasoning text into `content`
/// instead of a separate field — without this the user would get the model's
/// chain of thought pasted along with the cleaned transcription.
fn strip_think_block(s: &str) -> &str {
    if let Some(rest) = s.trim_start().strip_prefix("<think>") {
        if let Some(end) = rest.find("</think>") {
            return rest[end + "</think>".len()..].trim_start();
        }
    }
    s
}

/// Returns `true` when a transcription has no meaningful content to
/// post-process (empty or whitespace-only). Used to skip the post-processing
/// LLM call when nothing was actually transcribed, which would otherwise make
/// the model reply with an error message such as "you need to provide the
/// transcription".
fn is_blank_transcription(transcription: &str) -> bool {
    transcription.trim().is_empty()
}

async fn complete_unless_cancelled<F, C>(operation: F, is_cancelled: C) -> Option<F::Output>
where
    F: Future,
    C: Fn() -> bool,
{
    tokio::pin!(operation);

    loop {
        if is_cancelled() {
            return None;
        }

        if let Ok(result) =
            tokio::time::timeout(CANCELLATION_POLL_INTERVAL, operation.as_mut()).await
        {
            return Some(result);
        }
    }
}

fn should_use_streaming_overlay(style: OverlayStyle, is_streaming: bool) -> bool {
    style == OverlayStyle::Live && is_streaming
}

pub(crate) async fn post_process_transcription(
    settings: &crate::settings::AppSettings,
    transcription: &str,
) -> Option<String> {
    let template_ctx = crate::template::TemplateContext::new(transcription);
    post_process_transcription_with_context(settings, transcription, &template_ctx).await
}

pub(crate) async fn post_process_transcription_with_context(
    settings: &crate::settings::AppSettings,
    transcription: &str,
    template_ctx: &crate::template::TemplateContext,
) -> Option<String> {
    if is_blank_transcription(transcription) {
        debug!("Post-processing skipped because the transcription is empty");
        return None;
    }

    // 1. Resolve prompt
    let prompt_obj = if let Some(selected_id) = &settings.post_process_selected_prompt_id {
        settings
            .post_process_prompts
            .iter()
            .find(|p| &p.id == selected_id)
            .or_else(|| settings.post_process_prompts.first())
    } else {
        settings.post_process_prompts.first()
    };

    let prompt_obj = match prompt_obj {
        Some(p) => p,
        None => {
            debug!("Post-processing skipped because no prompts are available");
            return None;
        }
    };

    post_process_text_with_prompt_and_context(settings, transcription, prompt_obj, template_ctx)
        .await
}

#[allow(dead_code)]
pub(crate) async fn post_process_meeting_transcription(
    settings: &crate::settings::AppSettings,
    transcription: &str,
) -> (Option<String>, Option<String>) {
    let template_ctx = crate::template::TemplateContext::new(transcription);
    post_process_meeting_transcription_with_context(settings, transcription, &template_ctx).await
}

pub(crate) async fn post_process_meeting_transcription_with_context(
    settings: &crate::settings::AppSettings,
    transcription: &str,
    template_ctx: &crate::template::TemplateContext,
) -> (Option<String>, Option<String>) {
    if is_blank_transcription(transcription) {
        debug!("Meeting post-processing skipped because the transcription is empty");
        return (None, None);
    }

    // Find the meeting minutes prompt (by id "default_meeting_minutes" or name containing "meeting")
    let prompt_obj = settings
        .post_process_prompts
        .iter()
        .find(|p| p.id == "default_meeting_minutes" || p.name.to_lowercase().contains("meeting"))
        .or_else(|| settings.post_process_prompts.first());

    let prompt_obj = match prompt_obj {
        Some(p) => p,
        None => {
            debug!("Meeting post-processing skipped because no prompts are available");
            return (None, None);
        }
    };

    let processed = post_process_text_with_prompt_and_context(
        settings,
        transcription,
        prompt_obj,
        template_ctx,
    )
    .await;
    (processed, Some(prompt_obj.prompt.clone()))
}

pub(crate) async fn post_process_text_with_prompt(
    settings: &crate::settings::AppSettings,
    transcription: &str,
    prompt_obj: &crate::settings::LLMPrompt,
) -> Option<String> {
    let template_ctx = crate::template::TemplateContext::new(transcription);
    post_process_text_with_prompt_and_context(settings, transcription, prompt_obj, &template_ctx)
        .await
}

pub(crate) async fn post_process_text_with_prompt_and_context(
    settings: &crate::settings::AppSettings,
    transcription: &str,
    prompt_obj: &crate::settings::LLMPrompt,
    template_ctx: &crate::template::TemplateContext,
) -> Option<String> {
    if is_blank_transcription(transcription) {
        debug!("Post-processing skipped because input text is empty");
        return None;
    }

    let prompt = &prompt_obj.prompt;
    if prompt.trim().is_empty() {
        debug!("Post-processing skipped because the selected prompt is empty");
        return None;
    }

    info!(
        "Starting LLM post-processing for prompt '{}' (id: '{}') with input text ({} chars):\n{}",
        prompt_obj.name,
        prompt_obj.id,
        transcription.len(),
        transcription
    );

    // 2. Build ordered candidate providers
    let mut candidate_providers: Vec<&crate::settings::PostProcessProvider> = Vec::new();

    // Check if prompt has a preferred provider bound to it
    if let Some(preferred_id) = &prompt_obj.preferred_provider_id {
        if let Some(pref) = settings
            .post_process_providers
            .iter()
            .find(|p| &p.id == preferred_id && p.enabled)
        {
            candidate_providers.push(pref);
        }
    }

    // Add remaining enabled providers in priority order
    for provider in &settings.post_process_providers {
        if provider.enabled && !candidate_providers.iter().any(|p| p.id == provider.id) {
            candidate_providers.push(provider);
        }
    }

    if candidate_providers.is_empty() {
        warn!("Post-processing skipped because no enabled providers were found");
        return None;
    }

    // 3. Fallback chain loop
    for provider in candidate_providers {
        let model = settings
            .post_process_models
            .get(&provider.id)
            .cloned()
            .unwrap_or_default();

        let api_key = settings
            .post_process_api_keys
            .get(&provider.id)
            .cloned()
            .unwrap_or_default();

        // If provider requires an API key and none is set, skip
        if api_key.trim().is_empty() && provider.id != "custom" && provider.id != "ollama" {
            debug!("Skipping provider '{}': missing API key", provider.id);
            continue;
        }

        let effective_model = if model.trim().is_empty() {
            crate::settings::default_model_for_provider(&provider.id)
        } else {
            model
        };

        if effective_model.trim().is_empty() {
            debug!("Skipping provider '{}': no model configured", provider.id);
            continue;
        }

        info!(
            "Attempting LLM post-processing with provider '{}' (model: {}, structured: {})",
            provider.id, effective_model, provider.supports_structured_output
        );

        let system_prompt =
            crate::template::expand_template_for_system_prompt(prompt, template_ctx);
        let user_content = transcription.to_string();

        info!(
            "Post-processing payload:\n--- System Prompt ---\n{}\n--- User Content ---\n{}",
            system_prompt, user_content
        );

        if provider.supports_structured_output {
            let json_schema = serde_json::json!({
                "type": "object",
                "properties": {
                    (TRANSCRIPTION_FIELD): {
                        "type": "string",
                        "description": "The cleaned and processed transcription text"
                    }
                },
                "required": [TRANSCRIPTION_FIELD],
                "additionalProperties": false
            });

            match crate::llm_client::send_chat_completion_with_schema(
                provider,
                api_key.clone(),
                &effective_model,
                user_content.clone(),
                Some(system_prompt.clone()),
                Some(json_schema),
                false,
            )
            .await
            {
                Ok(Some(content)) => {
                    info!("LLM message content (structured output):\n{}", content);
                    let content = strip_think_block(&content);
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(content) {
                        if let Some(transcription_val) =
                            json.get(TRANSCRIPTION_FIELD).and_then(|t| t.as_str())
                        {
                            let result = strip_invisible_chars(transcription_val);
                            info!(
                                "Post-processing succeeded via provider '{}' (model: {}). Extracted '{}' field:\n{}",
                                provider.id, effective_model, TRANSCRIPTION_FIELD, result
                            );
                            return Some(result);
                        } else {
                            warn!(
                                "Structured JSON parsed, but '{}' field was missing: {:?}",
                                TRANSCRIPTION_FIELD, json
                            );
                        }
                    } else {
                        warn!("Content was not valid JSON, falling back to cleaned raw content.");
                    }
                    let cleaned = strip_invisible_chars(content);
                    info!(
                        "Post-processing succeeded via provider '{}' (model: {}). Final text:\n{}",
                        provider.id, effective_model, cleaned
                    );
                    return Some(cleaned);
                }
                Ok(None) => {
                    warn!(
                        "Provider '{}' returned empty response. Trying next candidate...",
                        provider.id
                    );
                    continue;
                }
                Err(e) => {
                    warn!(
                        "Provider '{}' structured request failed: {e}. Trying standard fallback...",
                        provider.id
                    );
                }
            }
        }

        // Standard completion attempt
        let processed_prompt = if crate::template::has_input_placeholder(prompt) {
            crate::template::expand_template(prompt, template_ctx)
        } else {
            let expanded_instruction = crate::template::expand_template(prompt, template_ctx);
            format!("{}\n\n{}", expanded_instruction.trim(), transcription)
        };

        info!(
            "Standard completion prompt for provider '{}' (model: {}):\n{}",
            provider.id, effective_model, processed_prompt
        );
        match crate::llm_client::send_chat_completion(
            provider,
            api_key,
            &effective_model,
            processed_prompt,
            false,
        )
        .await
        {
            Ok(Some(content)) => {
                info!("LLM message content (standard completion):\n{}", content);
                let cleaned = strip_invisible_chars(strip_think_block(&content));
                info!(
                    "Post-processing succeeded via provider '{}' (model: {}). Final text:\n{}",
                    provider.id, effective_model, cleaned
                );
                return Some(cleaned);
            }
            Ok(None) => {
                warn!(
                    "Provider '{}' returned empty response. Trying next candidate...",
                    provider.id
                );
            }
            Err(e) => {
                warn!(
                    "Provider '{}' failed: {e}. Trying next candidate...",
                    provider.id
                );
            }
        }
    }

    warn!("All configured post-processing providers failed. Falling back to raw transcription.");
    None
}

async fn maybe_convert_chinese_variant(
    effective_language: &str,
    transcription: &str,
) -> Option<String> {
    // Gate on the language the model actually transcribed in (the effective
    // language), not the persisted intent. A leftover zh-Hans/zh-Hant intent
    // from a previously selected model must not run OpenCC S2T/T2S over output a
    // non-Chinese model produced — that would silently rewrite any shared CJK
    // characters (e.g. Japanese kanji) in the result.
    let is_simplified = effective_language == "zh-Hans";
    let is_traditional = effective_language == "zh-Hant";

    if !is_simplified && !is_traditional {
        debug!("effective language is not Simplified or Traditional Chinese; skipping conversion");
        return None;
    }

    debug!(
        "Starting Chinese variant conversion using OpenCC for language: {}",
        effective_language
    );

    // Use OpenCC to convert based on selected language
    let config = if is_simplified {
        // Convert Traditional Chinese to Simplified Chinese
        BuiltinConfig::Tw2sp
    } else {
        // Convert Simplified Chinese to Traditional Chinese
        BuiltinConfig::S2tw
    };

    match OpenCC::from_config(config) {
        Ok(converter) => {
            let converted = converter.convert(transcription);
            debug!(
                "OpenCC translation completed. Input length: {}, Output length: {}",
                transcription.len(),
                converted.len()
            );
            Some(converted)
        }
        Err(e) => {
            error!("Failed to initialize OpenCC converter: {}. Falling back to original transcription.", e);
            None
        }
    }
}

pub(crate) struct ProcessedTranscription {
    pub final_text: String,
    pub post_processed_text: Option<String>,
    pub post_process_prompt: Option<String>,
}

/// Resolve the persisted language *intent* into the language the currently-loaded
/// model will actually use — the same capability-aware coercion the transcription
/// paths apply (see [`crate::managers::model::effective_language`]). Post-processing
/// resolves it independently so it agrees with the language the transcription ran
/// in, without threading a value through the pipeline.
pub(crate) fn resolve_effective_language(ctx: &AppContext, settings: &AppSettings) -> String {
    let tm = &ctx.transcription;
    let model_manager = &ctx.model;
    let active_model = tm
        .get_current_model()
        .unwrap_or_else(|| settings.selected_model.clone());
    match model_manager.get_model_info(&active_model) {
        Some(info) => crate::managers::model::effective_language(
            &settings.selected_language,
            &info.supported_languages,
            info.supports_language_detection,
        ),
        None => settings.selected_language.clone(),
    }
}

pub(crate) async fn process_transcription_output(
    ctx: &AppContext,
    transcription: &str,
    post_process: bool,
    is_meeting: bool,
) -> ProcessedTranscription {
    let settings = get_settings(ctx);
    let mut final_text = transcription.to_string();
    let mut post_processed_text: Option<String> = None;
    let mut post_process_prompt: Option<String> = None;

    // Resolve the language the transcription actually ran in (the persisted
    // intent coerced against the loaded model's capabilities) so OpenCC keys off
    // the effective language rather than a possibly-stale intent.
    let effective_language = resolve_effective_language(ctx, &settings);
    if let Some(converted_text) =
        maybe_convert_chinese_variant(&effective_language, transcription).await
    {
        final_text = converted_text;
    }

    let template_ctx = crate::template::TemplateContext::gather(ctx, &final_text, None, None, None);

    if is_meeting {
        let (processed_text, prompt_used) =
            post_process_meeting_transcription_with_context(&settings, &final_text, &template_ctx)
                .await;
        if let Some(processed) = processed_text {
            post_processed_text = Some(processed.clone());
            final_text = processed;
            post_process_prompt = prompt_used;
        }
    } else if post_process {
        if let Some(processed_text) =
            post_process_transcription_with_context(&settings, &final_text, &template_ctx).await
        {
            post_processed_text = Some(processed_text.clone());
            final_text = processed_text;

            if let Some(prompt_id) = &settings.post_process_selected_prompt_id {
                if let Some(prompt) = settings
                    .post_process_prompts
                    .iter()
                    .find(|prompt| &prompt.id == prompt_id)
                {
                    post_process_prompt = Some(prompt.prompt.clone());
                }
            }
        }
    } else if final_text != transcription {
        post_processed_text = Some(final_text.clone());
    }

    ProcessedTranscription {
        final_text,
        post_processed_text,
        post_process_prompt,
    }
}

impl ShortcutAction for TranscribeAction {
    fn start(&self, ctx: &AppContext, binding_id: &str, _shortcut_str: &str) {
        let start_time = Instant::now();
        debug!("TranscribeAction::start called for binding: {}", binding_id);

        // Load model in the background if local mode is enabled
        let tm = &ctx.transcription;
        let rm = &ctx.audio;
        let mut recording_error: Option<String> = None;

        let settings = get_settings(ctx);
        let is_always_on = settings.always_on_microphone;

        // Load ASR model (if local mode is active) and VAD model in parallel
        let kickoff_started = Instant::now();
        if settings.local_transcription_enabled {
            tm.initiate_model_load();
        }
        let rm_clone = Arc::clone(rm);
        std::thread::spawn(move || {
            if let Err(e) = rm_clone.preload_vad() {
                debug!("VAD pre-load failed: {}", e);
            }
        });
        let kickoff_elapsed = kickoff_started.elapsed();

        let binding_id = binding_id.to_string();
        let tray_started = Instant::now();
        set_tray_state(ctx, TrayIconState::Recording);
        let tray_elapsed = tray_started.elapsed();

        // Get the microphone mode to determine audio feedback timing
        let plan_started = Instant::now();
        let selected_model_info = if settings.local_transcription_enabled {
            ctx.model.get_model_info(&settings.selected_model)
        } else {
            None
        };

        if settings.local_transcription_enabled && selected_model_info.is_none() {
            warn!(
                "No speech-to-text model selected or found on disk: '{}'",
                settings.selected_model
            );
            recording_error =
                Some("No model selected. Please choose a model in Settings -> Models, or enable Cloud Providers in Settings -> Providers.".to_string());
        }

        // Use the app-facing model capability as the single pre-recording source
        // for live streaming decisions. Unknown support is represented as false
        // until the model registry is updated by discovery or runtime load.
        let model_supports_streaming = if settings.local_transcription_enabled {
            selected_model_info
                .as_ref()
                .map(|m| m.supports_streaming)
                .unwrap_or(false)
        } else {
            settings.is_deepgram_streaming_active()
        };
        let is_meeting = self.mode == TranscribeMode::Meeting;
        let vad_policy = if is_meeting || !settings.vad_enabled {
            VadPolicy::Disabled
        } else if model_supports_streaming {
            VadPolicy::Streaming
        } else {
            VadPolicy::Offline
        };
        if recording_error.is_none() && model_supports_streaming {
            tm.start_stream();
        }
        let plan_elapsed = plan_started.elapsed();

        // Sizing the overlay follows the same advertised capability. A model that
        // doesn't stream (or whose capability is not known yet) gets the compact
        // pill instead of an oversized transparent live window.
        let overlay_started = Instant::now();
        if recording_error.is_none() {
            match settings.overlay_style {
                OverlayStyle::Live if model_supports_streaming => {
                    if is_meeting {
                        utils::show_meeting_streaming_overlay(ctx);
                    } else {
                        utils::show_streaming_overlay(ctx);
                    }
                }
                OverlayStyle::Live | OverlayStyle::Minimal => {
                    if is_meeting {
                        utils::show_meeting_recording_overlay(ctx);
                    } else {
                        show_recording_overlay(ctx);
                    }
                }
                OverlayStyle::None => {} // show_overlay_state no-ops on None anyway
            }
        }
        // Everything above runs before capture can begin, so each span here is
        // added keypress->capture latency.
        debug!(
            "start-path pre-recording steps: model_kickoff={:?} tray={:?} settings+stream_plan={:?} overlay={:?}",
            kickoff_elapsed,
            tray_elapsed,
            plan_elapsed,
            overlay_started.elapsed()
        );
        debug!("Microphone mode - always_on: {}", is_always_on);

        let recording_start_time = Instant::now();
        if recording_error.is_none() {
            match rm.try_start_recording(&binding_id, vad_policy) {
                Ok(readiness) => {
                    debug!(
                        "Recording request accepted in {:?}; waiting for first microphone samples",
                        recording_start_time.elapsed()
                    );
                    let generation = readiness.generation();
                    let app_clone = ctx.clone();
                    let rm_clone = Arc::clone(rm);
                    std::thread::spawn(move || {
                        if !readiness.wait() {
                            debug!("Microphone readiness wait ended without receiving samples");
                            return;
                        }

                        // Development-only preview hook for evaluating the brief
                        // arming animation on hardware that normally starts too fast
                        // to make it visible.
                        #[cfg(debug_assertions)]
                        if let Ok(delay_ms) = std::env::var("OTUSH_DEBUG_MIC_READY_DELAY_MS")
                            .unwrap_or_default()
                            .parse::<u64>()
                        {
                            let delay_ms = delay_ms.min(10_000);
                            if delay_ms > 0 {
                                debug!(
                                    "Delaying microphone-ready cue by {delay_ms}ms for UI preview"
                                );
                                std::thread::sleep(Duration::from_millis(delay_ms));
                            }
                        }

                        if !rm_clone.is_recording_readiness_current(generation) {
                            debug!("Microphone became ready for an inactive recording");
                            return;
                        }

                        debug!("Microphone is receiving samples; recording is ready");
                        utils::emit_recording_ready(&app_clone);

                        // The start chime is a readiness cue, so it must follow the
                        // first real input callback rather than Stream::play() or a
                        // fixed delay. The helper returns immediately when feedback
                        // is disabled; mute still follows the same readiness point.
                        if rm_clone.is_recording_readiness_current(generation) {
                            play_feedback_sound_blocking(&app_clone, SoundType::Start);
                        }
                        if rm_clone.is_recording_readiness_current(generation) {
                            rm_clone.apply_mute();
                        }
                    });
                }
                Err(e) => {
                    debug!("Failed to start recording: {}", e);
                    recording_error = Some(e);
                }
            }
        }

        if recording_error.is_none() {
            // Dynamically register the cancel shortcut in a separate task to avoid deadlock
            shortcut::register_cancel_shortcut(ctx);
        } else {
            // Starting failed (for example due to missing model or blocked microphone permissions).
            // Revert UI and coordinator state so we don't stay stuck in recording mode.
            tm.cancel_stream();
            utils::hide_recording_overlay(ctx);
            set_tray_state(ctx, TrayIconState::Idle);
            ctx.coordinator.notify_cancel(false);
            if let Some(err) = recording_error {
                let error_type = if is_microphone_access_denied(&err) {
                    "microphone_permission_denied"
                } else if is_no_input_device_error(&err) {
                    "no_input_device"
                } else {
                    "unknown"
                };
                ctx.bus.send(AppEvent::RecordingError(
                    crate::context::RecordingErrorEvent {
                        error_type: error_type.to_string(),
                        detail: Some(err.clone()),
                    },
                ));
                ctx.bus.send(AppEvent::TranscriptionError(err));
            }
        }

        debug!(
            "TranscribeAction::start completed in {:?}",
            start_time.elapsed()
        );
    }

    fn stop(&self, ctx: &AppContext, binding_id: &str, _shortcut_str: &str) {
        // Stop any pending Push-to-Talk release polling
        crate::shortcut::ptt::cancel_ptt_release_watcher();

        // Prevent a slow microphone from emitting a ready event or start chime
        // after the user has already requested stop.
        ctx.audio.invalidate_recording_readiness();

        // Unregister the cancel shortcut when transcription stops
        shortcut::unregister_cancel_shortcut(ctx);

        let stop_time = Instant::now();
        debug!("TranscribeAction::stop called for binding: {}", binding_id);

        let ah = ctx.clone();
        let rm = Arc::clone(&ctx.audio);
        let tm = Arc::clone(&ctx.transcription);
        let hm = Arc::clone(&ctx.history);

        set_tray_state(ctx, TrayIconState::Transcribing);
        // Stop should give immediate visual feedback. Live streaming can keep
        // the larger panel, but it still switches from listening to a working
        // spinner while the stream finalizes. Non-streaming paths use the
        // compact transcribing pill (None no-ops in show_*).
        let style = get_settings(ctx).overlay_style;
        let mode = self.mode;
        let is_meeting = mode == TranscribeMode::Meeting;
        let post_process = mode == TranscribeMode::PostProcess || is_meeting;
        let use_streaming_overlay = should_use_streaming_overlay(style, tm.is_streaming());

        if use_streaming_overlay {
            tm.emit_stream_working(StreamWorkKind::Transcribing);
        } else if is_meeting {
            utils::show_meeting_transcribing_overlay(ctx);
        } else {
            show_transcribing_overlay(ctx);
        }

        // Unmute before playing audio feedback so the stop sound is audible
        rm.remove_mute();

        // Play audio feedback for recording stop
        play_feedback_sound(ctx, SoundType::Stop);

        let binding_id = binding_id.to_string(); // Clone binding_id for the async task
        let cancel_generation = rm.cancel_generation();

        crate::runtime::spawn(async move {
            let _guard = FinishGuard(ah.clone());
            debug!(
                "Starting async transcription task for binding: {}",
                binding_id
            );

            let stop_recording_time = Instant::now();
            if let Some(samples) = rm.stop_recording(&binding_id, cancel_generation) {
                debug!(
                    "Recording stopped and samples retrieved in {:?}, sample count: {}",
                    stop_recording_time.elapsed(),
                    samples.len()
                );

                if rm.was_cancelled_since(cancel_generation) {
                    debug!("Transcription operation cancelled after recording stop");
                    tm.cancel_stream();
                    utils::hide_recording_overlay(&ah);
                    set_tray_state(&ah, TrayIconState::Idle);
                    return;
                }

                if samples.is_empty() {
                    debug!("Recording produced no audio samples; skipping persistence");
                    // Tear down any streaming worker so its channel doesn't leak
                    // and block the next start_stream.
                    tm.cancel_stream();
                    utils::hide_recording_overlay(&ah);
                    set_tray_state(&ah, TrayIconState::Idle);
                } else {
                    let settings = get_settings(&ah);

                    // Apply voice enhancement DSP (software gain, 80Hz rumble filter, noise gate, AGC normalization)
                    let mut samples = samples;
                    let dsp_config = crate::audio_toolkit::audio::VoiceEnhancerConfig {
                        input_gain: settings.audio_input_gain,
                        high_pass_filter: settings.audio_high_pass_filter_enabled,
                        noise_reduction: settings.audio_noise_reduction_enabled,
                        noise_gate_threshold_db: settings.audio_noise_gate_threshold_db,
                        normalization: settings.audio_normalization_enabled,
                    };
                    crate::audio_toolkit::audio::VoiceEnhancer::process_with_config(
                        &mut samples,
                        &dsp_config,
                    );

                    // Save enhanced WAV concurrently with transcription
                    let sample_count = samples.len();
                    let file_name = format!("Otush-{}.wav", chrono::Utc::now().timestamp());
                    let wav_path = hm.recordings_dir().join(&file_name);
                    let wav_path_for_verify = wav_path.clone();
                    let samples_for_wav = samples.clone();
                    let wav_handle = crate::runtime::spawn_blocking(move || {
                        crate::audio_toolkit::save_wav_file(&wav_path, &samples_for_wav)
                    });

                    // Transcribe concurrently with WAV save. If a live stream was
                    // running, finalize it and use its text (all audio was already
                    // fed to the stream); otherwise batch-transcribe the samples.
                    let transcription_time = Instant::now();
                    let settings = get_settings(&ah);
                    let transcription_result = if settings.local_transcription_enabled {
                        match tm.finalize_stream() {
                            Ok(Some(text)) if !text.trim().is_empty() => Ok(text),
                            Ok(_) => match tm.transcribe(samples.clone()) {
                                Ok(text) => Ok(text),
                                Err(err) => {
                                    warn!("Local transcription error: {err}. Attempting cloud fallback...");
                                    crate::stt_client::transcribe_with_fallback(&settings, &samples)
                                        .await
                                }
                            },
                            Err(err) => {
                                warn!("Local streaming finalize error: {err}. Attempting cloud fallback...");
                                crate::stt_client::transcribe_with_fallback(&settings, &samples)
                                    .await
                            }
                        }
                    } else if settings.is_deepgram_streaming_active() {
                        match tm.finalize_stream() {
                            Ok(Some(text)) if !text.trim().is_empty() => Ok(text),
                            Ok(_) | Err(_) => {
                                warn!("Deepgram streaming finalize was empty or errored; falling back to batch...");
                                crate::stt_client::transcribe_with_fallback(&settings, &samples)
                                    .await
                            }
                        }
                    } else {
                        tm.cancel_stream();
                        crate::stt_client::transcribe_with_fallback(&settings, &samples).await
                    };

                    // Await WAV save and verify
                    let wav_saved = match wav_handle.await {
                        Ok(Ok(())) => {
                            match crate::audio_toolkit::verify_wav_file(
                                &wav_path_for_verify,
                                sample_count,
                            ) {
                                Ok(()) => true,
                                Err(e) => {
                                    error!("WAV verification failed: {}", e);
                                    false
                                }
                            }
                        }
                        Ok(Err(e)) => {
                            error!("Failed to save WAV file: {}", e);
                            false
                        }
                        Err(e) => {
                            error!("WAV save task panicked: {}", e);
                            false
                        }
                    };

                    if rm.was_cancelled_since(cancel_generation) {
                        debug!("Transcription operation cancelled before output handling");
                        utils::hide_recording_overlay(&ah);
                        set_tray_state(&ah, TrayIconState::Idle);
                        return;
                    }

                    match transcription_result {
                        Ok(transcription) => {
                            debug!(
                                "Transcription completed in {:?}: '{}'",
                                transcription_time.elapsed(),
                                utils::redact_text(&transcription)
                            );

                            if post_process {
                                if use_streaming_overlay {
                                    tm.emit_stream_working(StreamWorkKind::Polishing);
                                } else if is_meeting {
                                    utils::show_meeting_processing_overlay(&ah);
                                } else {
                                    show_processing_overlay(&ah);
                                }
                            }
                            let Some(processed) = complete_unless_cancelled(
                                process_transcription_output(
                                    &ah,
                                    &transcription,
                                    post_process,
                                    is_meeting,
                                ),
                                || rm.was_cancelled_since(cancel_generation),
                            )
                            .await
                            else {
                                debug!("Transcription operation cancelled during output handling");
                                utils::hide_recording_overlay(&ah);
                                set_tray_state(&ah, TrayIconState::Idle);
                                return;
                            };

                            if rm.was_cancelled_since(cancel_generation) {
                                debug!("Transcription operation cancelled before paste");
                                utils::hide_recording_overlay(&ah);
                                set_tray_state(&ah, TrayIconState::Idle);
                                return;
                            }

                            // Save to history if WAV was saved
                            if wav_saved {
                                if let Err(err) = hm.save_entry(
                                    file_name,
                                    transcription,
                                    post_process,
                                    processed.post_processed_text.clone(),
                                    processed.post_process_prompt.clone(),
                                ) {
                                    error!("Failed to save history entry: {}", err);
                                }
                            }

                            if processed.final_text.is_empty() {
                                utils::hide_recording_overlay(&ah);
                                set_tray_state(&ah, TrayIconState::Idle);
                            } else {
                                let paste_time = Instant::now();
                                let final_text = processed.final_text;
                                if rm.was_cancelled_since(cancel_generation) {
                                    debug!("Transcription operation cancelled before paste");
                                    utils::hide_recording_overlay(&ah);
                                    set_tray_state(&ah, TrayIconState::Idle);
                                } else {
                                    // Hide the overlay before injecting the
                                    // paste chord so the target app holds
                                    // keyboard focus (an overlay that stole
                                    // focus would swallow Ctrl+V).
                                    utils::hide_recording_overlay(&ah);
                                    match utils::paste(&ah, final_text) {
                                        Ok(()) => debug!(
                                            "Text pasted successfully in {:?}",
                                            paste_time.elapsed()
                                        ),
                                        Err(e) => {
                                            error!("Failed to paste transcription: {}", e);
                                            ah.bus.send(AppEvent::PasteError);
                                        }
                                    }
                                    utils::hide_recording_overlay(&ah);
                                    set_tray_state(&ah, TrayIconState::Idle);
                                }
                            }
                        }
                        Err(err) => {
                            if rm.was_cancelled_since(cancel_generation) {
                                debug!(
                                    "Transcription operation cancelled after transcription error"
                                );
                                utils::hide_recording_overlay(&ah);
                                set_tray_state(&ah, TrayIconState::Idle);
                                return;
                            }

                            error!("Transcription failed: {}", err);
                            // Surface the failure to the UI (toast). The full
                            // message is also in otush.log via the line above.
                            ah.bus.send(AppEvent::TranscriptionError(err));
                            // Save entry with empty text so user can retry
                            if wav_saved {
                                if let Err(save_err) = hm.save_entry(
                                    file_name,
                                    String::new(),
                                    post_process,
                                    None,
                                    None,
                                ) {
                                    error!("Failed to save failed history entry: {}", save_err);
                                }
                            }
                            utils::hide_recording_overlay(&ah);
                            set_tray_state(&ah, TrayIconState::Idle);
                        }
                    }
                }
            } else {
                debug!("No samples retrieved from recording stop");
                // Tear down any streaming worker so its channel doesn't leak.
                tm.cancel_stream();
                utils::hide_recording_overlay(&ah);
                set_tray_state(&ah, TrayIconState::Idle);
            }
        });

        debug!(
            "TranscribeAction::stop completed in {:?}",
            stop_time.elapsed()
        );
    }
}

// Cancel Action
struct CancelAction;

impl ShortcutAction for CancelAction {
    fn start(&self, ctx: &AppContext, _binding_id: &str, _shortcut_str: &str) {
        utils::cancel_current_operation(ctx);
    }

    fn stop(&self, _ctx: &AppContext, _binding_id: &str, _shortcut_str: &str) {
        // Nothing to do on stop for cancel
    }
}

// Test Action
struct TestAction;

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
        "cancel".to_string(),
        Arc::new(CancelAction) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "test".to_string(),
        Arc::new(TestAction) as Arc<dyn ShortcutAction>,
    );
    map
});

#[cfg(test)]
mod tests {
    use super::{
        complete_unless_cancelled, is_blank_transcription, should_use_streaming_overlay,
        strip_think_block,
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
        assert!(super::ACTION_MAP.contains_key("cancel"));
    }
}
