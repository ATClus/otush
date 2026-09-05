//! Global Tokio runtime.
//!
//! The GTK main loop runs on the glib executor; backend async work (reqwest,
//! hf-hub downloads, zbus/ashpd portal calls, blocking CPU tasks) requires a
//! Tokio reactor. A single multi-thread runtime is created at startup and
//! every async task is spawned onto it from any thread; results are marshaled
//! back to the UI with `glib::MainContext::invoke`.

use std::sync::OnceLock;

static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// Create the global runtime. Call once from `lib.rs::run` before any async
/// work.
///
/// The pool is intentionally small: backend work is a mix of short network
/// polls and a few long downloads, while heavy CPU (transcription) and
/// blocking I/O (SQLite, WAV) run on the same pool via `spawn_blocking`.
/// Keep main-thread-adjacent latency low by never `block_on` from GTK code.
pub fn init() {
    let _ = RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .thread_name("otush-worker")
            .worker_threads(4)
            .max_blocking_threads(8)
            .enable_all()
            .build()
            .expect("failed to create the Tokio runtime")
    });
}

/// Spawn a future onto the global runtime from any thread.
pub fn spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    RUNTIME
        .get()
        .expect("tokio runtime not initialized (call runtime::init first)")
        .spawn(future)
}

/// Spawn a blocking closure onto the runtime's blocking pool.
pub fn spawn_blocking<F, R>(f: F) -> tokio::task::JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    RUNTIME
        .get()
        .expect("tokio runtime not initialized (call runtime::init first)")
        .spawn_blocking(f)
}

/// Run a future to completion on the runtime, blocking the current thread.
///
/// # Panics / deadlocks
///
/// Never call this from the GTK main thread or from inside a future already
/// running on this runtime (it would block the reactor or deadlock). The only
/// legitimate caller is a dedicated worker thread that must bridge sync and
/// async code — today, the transcription stream worker driving the Deepgram
/// websocket pump.
pub fn block_on<F: std::future::Future>(future: F) -> F::Output {
    RUNTIME
        .get()
        .expect("tokio runtime not initialized (call runtime::init first)")
        .block_on(future)
}
