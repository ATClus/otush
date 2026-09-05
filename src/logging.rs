//! Logging setup replacing `tauri-plugin-log`: console output (RUST_LOG-aware),
//! a rotating single-file target under the log dir, and optional forwarding of
//! records to the UI (debug panel live log viewer) while debug mode is on.

use crate::context::{AppContext, AppEvent, EventBus};
use log::{Level, LevelFilter, Metadata, Record};
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Mutex;

/// File log level filter stored as `log::LevelFilter as u8`. Set from the
/// user's `log_level` setting; the file target filters against it.
pub static FILE_LOG_LEVEL: AtomicU8 = AtomicU8::new(LevelFilter::Debug as u8);

/// When `true`, log records are also forwarded to the UI via the event bus
/// (`AppEvent::LogRecord`) for the debug panel's live log viewer. Gated on
/// debug mode — the live log viewer is its only consumer and only exists in
/// debug mode — so normal runs never broadcast log records (which can include
/// file paths or transcribed text) onto the UI event bus.
pub static UI_LOG_STREAMING: AtomicBool = AtomicBool::new(false);

fn level_filter_from_u8(value: u8) -> LevelFilter {
    match value {
        0 => LevelFilter::Off,
        1 => LevelFilter::Error,
        2 => LevelFilter::Warn,
        3 => LevelFilter::Info,
        4 => LevelFilter::Debug,
        5 => LevelFilter::Trace,
        _ => LevelFilter::Trace,
    }
}

fn build_console_filter() -> env_filter::Filter {
    let mut builder = env_filter::Builder::new();

    match std::env::var("RUST_LOG") {
        Ok(spec) if !spec.trim().is_empty() => {
            if let Err(err) = builder.try_parse(&spec) {
                log::warn!(
                    "Ignoring invalid RUST_LOG value '{}': {}. Falling back to info-level console logging",
                    spec,
                    err
                );
                builder.filter_level(LevelFilter::Info);
            }
        }
        _ => {
            builder.filter_level(LevelFilter::Info);
        }
    }

    builder.build()
}

/// Global logger: console (stderr, RUST_LOG-filtered) + file + optional UI streaming.
struct OtushLogger {
    console_filter: env_filter::Filter,
    file: Mutex<Option<File>>,
    bus: Option<EventBus>,
}

impl log::Log for OtushLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        let ui_streaming = UI_LOG_STREAMING.load(Ordering::Relaxed)
            && metadata.level() <= level_filter_from_u8(FILE_LOG_LEVEL.load(Ordering::Relaxed));
        self.console_filter.enabled(metadata) || ui_streaming
    }

    fn log(&self, record: &Record) {
        let file_level = level_filter_from_u8(FILE_LOG_LEVEL.load(Ordering::Relaxed));

        // Console (always stderr so headless stdout stays clean for CI parsing).
        if self.console_filter.enabled(record.metadata()) {
            eprintln!("[{} {}] {}", record.level(), record.target(), record.args());
        }

        // File. Recover from a poisoned file mutex instead of panicking: a
        // logging failure must never crash transcription or recording.
        if record.level() <= file_level {
            if let Some(file) = self.file.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
                // Note: `chrono::Local::now()` runs per record. This logger is
                // called at most a few hundred times per second outside the
                // audio callback (which must never log per frame), so a
                // background writer thread is not warranted yet — revisit if
                // profiling shows `log` in a hot path.
                let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
                let _ = writeln!(
                    file,
                    "{ts} [{:<5}] {}: {}",
                    record.level(),
                    record.target(),
                    record.args()
                );
            }
        }

        // UI streaming (debug live log viewer).
        if UI_LOG_STREAMING.load(Ordering::Relaxed) && record.level() <= file_level {
            if let Some(bus) = &self.bus {
                bus.send(AppEvent::LogRecord(format!(
                    "[{:<5}] {}",
                    record.level(),
                    record.args()
                )));
            }
        }
    }

    fn flush(&self) {
        if let Some(file) = self.file.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            let _ = file.flush();
        }
    }
}

/// Install the global logger. `bus` enables forwarding records to the UI;
/// pass the app bus (headless mode can pass a throwaway bus).
pub fn init_logging(log_dir: &Path, bus: EventBus) {
    let console_filter = build_console_filter();
    let _ = std::fs::create_dir_all(log_dir);
    let file = File::create(log_dir.join("otush.log")).ok();
    let logger = OtushLogger {
        console_filter,
        file: Mutex::new(file),
        bus: Some(bus),
    };
    let _ = log::set_boxed_logger(Box::new(logger));
    log::set_max_level(LevelFilter::Trace);
}

/// Apply the persisted `log_level` setting to the file log filter.
pub fn apply_log_level(ctx: &AppContext) {
    let settings = ctx.settings();
    let level: Level = match settings.log_level {
        crate::settings::LogLevel::Trace => Level::Trace,
        crate::settings::LogLevel::Debug => Level::Debug,
        crate::settings::LogLevel::Info => Level::Info,
        crate::settings::LogLevel::Warn => Level::Warn,
        crate::settings::LogLevel::Error => Level::Error,
    };
    FILE_LOG_LEVEL.store(level.to_level_filter() as u8, Ordering::Relaxed);
    // Only forward logs to the UI while debug mode is on.
    UI_LOG_STREAMING.store(settings.debug_mode, Ordering::Relaxed);
}
