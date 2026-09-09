//! GTK4/libadwaita interface: main window, settings pages, floating palettes.
//!
//! [`window`] builds the navigation shell (sidebar + content stack with a
//! toast overlay). [`pages`] holds one builder per sidebar section; floating
//! palettes (`history_palette`, `notes_palette`, `todo_palette`,
//! `prompt_palette`, `chat_overlay` + `search_mode`) and [`doc_parser`] /
//! [`file_transcription`] are lazily built overlays. All widgets live on the
//! GTK main thread; backend contact goes through `commands::*` on
//! `&AppContext` and returns via the [`EventBus`](crate::context::EventBus).

pub mod chat_overlay;
pub mod doc_parser;
pub mod file_transcription;
pub mod history_palette;
pub mod markdown;
pub mod notes_palette;
pub mod pages;
pub mod prompt_palette;
pub mod search_mode;
pub mod todo_palette;
pub mod tts_controls;
pub mod window;
