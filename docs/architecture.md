# Otush Architecture

This document describes the intended layering of the Otush codebase
(a native GNOME app: GTK4 + libadwaita UI, Rust backend in the same process),
the threading contract every contributor must follow, and the direction of the
ongoing clean-up. New code must follow these rules; legacy code is being
migrated toward them incrementally.

## Layering

```text
managers/ + audio_toolkit/        GUI-agnostic backend
        │  (audio capture/VAD, transcription, models, history)
        ▼
commands/ + actions/              command layer (plain fns on &AppContext)
        │  (TranscribeAction pipeline, post-processing, cancel)
        ▼
ui/ + overlay/ + tray.rs          GTK4/libadwaita shell (main thread only)
```

- The backend **never touches a GTK widget**. It communicates outward only
  through [`AppContext`][ctx] (paths, settings, managers) and the
  [`EventBus`][bus] (`AppEvent` fan-out).
- The UI calls inward only through `commands::*` functions and
  `actions::ACTION_MAP` (driven by the [`TranscriptionCoordinator`][coord]).
- `utils.rs` holds a few process-wide helpers (`cancel_current_operation`,
  `redact_text`, environment detection) plus explicit re-exports of the
  overlay/tray/paste entry points. Import `overlay`/`tray`/`clipboard`
  directly in new code.

[ctx]: ../src/context.rs
[bus]: ../src/context.rs
[coord]: ../src/transcription_coordinator.rs

## Threading contract

| Work                                    | Where it runs                              |
| --------------------------------------- | ------------------------------------------ |
| GTK widgets, `AdwToast`, CSS, `present` | GTK main thread only, via `glib::MainContext::default().invoke(..)` |
| Backend async (reqwest, portal, downloads) | Global Tokio runtime via `runtime::spawn` |
| CPU/IO blocking (transcribe, WAV, DB)   | `runtime::spawn_blocking` (named)          |
| Lifecycle transitions (record/stop)     | `TranscriptionCoordinator` thread (pure `CoordinatorState` + `Effect`s) |
| Audio callback                          | cpal thread: metering + VAD feed only; never blocks, never allocates per frame on the hot path |

Rules:

1. **Never touch a widget off the main thread.** Backend callbacks marshal
   with `glib::MainContext::default().invoke`, capture only
   `glib::SendWeakRef`s, and check `upgrade()` before use.
2. **Never block the main thread.** Model load, transcription, downloads, and
   SQLite writes run on workers; results hop back via `invoke`.
3. **Never hold a mutex across I/O or a callback.** Keep critical sections
   short; recover from poisoning (`into_inner()` + log) instead of panicking.
4. **Timeouts and spawned helpers always have liveness checks** — a
   `timeout_add` callback or async continuation must re-validate its weak refs
   and cancel generation before touching state.

## Key flows

- **Shortcut → coordinator → action → pipeline:** a portal/evdev edge reaches
  `TranscriptionCoordinator::send_input`, which serializes lifecycle decisions
  (debounce, PTT grace, busy-pipeline remember/forget) and executes
  `Start`/`Stop` effects via `ACTION_MAP`.
- **Pipeline:** `TranscribeAction::stop` stops capture → DSP → parallel WAV
  save + transcribe (stream finalize → batch → cloud fallback) →
  post-processing → history → clipboard/paste → overlay/tray reset.
  `FinishGuard` guarantees exactly-once `notify_processing_finished`.
- **Backend → UI:** `EventBus::send(AppEvent::…)` from any thread; the GTK
  shell subscribes once (`app.rs`) and pages/palettes subscribe with weak
  refs, upgrading on the main loop.
- **Headless CLI:** `--transcribe-file` / `--list-models` / `--list-devices`
  run before the GTK shell starts and exit with a result code; `stdout` stays
  parseable (logs go to `stderr`).

## Settings and secrets

- `settings.rs` is the schema + defaults + migrations + JSON store
  (`settings_store.json`, atomic temp+rename write, corruption salvage). It is
  being split into `settings/{schema,defaults,providers,migrations,store}`.
- Secrets (`SecretMap`) are debug-redacted. Never log API keys, pasted text,
  or full transcripts; `redact_text` redacts diagnostics in release builds.

## Compatibility constraints

- Target is Linux/GNOME/Wayland only. Keep `libadwaita` `v1_3`/`v1_4`
  feature compatibility for Ubuntu 24.04; gate newer Adwaita APIs.
- Single instance via `GApplication`: second launches forward their command
  line to the primary (`app.rs` `connect_command_line`).
- The recording overlay is a fullscreen transparent surface (Mutter lacks
  wlr-layer-shell); an empty input region keeps it click-through. GTK4 has
  no partial invalidation, so the pulse timer repaints at 10 FPS while
  visible and is removed on hide.
- Settings pages build lazily on first sidebar selection; rebuilt groups
  keep an owned `PageGroup` (weak row refs) instead of a global keyed map.

## Clean-up direction (in progress)

- Locks recover from poisoning (`unwrap_or_else(|e| e.into_inner())`) instead
  of panicking; the `utils.rs` overlay/tray/clipboard re-export facade was
  removed (import those modules directly).
- UI fire-and-forget failures are reported, not swallowed: `AppContext::
  report_error` logs (warn) and emits `AppEvent::CommandFailed`, which the
  shell surfaces as a toast. Do not reintroduce `let _ = …` on command calls.
- `shortcut/mod.rs` is split into `shortcut/settings_*.rs` by domain, and
  zero-caller settings setters were deleted outright (verified per-name:
  each lived only in its `settings_*.rs` file). The UI writes those fields
  inline today; reintroduce a setter only together with a real caller — do
  not re-add unused wrappers.
- (The unimplementable-on-Wayland receipt-sequenced `paste_tx` module and its
  `reliable_paste` setting were deleted outright: Wayland offers no
  clipboard-read receipts, so the contract could never run on the target
  platform.)
- Remaining: split the other god modules (`settings.rs`,
  `managers/transcription.rs`, `actions.rs`, `overlay.rs`) without changing
  serialized keys or user-visible behavior; introduce typed errors
  (`thiserror`) at UI boundaries instead of `Result<_, String>`.
- Narrow dependency features (`symphonia`, `reqwest`, `image`), pin git deps
  with `rev`, and declare explicit `tokio` features.

## Module splits (done; pure code motion, no behavior change)

- `settings.rs` → `settings/{schema,defaults,store,migrations}.rs`:
  persisted types live in `schema.rs`, serde defaults in `defaults.rs`,
  file I/O in `store.rs`, version upgrades in `migrations.rs`. The
  `settings.rs` facade re-exports the public surface; serialized keys are
  untouched.
- `managers/transcription.rs` → `managers/transcription/
  {types,manager,streaming,transcribe,backend,tests}.rs`: stream events and
  the router in `types.rs`, the manager lifecycle in `manager.rs`, worker
  helpers in `streaming.rs`, batch helpers in `transcribe.rs`,
  devices/accelerators in `backend.rs`. Zero-caller items stay
  module-private via `super::` imports (no facade re-export).
- `actions.rs` → `actions/{transcribe,palette,support}.rs`: the
  transcription pipeline in `transcribe.rs`, palette/utility actions plus
  `ACTION_MAP` in `palette.rs`, cancellation primitives in `support.rs`.
- `overlay.rs` → `overlay/{state,paint,tests}.rs`: window lifecycle and
  events in `state.rs`, Cairo layout/painting in `paint.rs`; sibling
  modules share statics via `pub(super)` imports. Asset `include!` paths
  are relative to `src/overlay/`.
- `commands/errors.rs` defines the typed [`CommandError`][cmd-err] enum
  (`thiserror`): every `commands::*` function returns
  `CommandResult<T>` instead of `Result<T, String>`. Each variant's
  `Display` text matches the legacy `String` message byte-for-byte, so
  toasts, logs, and CLI output are unchanged — but callers can now match
  on failure kind instead of message text.

[cmd-err]: ../src/commands/errors.rs
