<div align="center">
  <img src="resources/otush-128.png" width="96" height="96" alt="Otush Logo" />
  <h1>Otush</h1>
  <p><strong>The Native AI Voice & Productivity Suite for GNOME</strong></p>
  <p>
    <em>Offline Speech-to-Text • Meeting Intelligence • Context AI Actions • Quick Notes • Task Checklist • Document OCR • Web Deep Research</em>
  </p>
  <p>
    <img src="https://img.shields.io/badge/Platform-Linux%20%2F%20GNOME%2046%2B-blue?logo=linux" alt="Linux Platform" />
    <img src="https://img.shields.io/badge/Wayland-Native-success" alt="Wayland Native" />
    <img src="https://img.shields.io/badge/UI-GTK4%20%2F%20Libadwaita-indigo?logo=gnome" alt="GTK4 Libadwaita" />
    <img src="https://img.shields.io/badge/Language-Rust-orange?logo=rust" alt="Rust" />
    <img src="https://img.shields.io/badge/Privacy-100%25%20Offline%20Capable-brightgreen" alt="Offline Capable" />
    <img src="https://img.shields.io/badge/License-MIT-green.svg" alt="License" />
  </p>
</div>

---

**Otush** is a native, privacy-first desktop application designed specifically for Linux (GNOME / Wayland). Originally conceived as an offline speech-to-text dictation utility, Otush has expanded into a full-featured **AI Voice & Productivity Suite**.

Whether you want to dictate text directly into any window, record long meetings and generate structured minutes, transform selected text across desktop apps with AI prompt templates, capture fleeting thoughts in notes, manage checklists, extract structured Markdown from documents and images with vision OCR, or conduct deep multi-source web research — Otush gives you unified floating palettes and global hotkeys that never break your flow.

Runs **100% locally and offline** by default using Whisper, ONNX, and local Ollama, with optional support for leading cloud AI and speech providers. Built in pure Rust with GTK4 and Libadwaita: **no Electron, no webview, and no Node.js runtime**.

---

## The 7 Suite Pillars

### 1. Speech-to-Text & Live Voice Dictation
- **Fully Offline Inference**: Whisper-family models run locally through `transcribe-cpp` with hardware acceleration (Vulkan GPU compute and OpenBLAS CPU acceleration).
- **Fast ONNX Models**: Parakeet, Moonshine, SenseVoice, GigaAM, Canary, and Cohere run locally via `transcribe-rs`.
- **Intelligent VAD**: Low-latency voice-activity detection powered by Silero VAD v4 with adjustable sensitivity, push-to-talk, and always-on toggle modes.
- **Cloud STT Fallback**: Optional high-speed cloud providers (Groq Whisper, OpenAI Whisper, Deepgram, Mistral, Cerebras, and custom endpoints).
- **Audio Feedback**: Built-in sound themes (pop, marimba) with adjustable volume.
- **File & Batch Transcription**: Transcribe existing audio files via the GUI dialog or headless CLI.

### 2. Meeting Mode (Live Meets)
- Dedicated meeting recording engine (`Ctrl+Alt+M`).
- Captures extended audio sessions without interruption.
- Automatically structures raw transcripts into executive summaries, key discussion points, and actionable follow-ups using local or cloud AI models.

### 3. Selection AI & Contextual Actions
- Highlight text in any application (browser, code editor, email client, PDF viewer).
- Press `Ctrl+Alt+P` to open the prompt palette.
- Instantly proofread, summarize, translate, rewrite in another tone, or explain code. The processed result is pasted back directly into your focused app.

### 4. Quick Notes & Idea Scratchpad
- Instant floating scratchpad overlay (`Ctrl+Alt+N`) for capturing ideas without switching windows.
- Tagging, pinning, full-text search, and one-click voice dictation directly into Markdown notes.
- Seamlessly reviewed and managed in the dedicated workspace.

### 5. Tasks & Voice Todos
- Floating checklist palette (`Ctrl+Alt+T`) for fast task capture.
- Convert spoken ideas into organized action items.
- Check off completed items, filter active vs. completed tasks, and clear finished items.

### 6. Document Parser & Vision OCR
- Multimodal document parser and OCR dialog (`Ctrl+Alt+D`).
- Extract clean, structured Markdown from PDFs, scans, images, screenshots, and Word documents using local parsers and vision-capable AI models.
- Searchable document archive integrated into your local history.

### 7. Web & Deep Research
- Floating research palette (`Ctrl+Alt+S`) and workspace page.
- Conduct real-time web investigations and multi-source AI research powered by Tavily and Firecrawl.
- Deep synthesis mode aggregates and reasons across multiple live sources.

---

## Global Keyboard Shortcuts

All shortcuts are globally registered via the Wayland XDG Desktop Portal (with an evdev engine fallback) and can be customized in **Settings → Keyboard Shortcuts**:

| Action | Default Shortcut | Description |
| :--- | :--- | :--- |
| **Transcribe** | `Super+Space` / `Ctrl+Space` | Start/stop voice dictation into the focused window |
| **Transcribe with AI** | `Ctrl+Shift+Space` | Dictate with automatic LLM cleanup / post-processing |
| **Meeting Mode** | `Ctrl+Alt+M` | Record continuous meeting audio and generate structured minutes |
| **Transform Selection** | `Ctrl+Alt+P` | Open AI prompt palette for currently selected text |
| **Quick Note** | `Ctrl+Alt+N` | Open floating note scratchpad overlay |
| **Tasks & Todos** | `Ctrl+Alt+T` | Open floating task checklist & voice todo palette |
| **Document Parser / OCR** | `Ctrl+Alt+D` | Open document parser & vision OCR dialog |
| **Web & Deep Research** | `Ctrl+Alt+S` | Open floating web research & search overlay |
| **Transcription History** | `Ctrl+Alt+H` | Open floating history palette with quick paste |
| **Cancel** | `Escape` | Cancel recording or dismiss active floating overlay |

---

## Wayland & GNOME Native Integration

- **Transparent Recording Overlay**: A click-through surface displaying state, mic level, and live streaming transcript.
- **Libadwaita Navigation**: Adaptive split-view window conforming to GNOME Human Interface Guidelines (HIG).
- **Theme-Adaptive Tray**: StatusNotifierItem tray icon (via `ksni`) supporting light and dark GNOME Shell panels with idle, recording, transcribing, and alert states.
- **Local SQLite Vault**: Every transcript, note, task, and parsed document is stored securely in `~/.local/share/com.clusterat.otush/history.db`.

---

## Supported AI & Speech Providers

Otush supports both fully private local execution and top-tier cloud APIs:

| Provider | Type | Capabilities |
| :--- | :--- | :--- |
| **Local Whisper / ONNX** | Local | Offline speech-to-text (transcribe-cpp / transcribe-rs) |
| **Ollama** | Local | Offline LLM post-processing, notes, tasks, summaries |
| **OpenAI** | Cloud | Whisper STT, GPT-4o / GPT-4o-mini post-processing & OCR |
| **Anthropic** | Cloud | Claude 3.5 Sonnet / Haiku reasoning & document parsing |
| **Groq** | Cloud | Ultra-fast Whisper STT & Llama-3 inference |
| **Deepgram** | Cloud | High-speed Nova-2 speech recognition |
| **Mistral AI** | Cloud | Mistral & Voxtral speech and language models |
| **Cerebras** | Cloud | High-throughput Llama inference |
| **OpenRouter** | Cloud | Unified access to hundreds of open/proprietary models |
| **Tavily / Firecrawl** | Cloud | Web search, page scraping, and deep research synthesis |

---

## Command-Line Flags

Otush provides full command-line control for scripting, tiling window managers (i3, Sway, Hyprland), and terminal workflows:

| Flag | Description |
| :--- | :--- |
| `--toggle-transcription` | Toggle voice recording on/off (sent to a running instance) |
| `--toggle-post-process` | Toggle recording with AI post-processing on/off |
| `--cancel` | Cancel the current recording or active operation |
| `--start-hidden` | Launch in the background with window hidden (tray icon active) |
| `--no-tray` | Launch without system tray (closing window exits the process) |
| `--debug` | Enable debug mode with verbose trace logging |
| `--transcribe-file <WAV>` | Transcribe a 16 kHz mono WAV headlessly and output text |
| `--list-devices` | List compute devices (CPU, Vulkan GPUs) for transcription |
| `--list-models` | List installed and catalog speech models |

### Window Manager Integration Example
Bind Otush remote commands to your preferred window manager shortcuts:

```bash
# Toggle dictation from any script or WM binding:
otush --toggle-transcription

# Record with AI post-processing:
otush --toggle-post-process

# Cancel in-progress operation:
otush --cancel
```

---

## Data Locations

| Content | Path |
| :--- | :--- |
| **Settings** | `~/.local/share/com.clusterat.otush/settings_store.json` |
| **Speech Models** | `~/.local/share/com.clusterat.otush/models/` |
| **History & Workspace DB** | `~/.local/share/com.clusterat.otush/history.db` |
| **Audio Recordings** | `~/.local/share/com.clusterat.otush/recordings/` |
| **Logs** | `~/.local/share/com.clusterat.otush/logs/otush.log` |

---

## Building and Installation

### Prerequisites (Ubuntu 24.04+ LTS)
Install the required native development libraries:

```bash
sudo apt update
sudo apt install -y \
  libgtk-4-dev \
  libadwaita-1-dev \
  libasound2-dev \
  libssl-dev \
  libvulkan-dev \
  glslc \
  spirv-headers \
  glslang-tools \
  libopenblas-dev
```

### Download the Silero VAD Model
The voice-activity detection model is required for audio segmentation:

```bash
mkdir -p resources/models
curl -o resources/models/silero_vad_v4.onnx https://blob.handy.computer/silero_vad_v4.onnx
```

### Build & Run
```bash
# Debug build & launch
cargo run

# Optimized release build
cargo build --release
```

Speech models can be downloaded directly inside the application under **Preferences → Speech Recognition**, or placed manually into `~/.local/share/com.clusterat.otush/models/`.

### Packaging
- **Debian Package**:
  ```bash
  cargo install cargo-deb
  cargo deb
  ```
- **Flatpak**:
  ```bash
  flatpak-builder --user build-dir packaging/flatpak/com.clusterat.otush.yml
  ```
- **Desktop Entry**: Installed via `packaging/com.clusterat.otush.desktop`.

---

## Troubleshooting

- **Overlay doesn't appear**: Verify that your desktop session is Wayland (`echo $XDG_SESSION_TYPE`).
- **Global shortcuts not triggering under `cargo run`**: Without an installed `.desktop` file, the XDG GlobalShortcuts portal may report that an app ID is missing. Otush automatically falls back to its built-in `evdev` keyboard engine.
- **Microphone access denied**: Ensure portal microphone access is allowed in GNOME Settings → Privacy → Microphone.

---

## Credits & License

- **Otush** is maintained by the Otush contributors and licensed under the [MIT License](LICENSE).
- Otush is derived from [Handy](https://github.com/cjpais/Handy) by Chris Pais.
- The user interface, productivity workspaces (Notes, Tasks, OCR, Web Research), meeting mode, Linux/GNOME integration  and more were natively built/rebuilt for GTK4 & Libadwaita.
