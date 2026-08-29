# Contributing to Otush

Thank you for your interest in contributing to Otush! This guide covers how to
get started with this open source, offline speech-to-text application for
GNOME.

## Getting started

1. Fork the repository and clone your fork:

   ```bash
   git clone git@github.com:YOUR_USERNAME/otush.git
   cd otush
   ```

2. Install the native build dependencies (see [BUILD.md](BUILD.md)) and the
   Silero VAD model:

   ```bash
   mkdir -p resources/models
   curl -o resources/models/silero_vad_v4.onnx https://blob.handy.computer/silero_vad_v4.onnx
   ```

3. Create a branch for your work and make your changes.

## Development workflow

- **Build & run:** `cargo build` / `cargo run`
- **Before committing, run:**
  ```bash
  cargo fmt
  cargo clippy -- -D warnings
  cargo test
  ```
- Follow the existing architecture (see [AGENTS.md](AGENTS.md)): the backend
  talks to the UI only through `AppContext` + `EventBus`; GTK widgets are
  never touched off the main thread.
- Handle errors explicitly — avoid `unwrap` in production code.

## Committing

Use conventional commit prefixes (`feat:`, `fix:`, `docs:`, `refactor:`,
`chore:`), and focus the message on _why_, not _what_.

## Branding

This project is **Otush** (`com.clusterat.otush`). Do not reintroduce the
upstream "Handy" name in user-facing strings, packaging or identifiers — except
where it is a functional reference (`handy-computer` Hugging Face org,
`blob.handy.computer` model hosting, and the `handy-keys` crate, aliased as
`evdev-keys` in `Cargo.toml`). The upstream Handy project is credited in
[README.md](README.md).

## License

By contributing to Otush, you agree that your contributions will be licensed
under the MIT License. See [LICENSE](LICENSE) for details.
