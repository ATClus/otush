//! Otush core: a native GNOME (GTK4/libadwaita) speech-to-text application.
//!
//! The Rust core (managers, audio pipeline, transcription, downloads, history)
//! is GUI-agnostic. It talks to the UI through [`context::AppContext`] and the
//! [`context::EventBus`]; the GTK shell (Phase 2+) subscribes to [`AppEvent`]s.

mod actions;
pub mod agents;
mod app;
mod audio_feedback;
pub mod audio_toolkit;
mod autostart;
pub mod cli;
mod clipboard;
mod commands;
mod context;
mod input;
mod llm_client;
mod logging;
mod managers;
mod memory;
mod overlay;
pub mod portable;
mod resources;
mod runtime;
mod settings;
mod shortcut;
mod signal_handle;
pub mod stt_client;
pub mod template;
mod transcription_coordinator;
mod tray;
mod tray_i18n;
mod ui;
mod updater;
mod utils;
pub mod web_client;

pub use cli::CliArgs;
pub use context::{AppContext, AppEvent, AppPaths, EventBus, Subscription};
pub use transcription_coordinator::TranscriptionCoordinator;

pub use logging::{FILE_LOG_LEVEL, UI_LOG_STREAMING};

use crate::managers::audio::AudioRecordingManager;
use crate::managers::history::HistoryManager;
use crate::managers::model::ModelManager;
use crate::managers::transcription::TranscriptionManager;
use std::sync::Arc;

/// Build the full application context: paths, event bus, and every manager.
pub fn build_app_context() -> anyhow::Result<AppContext> {
    let paths = AppPaths::resolve();
    paths.ensure_dirs()?;

    let bus = EventBus::new();

    // Initialize the managers. The audio recorder receives the streaming router
    // explicitly, so always-on microphone startup can wire live-preview frames
    // even before the context is assembled.
    let model_manager = Arc::new(ModelManager::new(&paths, bus.clone())?);
    let transcription_manager = Arc::new(TranscriptionManager::new(
        &paths,
        bus.clone(),
        model_manager.clone(),
    )?);
    let recording_manager = Arc::new(AudioRecordingManager::new(
        &paths,
        bus.clone(),
        transcription_manager.stream_router(),
    )?);
    let history_manager = Arc::new(HistoryManager::new(&paths, bus.clone())?);
    let coordinator = Arc::new(TranscriptionCoordinator::new());

    let ctx = AppContext {
        paths,
        bus,
        model: model_manager,
        transcription: transcription_manager,
        audio: recording_manager,
        history: history_manager,
        coordinator,
    };

    // Late-bound wiring: the transcription idle watcher needs the audio
    // manager (weak ref), and the coordinator thread needs the full context.
    ctx.transcription.set_audio_manager(&ctx.audio);
    ctx.coordinator.bind_context(&ctx);

    Ok(ctx)
}

/// Core initialization shared by every entry path (GUI + headless CLI).
/// Note: shortcuts and enigo are NOT initialized here — the UI drives that
/// after onboarding completes (matches the pre-port behavior).
pub fn init_core(ctx: &AppContext, cli_args: &CliArgs) {
    // Initialize the transcribe-cpp native backend (logging + backend module
    // registration) once, before any whisper model is loaded.
    managers::transcription::init_transcribe_backend();

    // Apply accelerator preferences before any model loads.
    managers::transcription::apply_accelerator_settings(ctx);

    // Apply the persisted log level / debug streaming flag.
    logging::apply_log_level(ctx);
    if cli_args.debug {
        // CLI --debug overrides debug_mode and log level (runtime-only, not
        // persisted).
        UI_LOG_STREAMING.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    // Set up signal handlers for toggling transcription. On Linux, SIGUSR1 is
    // deliberately not handled — it belongs to WebKitGTK's garbage collector
    // (#1660) — see signal_handle.rs.
    #[cfg(unix)]
    signal_handle::setup_signal_handler(ctx);

    // Populate the overlay-enabled cache from initial settings so the audio
    // path (overlay::emit_levels, called ~24 Hz during recording) can do a
    // single atomic load instead of reading the settings file each frame.
    overlay::update_overlay_enabled_cache(
        ctx.settings().overlay_style != settings::OverlayStyle::None,
    );
}

/// Convert an unexpected panic on the headless worker into a normal CLI
/// failure.
fn run_headless_guarded<F>(operation: F) -> i32
where
    F: FnOnce() -> i32,
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)) {
        Ok(code) => code,
        Err(payload) => {
            let message = if let Some(message) = payload.downcast_ref::<&str>() {
                (*message).to_string()
            } else if let Some(message) = payload.downcast_ref::<String>() {
                message.clone()
            } else {
                "unknown panic".to_string()
            };
            eprintln!("error: headless transcription panicked: {message}");
            1
        }
    }
}

/// Headless one-shot transcription for the `--transcribe-file` /
/// `--list-devices` / `--list-models` path. Drives the same
/// `TranscriptionManager::transcribe` the app uses; no mic, no VAD, no
/// download. Returns a process exit code (0 ok, 1 runtime failure, 2 bad
/// input/usage).
fn run_headless_transcription(ctx: &AppContext, args: &CliArgs) -> i32 {
    use std::time::Instant;

    // --list-devices: print registered compute devices (with indices) and exit.
    if args.list_devices {
        let devices = crate::managers::transcription::describe_compute_devices();
        if devices.is_empty() {
            println!("No transcribe-cpp compute devices registered.");
        } else {
            println!("transcribe-cpp compute devices:");
            for d in &devices {
                println!("  {}", d);
            }
        }
        if args.transcribe_file.is_none() {
            return 0;
        }
    }

    // --list-models: print the model registry (catalog + on-disk + custom) with
    // their ids — the same ids `--model` accepts — then exit. `--json` emits the
    // full ModelInfo array for scripting.
    if args.list_models {
        let models = ctx.model.get_available_models();
        if args.json {
            match serde_json::to_string_pretty(&models) {
                Ok(s) => println!("{}", s),
                Err(e) => {
                    eprintln!("error: failed to serialize models: {}", e);
                    return 1;
                }
            }
        } else if models.is_empty() {
            println!("No models available.");
        } else {
            println!("Available models (✓ = installed):");
            let width = models.iter().map(|m| m.id.len()).max().unwrap_or(0);
            for m in &models {
                let mark = if m.is_downloaded { "✓" } else { " " };
                let rec = if m.is_recommended {
                    "  [recommended]"
                } else {
                    ""
                };
                println!(
                    "  {}  {:<width$}  {}{}",
                    mark,
                    m.id,
                    m.name,
                    rec,
                    width = width
                );
            }
        }
        if args.transcribe_file.is_none() {
            return 0;
        }
    }

    let Some(wav) = args.transcribe_file.clone() else {
        return 0;
    };

    let decoded = match crate::audio_toolkit::decode_media_file(&wav, None) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: failed to read and decode {}: {}", wav.display(), e);
            return 2;
        }
    };
    let samples = decoded.samples;
    let audio_secs = decoded.duration_secs;

    let tm = &ctx.transcription;

    let model_id = args
        .model
        .clone()
        .unwrap_or_else(|| settings::get_settings(ctx).selected_model);
    if model_id.is_empty() {
        eprintln!("error: no model selected (pass --model or pick one in the app)");
        return 2;
    }

    let device_index = args.device_index;

    let load_start = Instant::now();
    if let Err(e) = tm.load_model_with_device(&model_id, device_index) {
        eprintln!("error: load_model('{}') failed: {}", model_id, e);
        return 1;
    }
    let load_ms = load_start.elapsed().as_millis() as u64;
    let bound_backend = tm.current_backend();

    let runs = args.repeat.unwrap_or(1).max(1);
    let mut times_ms: Vec<u64> = Vec::new();
    let mut text = String::new();
    for i in 0..runs {
        if !tm.is_model_loaded() {
            if let Err(e) = tm.load_model_with_device(&model_id, device_index) {
                eprintln!("error: reload before run {} failed: {}", i + 1, e);
                return 1;
            }
        }
        let t = Instant::now();
        match tm.transcribe(samples.clone()) {
            Ok(out) => text = out,
            Err(e) => {
                eprintln!("error: transcribe failed: {}", e);
                return 1;
            }
        }
        times_ms.push(t.elapsed().as_millis() as u64);
    }
    let best_ms = times_ms.iter().copied().min().unwrap_or(0);
    let rtf = if best_ms > 0 {
        audio_secs / (best_ms as f64 / 1000.0)
    } else {
        0.0
    };

    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "model": model_id,
                "requested_device": device_index.map(|i| format!("index {i}")).unwrap_or_else(|| "settings".to_string()),
                "bound_backend": bound_backend,
                "audio_secs": audio_secs,
                "load_ms": load_ms,
                "transcribe_ms": times_ms,
                "best_ms": best_ms,
                "rtf": rtf,
                "text": text,
            })
        );
    } else {
        println!(
            "model={} device={} backend={} audio={:.2}s load={}ms best={}ms rtf={:.2}x",
            model_id,
            device_index
                .map(|i| format!("index {i}"))
                .unwrap_or_else(|| "settings".to_string()),
            bound_backend.as_deref().unwrap_or("?"),
            audio_secs,
            load_ms,
            best_ms,
            rtf,
        );
        println!("text: {}", text);
    }
    0
}

#[cfg(target_os = "linux")]
unsafe extern "C" fn noop_alsa_error_handler(
    _file: *const std::os::raw::c_char,
    _line: std::os::raw::c_int,
    _func: *const std::os::raw::c_char,
    _err: std::os::raw::c_int,
    _fmt: *const std::os::raw::c_char,
) {
    // Silence ALSA library stderr error spew (e.g. "Unknown PCM pulse/jack/oss")
}

#[cfg(target_os = "linux")]
pub fn silence_alsa_logging() {
    unsafe {
        type SndLibErrorHandler = unsafe extern "C" fn(
            *const std::os::raw::c_char,
            std::os::raw::c_int,
            *const std::os::raw::c_char,
            std::os::raw::c_int,
            *const std::os::raw::c_char,
        );
        extern "C" {
            fn snd_lib_error_set_handler(handler: SndLibErrorHandler) -> std::os::raw::c_int;
        }
        snd_lib_error_set_handler(noop_alsa_error_handler);
    }
}

#[cfg(not(target_os = "linux"))]
pub fn silence_alsa_logging() {}

/// Application entry point. Headless one-shot flags run without any GUI; the
/// GUI shell (GTK4/libadwaita) is initialized by `app.rs` once it lands.
pub fn run(cli_args: CliArgs) {
    silence_alsa_logging();

    // Pin glibc's dynamic mmap threshold before the first large allocation,
    // so per-dictation transient buffers are returned to the OS on free
    // instead of accumulating in malloc arenas (#1792).
    memory::init_allocator();

    // Detect portable mode before anything else.
    portable::init();

    // Logging (console + file) with a throwaway bus; managers share the real
    // bus from the context.
    let paths = AppPaths::resolve();
    let _ = paths.ensure_dirs();
    logging::init_logging(&paths.log_dir, EventBus::new());

    // Global Tokio runtime for backend async work (downloads, portal, reqwest).
    runtime::init();

    // Prefer the native Wayland backend whenever a Wayland display is present:
    // the recording overlay depends on wlr-layer-shell, which XWayland lacks.
    if std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("GDK_BACKEND").is_none() {
        std::env::set_var("GDK_BACKEND", "wayland");
    }

    let headless_mode =
        cli_args.transcribe_file.is_some() || cli_args.list_devices || cli_args.list_models;

    let ctx = match build_app_context() {
        Ok(ctx) => ctx,
        Err(e) => {
            log::error!("Failed to initialize core: {e}");
            std::process::exit(1);
        }
    };

    // Headless one-shot path: initialize what transcription needs and run on a
    // worker, then exit with the result code.
    if headless_mode {
        init_core(&ctx, &cli_args);
        let code = run_headless_guarded(|| run_headless_transcription(&ctx, &cli_args));
        // Drop the loaded engine before teardown.
        let _ = ctx.transcription.unload_model();
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let _ = std::io::stderr().flush();
        std::process::exit(code);
    }

    init_core(&ctx, &cli_args);

    // Set GLib application identity and ensure desktop entry is registered for XDG Desktop Portal
    glib::set_prgname(Some("com.clusterat.otush"));
    autostart::ensure_desktop_entry_registered();

    // Ensure autostart entry agrees with the in-app setting (recreates it
    // when a .deb upgrade or an external toggle removed it behind our back).
    autostart::ensure_autostart_consistency(&ctx);

    // Non-headless: run the native GNOME shell (GTK4/libadwaita).
    crate::app::run_gtk(ctx, &cli_args);
}
