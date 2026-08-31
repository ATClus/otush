# Otush

**Speech-to-text for GNOME. Free, open source and fully offline.**

Press a shortcut, speak, and your words appear in whatever text field is
focused — everything happens on your own computer, nothing is sent to the
cloud.

Otush is a native GNOME application: GTK4 + libadwaita on Wayland, with a Rust
backend in the same process. No webview, no Electron, no XWayland.

## Features

- **Global shortcuts** — start/stop transcription from anywhere, via the XDG
  GlobalShortcuts portal (Wayland-native) with an evdev fallback engine.
- **Fully offline** — Whisper-family models run locally through
  transcribe-cpp (GGML/GGUF, Vulkan/OpenBLAS backends); Parakeet, Moonshine,
  SenseVoice, GigaAM, Canary and Cohere through transcribe-rs (ONNX).
- **Recording overlay** — a Wayland layer-shell pill showing state and live
  mic level, with an optional live-streaming transcript mode.
- **Push-to-talk and always-on modes**, selectable language, VAD-based
  segmentation with adjustable sensitivity.
- **Post-processing** — fix punctuation, capitalization and filler words
  locally, or with an LLM provider (Anthropic/OpenAI-compatible APIs).
- **History** — every transcription is stored locally (SQLite) and searchable.
- **System tray** (StatusNotifierItem) — model switching, copy last
  transcript, quit.
- **Follows your system theme** — light/dark and accent colors come from
  GNOME via libadwaita; the icon set ships light, dark and colored variants.

## Requirements

- Ubuntu 24.04 LTS or newer (GNOME 46+, Wayland recommended)
- For building, the native libraries:
  `libgtk-4-dev libadwaita-1-dev libasound2-dev
  libssl-dev libvulkan-dev glslc spirv-headers glslang-tools libopenblas-dev`

See [BUILD.md](BUILD.md) for the full build guide and [QA.md](QA.md) for the
manual QA checklist on Ubuntu 24.04 / GNOME 46 / Wayland.

## Building and running

```bash
cargo build
cargo run
```

Headless one-shot CLI (no GUI):

```bash
cargo run -- --list-models
cargo run -- --transcribe-file path/to/16khz-mono-16bit.wav --model small
```

### Model setup

The Silero VAD model is required for voice-activity detection:

```bash
mkdir -p resources/models
curl -o resources/models/silero_vad_v4.onnx https://blob.handy.computer/silero_vad_v4.onnx
```

Speech models are downloaded from within the app (Settings → Models) — or
dropped manually into `~/.local/share/com.clusterat.otush/models`.

## Command-line flags

| Flag | Description |
| ---- | ----------- |
| `--toggle-transcription` | Toggle recording on/off (sent to a running instance) |
| `--toggle-post-process`  | Toggle recording with post-processing on/off |
| `--cancel`               | Cancel the current operation |
| `--start-hidden`         | Launch without showing the main window (tray icon visible) |
| `--no-tray`              | Launch without the system tray (closing the window quits) |
| `--debug`                | Enable debug mode with verbose (Trace) logging |
| `--transcribe-file WAV`  | Transcribe a 16 kHz mono WAV headlessly and exit |
| `--list-devices`         | List the transcribe-cpp compute devices |
| `--list-models`          | List the available models |

Remote-control flags reach the primary instance through GApplication's
single-instance mechanism, so they work from scripts, window managers and
custom keybindings:

```bash
otush --toggle-transcription   # start/stop recording
otush --toggle-post-process    # record + post-process
otush --cancel                 # cancel the current operation
```

## Data locations

| What | Where |
| ---- | ----- |
| Settings | `~/.local/share/com.clusterat.otush/settings_store.json` |
| Models | `~/.local/share/com.clusterat.otush/models` |
| History | `~/.local/share/com.clusterat.otush/history.db` |
| Recordings | `~/.local/share/com.clusterat.otush/recordings` |
| Logs | `~/.local/share/com.clusterat.otush/logs/otush.log` |

## Shortcut engines

Settings → General → Shortcuts offers two engines:

- **XDG GlobalShortcuts portal** (default) — Wayland-native, via the
  freedesktop portal (ashpd). This is what packaged installs use.
- **evdev engine** — direct keyboard capture through the handy-keys library
  (fallback when the portal is unavailable, e.g. running from `cargo run`
  without a desktop entry).

## Troubleshooting

- **Overlay doesn't show**: the recording overlay is a transparent, click-through
  surface. Make sure the session is Wayland (`GDK_BACKEND=wayland` is set).
- **Overlay steals focus / paste fails**: keep `Overlay → Style` at
  `None`/`Minimal` on compositors that treat layer surfaces as the active
  window; the overlay is hidden before pasting, which avoids focus loss.
- **Global shortcuts don't register under `cargo run`**: without an installed
  `.desktop` file the portal reports "An app id is required"; Otush then
  falls back to the evdev engine automatically.

## Packaging

- **deb**: `cargo install cargo-deb && cargo deb` (see
  `packaging/debian/Cargo.toml`).
- **Flatpak**: `packaging/flatpak/com.clusterat.otush.yml` (GNOME 46 runtime,
  portal permissions included).
- Desktop entry: `packaging/com.clusterat.otush.desktop`.

## Credits

Otush is a fork of [Handy](https://github.com/cjpais/Handy) — a free, open
source, extensible speech-to-text application. The Rust core (audio pipeline,
transcription, history, downloads) is derived from Handy; the UI was rebuilt
natively for GNOME. The original Handy project, its model hosting
(`blob.handy.computer`) and the `handy-computer` Hugging Face org remain the
upstream for model files. Both projects are MIT licensed.
