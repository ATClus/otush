//! AI chat agents: types, tool catalog, RAG retrieval, and the ReAct runner.
//!
//! GUI-agnostic by design (same rule as `managers/`): the runner never touches
//! GTK. The UI drives it through `commands::agents` and receives progress via
//! [`AppEvent`](crate::context::AppEvent). All network calls run on the shared
//! Tokio runtime via [`crate::runtime`]; API keys are read from settings at
//! call time and never logged.

pub mod rag;
pub mod runner;
pub mod tools;
pub mod types;

pub use rag::{chunk_text, rag_search, rebuild_index_for_docs};
pub use runner::{run_agent_turn, RunOutcome, StepSink, ToolStep};
pub use tools::{definitions, execute_tool, ToolExecution};
pub use types::{AgentResolved, ChatRole, ResolvedTool};
