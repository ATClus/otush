# Building Otush (native GNOME)

Otush is a **native GNOME application**: GTK4 + libadwaita UI running as a
Wayland client, with the Rust backend (audio capture/VAD, Whisper/transcribe-rs
transcription, model management, history, clipboard/paste) kept in the same
process.

## Prerequisites (Ubuntu 24.04+)

```bash
sudo apt update
sudo apt install -y build-essential pkg-config \
  libgtk-4-dev libadwaita-1-dev libgraphene-1.0-dev \
  libasound2-dev libssl-dev libvulkan-dev vulkan-tools glslc spirv-headers glslang-tools \
  libopenblas-dev librsvg2-dev
```

## Build

```bash
cargo build            # debug
cargo build --release  # release (LTO; slower first build)
```

## Run

```bash
cargo run                        # GUI
cargo run -- --start-hidden      # start minimized to tray
cargo run -- --no-tray           # no StatusNotifierItem
cargo run -- --toggle-transcription   # remote-control an already-running instance
```

Headless one-shot CLI (no GUI, no microphone):

```bash
cargo run -- --list-models
cargo run -- --list-devices
cargo run -- --transcribe-file path/to/16khz-mono-16bit.wav --model small
```

## Data locations

- User data (settings, models, history, recordings, logs):
  `~/.local/share/com.clusterat.otush/`
- Settings file: `~/.local/share/com.clusterat.otush/settings_store.json`
- Logs: `~/.local/share/com.clusterat.otush/logs/otush.log`

## Model setup (required once)

```bash
mkdir -p resources/models
curl -o resources/models/silero_vad_v4.onnx https://blob.handy.computer/silero_vad_v4.onnx
```

Models are downloaded from within the app (Models page) into
`~/.local/share/com.clusterat.otush/models`.

## GNOME-specific notes

- **Global shortcuts**: on Wayland, GNOME does not allow raw global grabs.
  Otush ships two engines (Settings → General → Shortcuts): the XDG
  GlobalShortcuts portal (preferred; requires recent GNOME) and the
  handy-keys evdev engine. In-app push-to-talk always works.
- **Tray icon**: uses the StatusNotifierItem spec; GNOME requires the
  "AppIndicator and KStatusNotifierItem Support" extension (shipped by
  default on Ubuntu's GNOME session).
- **Overlay**: a fullscreen transparent surface (GNOME/Mutter does not
  implement the wlr-layer-shell protocol) with a compact pill painted at the
  bottom-center of the screen and an empty input region so the rest of the
  screen stays click-through.

## Lint / format / test

```bash
cargo fmt && cargo clippy -- -D warnings && cargo test
```

## Packaging

- **deb**: `cargo install cargo-deb && cargo deb` (see
  `packaging/debian/Cargo.toml`), `depends: libgtk-4-1, libadwaita-1-0,
  libopenblas0`.
- **Flatpak** (recommended for GNOME Software integration): see
  `packaging/flatpak/com.clusterat.otush.yml`.
- Bundled resources (`resources/`) must ship next to the binary (or under
  `/usr/share/otush/resources`).
- Desktop entry: `packaging/com.clusterat.otush.desktop`.
- App icons: `resources/otush.png` (512), `resources/otush-128.png` and the
  scalable `resources/otush-icon.svg`; regenerated with
  `python3 scripts/gen_icons.py`.
