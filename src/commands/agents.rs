//! UI-facing commands for AI chat agents (settings + chat + RAG).
//!
//! GUI-agnostic orchestration over [`crate::agents`]: resolving agents,
//! creating/listing chats, sending turns on the shared Tokio runtime, and
//! indexing/searching local documents for RAG. Progress and completion flow
//! back through [`AppEvent`](crate::context::AppEvent) so GTK widgets update
//! on the main thread; failures use [`AppContext::report_error`].

use crate::agents::runner::{run_agent_turn, StepSink, ToolStep};
use crate::agents::types::resolve_agent;
use crate::commands::{CommandError, CommandResult};
use crate::context::{AppContext, AppEvent};
use crate::managers::history::{AgentChat, AgentMessage};
use crate::settings::{AgentConfig, AGENT_TOOL_NAMES};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

/// One in-flight agent turn, cancellable from the UI (`stop_chat`).
struct LiveSink {
    bus: crate::context::EventBus,
    chat_id: i64,
    cancelled: AtomicBool,
}

impl StepSink for LiveSink {
    fn on_step(&self, step: ToolStep) {
        self.bus.send(AppEvent::AgentStep {
            chat_id: self.chat_id,
            tool: step.name,
            summary: step.summary,
        });
    }

    fn on_token(&self, delta: &str) {
        // Coalescing happens in the UI (one GTK invoke per SSE chunk is
        // fine at token rates; the label update is cheap).
        self.bus.send(AppEvent::AgentToken {
            chat_id: self.chat_id,
            delta: delta.to_string(),
        });
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

/// Active tool-loop turns keyed by chat id (for cooperative Stop).
type ActiveTurns = Vec<(i64, Arc<LiveSink>)>;

static ACTIVE_TURNS: std::sync::LazyLock<std::sync::Mutex<ActiveTurns>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

fn register_sink(chat_id: i64, sink: Arc<LiveSink>) {
    let mut guard = ACTIVE_TURNS.lock().unwrap_or_else(|e| e.into_inner());
    guard.retain(|(id, _)| *id != chat_id);
    guard.push((chat_id, sink));
}

fn take_sink(chat_id: i64) -> Option<Arc<LiveSink>> {
    let mut guard = ACTIVE_TURNS.lock().unwrap_or_else(|e| e.into_inner());
    let pos = guard.iter().position(|(id, _)| *id == chat_id)?;
    Some(guard.remove(pos).1)
}

/// Validate and persist a new agent.
pub fn create_agent(ctx: &AppContext, agent: AgentConfig) -> CommandResult<AgentConfig> {
    validate_agent_fields(ctx, &agent, true)?;
    let mut settings = ctx.settings();
    settings.agents.push(agent.clone());
    ctx.write_settings(&settings);
    ctx.notify_setting_changed("agents", serde_json::json!(&settings.agents));
    Ok(agent)
}

/// Validate and persist changes to an existing agent.
pub fn update_agent(ctx: &AppContext, agent: AgentConfig) -> CommandResult<AgentConfig> {
    validate_agent_fields(ctx, &agent, false)?;
    let mut settings = ctx.settings();
    let Some(pos) = settings.agents.iter().position(|a| a.id == agent.id) else {
        return Err(CommandError::AgentNotFound(agent.id));
    };
    settings.agents[pos] = agent.clone();
    ctx.write_settings(&settings);
    ctx.notify_setting_changed("agents", serde_json::json!(&settings.agents));
    Ok(agent)
}

/// Delete an agent (chats are kept; they rebind to the fallback agent).
pub fn delete_agent(ctx: &AppContext, agent_id: &str) -> CommandResult<()> {
    let mut settings = ctx.settings();
    let Some(pos) = settings.agents.iter().position(|a| a.id == agent_id) else {
        return Err(CommandError::AgentNotFound(agent_id.to_string()));
    };
    settings.agents.remove(pos);
    if settings.selected_agent_id.as_deref() == Some(agent_id) {
        settings.selected_agent_id = None;
    }
    ctx.write_settings(&settings);
    ctx.notify_setting_changed("agents", serde_json::json!(&settings.agents));
    Ok(())
}

/// Select the active agent for the overlay.
pub fn select_agent(ctx: &AppContext, agent_id: &str) -> CommandResult<()> {
    let settings = ctx.settings();
    let Some(agent) = settings.agent(agent_id) else {
        return Err(CommandError::AgentNotFound(agent_id.to_string()));
    };
    if !agent.enabled {
        return Err(CommandError::AgentProvider(format!(
            "Agent '{}' is disabled",
            agent.name
        )));
    }
    let mut settings = settings;
    settings.selected_agent_id = Some(agent_id.to_string());
    ctx.write_settings(&settings);
    ctx.notify_setting_changed("selected_agent_id", serde_json::json!(agent_id));
    Ok(())
}

fn validate_agent_fields(ctx: &AppContext, agent: &AgentConfig, is_new: bool) -> CommandResult<()> {
    if agent.id.trim().is_empty() {
        return Err(CommandError::ToolDenied(
            "Agent id cannot be empty".to_string(),
        ));
    }
    if agent.name.trim().is_empty() {
        return Err(CommandError::ToolDenied(
            "Agent name cannot be empty".to_string(),
        ));
    }
    if agent.system_prompt.trim().is_empty() {
        return Err(CommandError::ToolDenied(
            "Agent system prompt cannot be empty".to_string(),
        ));
    }
    let settings = ctx.settings();
    if is_new && settings.agent(&agent.id).is_some() {
        return Err(CommandError::ToolDenied(format!(
            "Agent id '{}' already exists",
            agent.id
        )));
    }
    let Some(provider) = settings.post_process_provider(&agent.provider_id) else {
        return Err(CommandError::AgentProvider(format!(
            "Unknown provider '{}'",
            agent.provider_id
        )));
    };
    if !provider.enabled {
        return Err(CommandError::AgentProvider(format!(
            "Provider '{}' is disabled. Enable it in Settings → Providers.",
            provider.label
        )));
    }
    for tool in &agent.enabled_tools {
        if !AGENT_TOOL_NAMES.contains(&tool.as_str()) {
            return Err(CommandError::ToolDenied(format!("Unknown tool '{tool}'")));
        }
    }
    Ok(())
}

/// Create a chat for `agent_id` (falls back to the selected agent).
pub fn start_chat(
    ctx: &AppContext,
    agent_id: Option<&str>,
    title: Option<&str>,
) -> CommandResult<AgentChat> {
    let settings = ctx.settings();
    let agent = match agent_id {
        Some(id) => settings
            .agent(id)
            .ok_or_else(|| CommandError::AgentNotFound(id.to_string()))?,
        None => settings
            .selected_agent()
            .ok_or_else(|| CommandError::AgentNotFound("no agents configured".to_string()))?,
    };
    let chat = ctx
        .history
        .create_agent_chat(&agent.id, title.unwrap_or_default())
        .map_err(CommandError::Backend)?;
    ctx.bus.send(AppEvent::AgentChatsChanged);
    Ok(chat)
}

/// List messages of one chat (oldest first).
pub fn list_messages(ctx: &AppContext, chat_id: i64) -> CommandResult<Vec<AgentMessage>> {
    ctx.history
        .list_agent_messages(chat_id)
        .map_err(CommandError::Backend)
}

/// List recent chats, newest first (optionally filtered by agent).
pub fn list_chats(
    ctx: &AppContext,
    agent_id: Option<&str>,
) -> CommandResult<Vec<crate::managers::history::AgentChat>> {
    ctx.history
        .list_agent_chats(agent_id)
        .map_err(CommandError::Backend)
}

/// Delete one chat and notify the overlay.
pub fn delete_chat(ctx: &AppContext, chat_id: i64) -> CommandResult<()> {
    ctx.history
        .delete_agent_chat(chat_id)
        .map_err(CommandError::Backend)?;
    ctx.bus.send(AppEvent::AgentChatsChanged);
    Ok(())
}

/// Rename one chat and notify the overlay.
pub fn rename_chat(ctx: &AppContext, chat_id: i64, title: &str) -> CommandResult<()> {
    ctx.history
        .rename_agent_chat(chat_id, title)
        .map_err(CommandError::Backend)?;
    ctx.bus.send(AppEvent::AgentChatsChanged);
    Ok(())
}

/// Cancel the in-flight turn of a chat, if any.
pub fn stop_chat(ctx: &AppContext, chat_id: i64) -> CommandResult<()> {
    if let Some(sink) = take_sink(chat_id) {
        sink.cancelled.store(true, Ordering::Relaxed);
    }
    ctx.bus.send(AppEvent::AgentDone {
        chat_id,
        message_id: None,
        truncated: false,
    });
    Ok(())
}

/// Send a user message: persists user + placeholder, spawns the ReAct turn,
/// and streams `AgentStep`/`AgentDone` events. Returns immediately.
pub fn send_message(ctx: &AppContext, chat_id: i64, text: &str) -> CommandResult<()> {
    let text = text.trim();
    if text.is_empty() {
        return Err(CommandError::ToolDenied(
            "Message cannot be empty".to_string(),
        ));
    }
    {
        let guard = ACTIVE_TURNS.lock().unwrap_or_else(|e| e.into_inner());
        if guard.iter().any(|(id, _)| *id == chat_id) {
            return Err(CommandError::ToolDenied(
                "This chat is already generating. Stop it first.".to_string(),
            ));
        }
    }
    let settings = ctx.settings();
    let chat = ctx
        .history
        .list_agent_chats(None)
        .map_err(CommandError::Backend)?
        .into_iter()
        .find(|c| c.id == chat_id)
        .ok_or_else(|| CommandError::AgentNotFound(format!("chat {chat_id}")))?;
    let agent = settings
        .agent(&chat.agent_id)
        .or_else(|| settings.selected_agent())
        .ok_or_else(|| CommandError::AgentNotFound(chat.agent_id.clone()))?;
    let resolved = resolve_agent(&settings, &agent.id).map_err(|msg| {
        if msg.contains("model") || msg.contains("Model") {
            CommandError::AgentModel(msg)
        } else {
            CommandError::AgentProvider(msg)
        }
    })?;

    ctx.history
        .append_agent_message(chat_id, "user", text, None)
        .map_err(CommandError::Backend)?;
    let assistant_row = ctx
        .history
        .append_agent_message(chat_id, "assistant", "…", None)
        .map_err(CommandError::Backend)?;
    ctx.bus.send(AppEvent::AgentMessageAdded {
        chat_id,
        message: crate::context::AgentMessageEvent {
            id: assistant_row.id,
            role: "assistant".to_string(),
            content: "…".to_string(),
            tool_name: None,
        },
    });

    let sink = Arc::new(LiveSink {
        bus: ctx.bus.clone(),
        chat_id,
        cancelled: AtomicBool::new(false),
    });
    register_sink(chat_id, sink.clone());

    let task_ctx = ctx.clone();
    let user_text = text.to_string();
    // Auto-title empty chats from the first user message (cheap, local).
    let is_first_turn = task_ctx
        .history
        .list_agent_messages(chat_id)
        .map(|msgs| {
            msgs.iter()
                .filter(|m| m.role == "user" || (m.role == "assistant" && m.content != "…"))
                .count()
                <= 1
        })
        .unwrap_or(false);
    if is_first_turn {
        let title: String = text.chars().take(60).collect();
        if !title.trim().is_empty() {
            let _ = task_ctx.history.rename_agent_chat(chat_id, &title);
            task_ctx.bus.send(AppEvent::AgentChatsChanged);
        }
    }
    crate::runtime::spawn(async move {
        let outcome =
            run_agent_turn(&task_ctx, &resolved, chat_id, &user_text, sink.as_ref()).await;
        let _ = take_sink(chat_id);
        match outcome {
            Ok(outcome) => {
                let final_text = if outcome.text.trim().is_empty() {
                    "(no response)".to_string()
                } else {
                    outcome.text.clone()
                };
                // Replace the "…" placeholder with the final text.
                let conn_text = final_text.clone();
                let _ =
                    task_ctx
                        .history
                        .append_agent_message(chat_id, "assistant", &conn_text, None);
                task_ctx.bus.send(AppEvent::AgentDone {
                    chat_id,
                    message_id: Some(assistant_row.id),
                    truncated: outcome.truncated_by_step_cap,
                });
                // Emit the full text as an update so the overlay can swap the
                // placeholder without refetching the whole transcript.
                task_ctx.bus.send(AppEvent::AgentMessageAdded {
                    chat_id,
                    message: crate::context::AgentMessageEvent {
                        id: assistant_row.id,
                        role: "assistant".to_string(),
                        content: final_text,
                        tool_name: None,
                    },
                });
            }
            Err(err) => {
                task_ctx.report_error("agent_chat", &err);
                task_ctx.bus.send(AppEvent::AgentDone {
                    chat_id,
                    message_id: Some(assistant_row.id),
                    truncated: false,
                });
            }
        }
    });
    Ok(())
}

/// Export one chat as Markdown (messages + tool summaries).
pub fn export_chat_markdown(ctx: &AppContext, chat_id: i64) -> CommandResult<String> {
    let messages = ctx
        .history
        .list_agent_messages(chat_id)
        .map_err(CommandError::Backend)?;
    let mut md = String::from("# Agent chat\n\n");
    for msg in messages {
        match msg.role.as_str() {
            "user" => md.push_str(&format!("## You\n\n{}\n\n", msg.content)),
            "assistant" => md.push_str(&format!("## Assistant\n\n{}\n\n", msg.content)),
            _ => md.push_str(&format!(
                "## Tool ({})\n\n{}\n\n",
                msg.tool_name.as_deref().unwrap_or("tool"),
                msg.content
            )),
        }
    }
    Ok(md)
}

/// Index parsed suite documents (`suite_docs` ids) into the RAG store.
/// Chunks land without embeddings; when a provider configures an
/// `embeddings_model`, a background backfill is kicked off automatically.
pub fn index_suite_docs(ctx: &AppContext, doc_ids: &[i64]) -> CommandResult<usize> {
    let docs = ctx.history.list_docs().map_err(CommandError::Backend)?;
    let wanted: Vec<_> = docs
        .into_iter()
        .filter(|d| doc_ids.contains(&d.id))
        .map(|d| {
            (
                "suite_doc".to_string(),
                d.id.to_string(),
                d.title.clone(),
                format!("otush://suite_doc/{}", d.id),
                d.parsed_content.clone(),
            )
        })
        .collect();
    let indexed =
        crate::agents::rebuild_index_for_docs(ctx, &wanted).map_err(CommandError::ToolFailed)?;
    backfill_embeddings(ctx);
    Ok(indexed)
}

/// Clear the whole RAG index.
pub fn clear_rag_index(ctx: &AppContext) -> CommandResult<()> {
    ctx.history.clear_rag_index().map_err(CommandError::Backend)
}

/// Embedding status for the Agents page: (total chunks, embedded chunks,
/// active model or `None`).
pub fn rag_embedding_status(ctx: &AppContext) -> (u64, u64, Option<String>) {
    let settings = ctx.settings();
    let model = settings
        .post_process_providers
        .iter()
        .filter(|p| p.enabled)
        .filter_map(|p| {
            p.embeddings_model
                .as_deref()
                .map(str::trim)
                .filter(|m| !m.is_empty())
                .map(|m| (p.id.clone(), m.to_string()))
        })
        .next()
        .map(|(_, model)| model);
    let (total, embedded) = ctx.history.rag_embedding_stats(model.as_deref());
    (total, embedded, model)
}

/// Kick off a bounded background job embedding chunks that miss vectors for
/// the first enabled provider with an `embeddings_model`. No-op when no
/// provider configures one (pure FTS5 mode) or when nothing is missing.
/// Batches of 32, at most 256 chunks per kick, so a huge re-index cannot
/// stall the runtime; the next kick (next indexing or chat turn) continues.
pub fn backfill_embeddings(ctx: &AppContext) {
    let settings = ctx.settings();
    let Some((provider, model, api_key)) = settings
        .post_process_providers
        .iter()
        .filter(|p| p.enabled)
        .filter_map(|p| {
            let model = p
                .embeddings_model
                .as_deref()
                .map(str::trim)
                .filter(|m| !m.is_empty())?;
            let key = settings
                .post_process_api_keys
                .get(&p.id)
                .cloned()
                .unwrap_or_default();
            Some((p.clone(), model.to_string(), key))
        })
        .next()
    else {
        return;
    };
    if api_key.trim().is_empty() && provider.id != "custom" && provider.id != "ollama" {
        return;
    }
    let missing = ctx
        .history
        .rag_chunks_missing_embedding(&model, 256)
        .unwrap_or_default();
    if missing.is_empty() {
        return;
    }
    let task_ctx = ctx.clone();
    crate::runtime::spawn(async move {
        for batch in missing.chunks(32) {
            let inputs: Vec<String> = batch.iter().map(|c| c.content.clone()).collect();
            let vectors = match crate::llm_client::fetch_embeddings(
                &provider,
                api_key.clone(),
                &model,
                &inputs,
            )
            .await
            {
                Ok(vectors) => vectors,
                Err(err) => {
                    task_ctx.report_error("rag_embeddings", err);
                    return;
                }
            };
            for (chunk, vector) in batch.iter().zip(vectors.iter()) {
                let bytes = crate::llm_client::encode_embedding(vector);
                if let Err(err) =
                    task_ctx
                        .history
                        .store_chunk_embedding(chunk.chunk_id, &model, &bytes)
                {
                    task_ctx.report_error("rag_embeddings", err);
                    return;
                }
            }
        }
        task_ctx.bus.send(AppEvent::SettingsChanged {
            setting: "rag_index".to_string(),
            value: serde_json::json!({ "embedded": true }),
        });
    });
}
