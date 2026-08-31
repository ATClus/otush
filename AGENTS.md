# AGENTS.md

This file provides guidance to AI coding assistants working with code in this repository.

## Development Commands

**Prerequisites:**

- [Rust](https://rustup.rs/) (latest stable)
- Ubuntu 24.04+ native libraries (see [BUILD.md](BUILD.md)):
  `libgtk-4-dev libadwaita-1-dev libasound2-dev libssl-dev libvulkan-dev libopenblas-dev`

**Core Development:**

```bash
# Build (debug / release)
cargo build
cargo build --release

# Run the GNOME app
cargo run

# Run headless one-shot CLI (no GUI)
cargo run -- --list-models
cargo run -- --transcribe-file path/to/16khz-mono-16bit.wav --model small
```

**Linting and Formatting (run before committing):**

```bash
cargo fmt
cargo clippy -- -D warnings
cargo test
```

**Model Setup (Required for Development):**

```bash
mkdir -p resources/models
curl -o resources/models/silero_vad_v4.onnx https://blob.handy.computer/silero_vad_v4.onnx
```

For detailed platform-specific build setup, see [BUILD.md](BUILD.md).

## Architecture Overview

Otush is a **native GNOME application** (GTK4 + libadwaita on Wayland) with a Rust backend kept in the same process. There is no webview, no Node toolchain, and no XWayland.

### Structure (src/)

- `lib.rs` — crate entry: `run()` builds the `AppContext` (paths, managers, event bus), initializes the core, and starts the GTK shell (`app::run_gtk`) or the headless CLI path.
- `context.rs` — the host abstraction: `AppContext` (paths + managers + `EventBus`), `AppPaths` (portable-aware dirs), `EventBus` (backend → UI fan-out) and `AppEvent` (the single event enum).
- `app.rs` — GTK shell: `adw::Application`, single-instance + remote CLI via GApplication's `command-line` signal, event-bus → toast/theme wiring.
- `ui/` — GTK4 widgets: `window.rs` (navigation shell), `pages/` (General, Models, Post-Processing, History, Advanced, Debug, About), all `AdwPreferencesPage`s.
- `overlay.rs` — recording overlay: a fullscreen transparent, click-through surface (GNOME/Mutter lacks the wlr-layer-shell protocol) with a Cairo-painted pill at the bottom/top screen edge (state, pulsing record button, mic waveform, streaming text).
- `tray.rs` — StatusNotifierItem via `ksni`.
- `managers/` — core business logic:
  - `audio.rs` — audio recording and device management (cpal)
  - `model.rs` — model catalog, downloads, extraction
  - `transcription.rs` — speech-to-text pipeline (transcribe-cpp / transcribe-rs)
  - `history.rs` — transcription history (rusqlite)
- `audio_toolkit/` — low-level audio processing (devices, recording, resampling, VAD).
- `commands/` — the command layer (plain functions on `&AppContext`) that the UI calls.
- `shortcut/` — shortcut engines behind one interface: `portal.rs` (XDG GlobalShortcuts portal via `ashpd`, Wayland-native) and `evdev.rs` (evdev via the handy-keys crate); `handler.rs` dispatches events.
- `settings.rs` — settings model + JSON store (`settings_store.json`).
- `clipboard.rs`, `paste_tx/` — clipboard + paste (wtype/ydotool/xdotool/enigo, GNOME-Wayland aware).
- `logging.rs` — console + file logging with UI streaming in debug mode.
- `autostart.rs` — XDG autostart entry; `updater.rs` — GitHub releases check.

### Key Architecture Patterns

**AppContext pattern:** managers are constructed with `AppPaths` + `EventBus` (no GUI coupling) and assembled into one `AppContext` (`build_app_context`). The UI and CLI both use it; the coordinator thread late-binds it via `OnceLock`.

**Command–Event architecture:** UI → `commands::*` functions on `&AppContext`; backend → UI via `EventBus::send(AppEvent::…)`, marshaled onto the GTK main loop with `glib::MainContext::invoke`.

**Pipeline:** Audio → VAD → Whisper/Parakeet → text → clipboard/paste.

### Technology Stack

**Core libraries:** transcribe-cpp (GGML/GGUF), transcribe-rs (ONNX), cpal, vad-rs, evdev, rubato, rodio, rusqlite, reqwest, tokio.

**UI:** gtk4, libadwaita (version features `v1_3`/`v1_4` for Ubuntu 24.04 compatibility), glib, ashpd (GlobalShortcuts portal), ksni (tray), arboard (clipboard), image (icon decoding).

### Application Flow

1. `main.rs` parses CLI args → `otush::run`.
2. `run()` initializes portable mode, logging, `AppContext` (managers), core (transcribe backend, log level, signal handlers).
3. Non-headless: `app::run_gtk` creates the `adw::Application`; on `activate` the main window, overlay and tray are created and shortcuts are initialized.
4. A shortcut (portal or evdev) → coordinator → action → recording → transcription → paste.
5. Remote control (`--toggle-transcription` etc.) arrives via GApplication `command-line` and acts on the running instance.

### Single Instance Architecture

`GApplication` (adw::Application) provides single-instance behavior natively: a second launch forwards its command line to the primary's `command-line` handler, which maps remote-control flags (`--toggle-transcription`, `--toggle-post-process`, `--cancel`) to the coordinator and otherwise raises the window. The headless CLI (`--transcribe-file` / `--list-devices` / `--list-models`) runs before the GTK shell starts and exits with a result code.

## Internationalization (i18n)

User-facing strings are currently English-only (tray strings live in
`tray_i18n.rs`); gettext-based localization is planned. When adding user-facing
text, keep it in one place (a strings module or `.po` file) rather than
scattering literals.

## Code Style

- Run `cargo fmt` and `cargo clippy -- -D warnings` before committing
- Handle errors explicitly (avoid unwrap in production)
- Use descriptive names, add doc comments for public APIs
- GTK widgets: follow the `ui/pages/` pattern (commands on `&AppContext`,
  bus events marshaled via `glib::MainContext::invoke`); never touch widgets
  off the main thread

## CLI Parameters

Otush supports command-line parameters for integration with scripts, window managers, and autostart configurations.

**Implementation:** `cli.rs` (definitions), `main.rs` (parsing), `lib.rs` (applying), `signal_handle.rs` (shared logic)

| Flag                     | Description                                                |
| ------------------------ | ---------------------------------------------------------- |
| `--toggle-transcription` | Toggle recording on/off on a running instance              |
| `--toggle-post-process`  | Toggle recording with post-processing on/off               |
| `--cancel`               | Cancel the current operation on a running instance         |
| `--start-hidden`         | Launch without showing the main window (tray icon visible) |
| `--no-tray`              | Launch without system tray (closing window quits the app)  |
| `--debug`                | Enable debug mode with verbose (Trace) logging             |

**Key design decisions:**

- CLI flags are runtime-only overrides — they do NOT modify persisted settings
- Remote control flags arrive via GApplication's `command-line` signal: a second instance forwards its args to the primary, which acts and exits (see `app.rs`)
- `send_transcription_input()` in `signal_handle.rs` is shared between signal handlers and CLI

## Debug Mode

Enable via Settings → Debug (the `debug_mode` setting) or the `--debug` CLI
flag. Debug mode enables the verbose file log level and the live log viewer
(in `ui/pages/debug.rs`).

## Platform Notes

- **Linux (GNOME/Wayland)** is the target platform. OpenBLAS + Vulkan
  acceleration; the recording overlay is a fullscreen transparent surface (input region makes it click-through); global shortcuts
  use the XDG GlobalShortcuts portal (ashpd) with an evdev fallback.
- macOS/Windows support was removed in the native conversion; the Rust core
  remains portable.

## Troubleshooting

See the [Troubleshooting](README.md#troubleshooting) section in README.md.

## Git workflow

**Commits:** Use conventional commit prefixes (`feat:`, `fix:`, `docs:`,
`refactor:`, `chore:`). Focus the message on _why_, not _what_.

**Branding:** This project is Otush (`com.clusterat.otush`). Do not reintroduce
the upstream "Handy" name in user-facing strings, packaging or identifiers —
except where it is a functional reference (the `handy-computer` Hugging Face
org / `blob.handy.computer` model hosting). The upstream Handy project is credited
in [README.md](README.md).
