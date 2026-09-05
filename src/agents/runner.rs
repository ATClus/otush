//! ReAct runner: the agentic loop `llm → tool_calls → execute → llm → …`.
//!
//! One [`run_agent_turn`] call performs a single user turn: it appends the
//! user message, optionally injects RAG context, then loops until the model
//! answers or `max_tool_steps` is reached. Progress flows through the
//! [`StepSink`] callback (the commands layer forwards it as `AppEvent`s).
//!
//! The LLM and tool executor are injectable so the loop is unit-testable
//! without network access; production wires
//! `llm_client::send_chat_messages_streamed` and [`crate::agents::tools`].

use crate::agents::rag;
use crate::agents::types::{AgentResolved, ResolvedTool};
use crate::context::AppContext;
use crate::llm_client::{AssistantReply, ChatMessage, ToolCallOut, ToolDefinition};

/// One executed tool step, surfaced to the UI timeline.
#[derive(Clone, Debug)]
pub struct ToolStep {
    pub name: String,
    pub summary: String,
}

/// Progress callback invoked from the runner thread. Must be cheap and
/// non-blocking; the commands layer marshals onto the GTK main loop.
/// `Send + Sync` so a `&dyn StepSink` can cross `.await` points on the
/// multi-thread Tokio runtime.
pub trait StepSink: Send + Sync {
    fn on_step(&self, step: ToolStep);
    /// One streamed content token of the final answer (SSE delta). The
    /// default ignores it so test sinks stay minimal.
    fn on_token(&self, _delta: &str) {}
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// Outcome of one agent turn.
#[derive(Clone, Debug)]
pub struct RunOutcome {
    /// Final assistant text (possibly partial when steps ran out).
    pub text: String,
    pub steps_executed: u32,
    /// True when the loop stopped because of the step cap, not an answer.
    pub truncated_by_step_cap: bool,
}

/// LLM backend for the loop. Production uses
/// `llm_client::send_chat_messages_streamed`; the optional
/// `stream` sink receives SSE content deltas for live UI rendering.
pub trait LlmBackend: Send + Sync {
    fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDefinition],
        stream: Option<&dyn crate::llm_client::StreamSink>,
    ) -> impl std::future::Future<Output = Result<AssistantReply, String>> + Send;
}

/// Tool executor for the loop. Production uses [`crate::agents::tools`].
pub trait ToolExecutor: Send + Sync {
    fn execute(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> impl std::future::Future<Output = Result<(String, String), String>> + Send;
}

/// Production wiring: real LLM + real web tools.
pub struct LiveBackend {
    pub resolved: AgentResolved,
    pub settings: crate::settings::AppSettings,
}

impl LlmBackend for LiveBackend {
    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDefinition],
        stream: Option<&dyn crate::llm_client::StreamSink>,
    ) -> Result<AssistantReply, String> {
        crate::llm_client::send_chat_messages_streamed(
            &self.resolved.provider,
            self.resolved.api_key.clone(),
            &self.resolved.model,
            messages.to_vec(),
            if tools.is_empty() {
                None
            } else {
                Some(tools.to_vec())
            },
            false,
            stream,
        )
        .await
    }
}

impl ToolExecutor for LiveBackend {
    async fn execute(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<(String, String), String> {
        crate::agents::tools::execute_tool(&self.settings, name, args)
            .await
            .map(|exec| (exec.content, exec.summary))
    }
}

/// Run one user turn with injectable backends. Pure orchestration: no GTK,
/// no globals, cancellation checked before every LLM call and tool exec.
pub async fn run_turn_with<L, E>(
    agent: &AgentResolved,
    history: Vec<(String, String, Option<String>)>,
    user_text: &str,
    rag_block: Option<String>,
    llm: &L,
    tools: &E,
    sink: &dyn StepSink,
) -> Result<RunOutcome, String>
where
    L: LlmBackend,
    E: ToolExecutor,
{
    let mut messages: Vec<ChatMessage> = Vec::new();
    if !agent.agent.system_prompt.trim().is_empty() {
        messages.push(ChatMessage::system(agent.agent.system_prompt.clone()));
    }
    if let Some(block) = rag_block {
        messages.push(ChatMessage::system(block));
    }
    for (role, content, _tool_name) in history {
        match role.as_str() {
            "assistant" => messages.push(ChatMessage::user(content)),
            "tool" => messages.push(ChatMessage::user(content)),
            _ => messages.push(ChatMessage::user(content)),
        }
    }
    // NOTE: history rows are re-emitted as plain user/system text for the
    // request because the persisted transcript collapses assistant+tool turns
    // into readable Markdown; only the live loop below carries real
    // `assistant(tool_calls)` ↔ `tool` message pairs.
    messages.push(ChatMessage::user(user_text));

    let tool_defs = crate::agents::tools::definitions(&agent.agent.effective_tools());
    let max_steps = agent.agent.effective_max_steps();
    let tool_budget = agent.agent.effective_tool_budget();
    let mut steps_executed: u32 = 0;
    // Per-tool call counts inside this turn: once a tool hits `tool_budget`,
    // further calls are refused with a retry hint instead of executing.
    let mut tool_counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();

    /// Token forwarder: streams final-answer deltas to the step sink while
    /// doubling as the `StreamSink` the LLM client needs for cancellation.
    struct TokenForward<'a> {
        sink: &'a dyn StepSink,
    }
    impl crate::llm_client::StreamSink for TokenForward<'_> {
        fn on_token(&self, delta: &str) {
            self.sink.on_token(delta);
        }
        fn is_cancelled(&self) -> bool {
            self.sink.is_cancelled()
        }
    }
    let forward = TokenForward { sink };

    loop {
        if sink.is_cancelled() {
            return Err("Chat stopped by user".to_string());
        }
        let reply = llm.chat(&messages, &tool_defs, Some(&forward)).await?;
        if reply.tool_calls.is_empty() {
            let text = reply.content.unwrap_or_default();
            return Ok(RunOutcome {
                text,
                steps_executed,
                truncated_by_step_cap: false,
            });
        }
        if steps_executed >= max_steps {
            let text = reply.content.unwrap_or_default();
            let text = if text.trim().is_empty() {
                format!(
                    "I ran out of tool steps ({max_steps}) before finishing. Raise the agent's step limit in Settings → Agents if this keeps happening."
                )
            } else {
                text
            };
            return Ok(RunOutcome {
                text,
                steps_executed,
                truncated_by_step_cap: true,
            });
        }
        // Execute the calls, then feed results back as `tool` messages.
        let calls: Vec<ToolCallOut> = reply.tool_calls.into_iter().take(4).collect();
        let resolved: Vec<ResolvedTool> = calls
            .iter()
            .map(|call| {
                let args: serde_json::Value = serde_json::from_str(&call.function.arguments)
                    .unwrap_or_else(
                        |_| serde_json::json!({ "_parse_error": call.function.arguments }),
                    );
                ResolvedTool {
                    id: call.id.clone(),
                    name: call.function.name.clone(),
                    arguments: args,
                }
            })
            .collect();
        messages.push(ChatMessage::assistant_with_tools(
            reply.content.clone().unwrap_or_default(),
            calls,
        ));
        for tool in resolved {
            if sink.is_cancelled() {
                return Err("Chat stopped by user".to_string());
            }
            if tool.arguments.get("_parse_error").is_some() {
                let msg = format!(
                    "Tool '{}' received invalid JSON arguments and was not executed. Retry with valid JSON matching the tool schema.",
                    tool.name
                );
                messages.push(ChatMessage::tool_result(tool.id.clone(), msg.clone()));
                sink.on_step(ToolStep {
                    name: tool.name.clone(),
                    summary: "invalid arguments — rejected".to_string(),
                });
                steps_executed += 1;
                continue;
            }
            let used = tool_counts.get(&tool.name).copied().unwrap_or(0);
            if used >= tool_budget {
                let msg = format!(
                    "Tool '{}' already reached its per-turn budget ({tool_budget} calls). Summarize what you have or try a different tool instead of calling it again.",
                    tool.name
                );
                messages.push(ChatMessage::tool_result(tool.id.clone(), msg.clone()));
                sink.on_step(ToolStep {
                    name: tool.name.clone(),
                    summary: format!("budget reached ({tool_budget}) — refused"),
                });
                steps_executed += 1;
                continue;
            }
            tool_counts.insert(tool.name.clone(), used + 1);
            match tools.execute(&tool.name, &tool.arguments).await {
                Ok((content, summary)) => {
                    messages.push(ChatMessage::tool_result(tool.id.clone(), content));
                    sink.on_step(ToolStep {
                        name: tool.name.clone(),
                        summary,
                    });
                }
                Err(err) => {
                    messages.push(ChatMessage::tool_result(
                        tool.id.clone(),
                        format!("Tool '{}' failed: {err}. Explain the limitation to the user or try a different tool.", tool.name),
                    ));
                    sink.on_step(ToolStep {
                        name: tool.name.clone(),
                        summary: format!("failed: {err}"),
                    });
                }
            }
            steps_executed += 1;
            if steps_executed >= max_steps {
                break;
            }
        }
    }
}

/// Production entry: loads history + RAG, runs the loop, persists the turn.
pub async fn run_agent_turn(
    ctx: &AppContext,
    agent: &AgentResolved,
    chat_id: i64,
    user_text: &str,
    sink: &dyn StepSink,
) -> Result<RunOutcome, String> {
    let history_rows = ctx
        .history
        .list_agent_messages(chat_id)
        .map_err(|e| format!("Failed to load chat history: {e}"))?;
    let history: Vec<(String, String, Option<String>)> = history_rows
        .into_iter()
        .map(|m| (m.role, m.content, m.tool_name))
        .collect();

    let rag_block = if agent.agent.rag_enabled {
        let (query_vector, embedding_model) = resolve_query_embedding(ctx, user_text).await;
        let passages = rag::hybrid_search(
            ctx,
            user_text,
            agent.agent.effective_top_k(),
            query_vector.as_deref(),
            embedding_model.as_deref(),
        );
        rag::render_context_block(&passages)
    } else {
        None
    };

    let settings = ctx.settings();
    let live = LiveBackend {
        resolved: agent.clone(),
        settings,
    };
    run_turn_with(agent, history, user_text, rag_block, &live, &live, sink).await
}

/// Embed the user query when a provider configures an `embeddings_model`.
/// Returns `(vector, model)`; `None` on any failure (chat falls back to pure
/// BM25). Also kicks an opportunistic backfill so partially embedded indexes
/// converge without the user opening Settings.
async fn resolve_query_embedding(
    ctx: &AppContext,
    user_text: &str,
) -> (Option<Vec<f32>>, Option<String>) {
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
        return (None, None);
    };
    if api_key.trim().is_empty() && provider.id != "custom" && provider.id != "ollama" {
        return (None, None);
    }
    crate::commands::agents::backfill_embeddings(ctx);
    let query = user_text.trim();
    if query.is_empty() {
        return (None, None);
    }
    match crate::llm_client::fetch_embeddings(&provider, api_key, &model, &[query.to_string()])
        .await
    {
        Ok(mut vectors) => match vectors.pop() {
            Some(vector) if !vector.is_empty() => (Some(vector), Some(model)),
            _ => (None, None),
        },
        Err(err) => {
            log::debug!("RAG query embedding failed ({model}): {err}");
            (None, None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::defaults::get_default_settings;
    use std::sync::{Arc, Mutex};

    struct NoopSink;
    impl StepSink for NoopSink {
        fn on_step(&self, _step: ToolStep) {}
    }

    struct RecordingSink {
        steps: Mutex<Vec<ToolStep>>,
    }
    impl StepSink for RecordingSink {
        fn on_step(&self, step: ToolStep) {
            self.steps
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(step);
        }
    }

    fn test_agent() -> AgentResolved {
        let settings = get_default_settings();
        let provider = settings
            .post_process_provider("openai")
            .expect("openai provider")
            .clone();
        AgentResolved {
            agent: settings
                .agent("chat-assistant")
                .expect("seed agent")
                .clone(),
            provider,
            model: "test-model".to_string(),
            api_key: "sk-test".to_string(),
        }
    }

    struct ScriptLlm {
        replies: Mutex<Vec<AssistantReply>>,
    }
    impl LlmBackend for ScriptLlm {
        async fn chat(
            &self,
            _messages: &[ChatMessage],
            _tools: &[ToolDefinition],
            _stream: Option<&dyn crate::llm_client::StreamSink>,
        ) -> Result<AssistantReply, String> {
            let mut replies = self.replies.lock().unwrap_or_else(|e| e.into_inner());
            Ok(replies.remove(0))
        }
    }

    struct FakeTools {
        calls: Mutex<Vec<(String, serde_json::Value)>>,
    }
    impl ToolExecutor for FakeTools {
        async fn execute(
            &self,
            name: &str,
            args: &serde_json::Value,
        ) -> Result<(String, String), String> {
            self.calls
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((name.to_string(), args.clone()));
            Ok((format!("result-of-{name}"), format!("{name} ok")))
        }
    }

    fn tool_reply(id: &str, name: &str) -> AssistantReply {
        AssistantReply {
            content: None,
            tool_calls: vec![ToolCallOut {
                id: id.to_string(),
                call_type: "function".to_string(),
                function: crate::llm_client::ToolFunctionCall {
                    name: name.to_string(),
                    arguments: "{\"query\":\"rust\"}".to_string(),
                },
            }],
        }
    }

    #[tokio::test]
    async fn direct_answer_needs_no_tools() {
        let agent = test_agent();
        let llm = ScriptLlm {
            replies: Mutex::new(vec![AssistantReply {
                content: Some("hello".to_string()),
                tool_calls: vec![],
            }]),
        };
        let tools = FakeTools {
            calls: Mutex::new(Vec::new()),
        };
        let outcome = run_turn_with(&agent, vec![], "hi", None, &llm, &tools, &NoopSink)
            .await
            .expect("turn");
        assert_eq!(outcome.text, "hello");
        assert_eq!(outcome.steps_executed, 0);
        assert!(!outcome.truncated_by_step_cap);
    }

    #[tokio::test]
    async fn tool_then_answer_feeds_result_back() {
        let agent = test_agent();
        let seen: Arc<Mutex<Vec<usize>>> = Arc::new(Mutex::new(Vec::new()));
        struct CountingLlm {
            replies: Mutex<Vec<AssistantReply>>,
            seen: Arc<Mutex<Vec<usize>>>,
        }
        impl LlmBackend for CountingLlm {
            async fn chat(
                &self,
                messages: &[ChatMessage],
                _tools: &[ToolDefinition],
                _stream: Option<&dyn crate::llm_client::StreamSink>,
            ) -> Result<AssistantReply, String> {
                self.seen
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(messages.len());
                let mut replies = self.replies.lock().unwrap_or_else(|e| e.into_inner());
                Ok(replies.remove(0))
            }
        }
        let llm = CountingLlm {
            replies: Mutex::new(vec![
                tool_reply("c1", "tavily_search"),
                AssistantReply {
                    content: Some("done".to_string()),
                    tool_calls: vec![],
                },
            ]),
            seen: seen.clone(),
        };
        let tools = FakeTools {
            calls: Mutex::new(Vec::new()),
        };
        let sink = RecordingSink {
            steps: Mutex::new(Vec::new()),
        };
        let outcome = run_turn_with(&agent, vec![], "q", None, &llm, &tools, &sink)
            .await
            .expect("turn");
        assert_eq!(outcome.text, "done");
        assert_eq!(outcome.steps_executed, 1);
        // Second LLM call saw the assistant(tool_calls) + tool messages.
        let seen = seen.lock().unwrap_or_else(|e| e.into_inner());
        assert!(seen[1] > seen[0]);
        let steps = sink.steps.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(steps.len(), 1);
    }

    #[tokio::test]
    async fn step_cap_truncates_with_partial_text() {
        let mut agent = test_agent();
        agent.agent.max_tool_steps = 1;
        let llm = ScriptLlm {
            replies: Mutex::new(vec![
                tool_reply("c1", "tavily_search"),
                tool_reply("c2", "tavily_search"),
                AssistantReply {
                    content: Some("late".to_string()),
                    tool_calls: vec![],
                },
            ]),
        };
        let tools = FakeTools {
            calls: Mutex::new(Vec::new()),
        };
        let outcome = run_turn_with(&agent, vec![], "q", None, &llm, &tools, &NoopSink)
            .await
            .expect("turn");
        assert!(outcome.truncated_by_step_cap);
        assert_eq!(outcome.steps_executed, 1);
    }

    #[tokio::test]
    async fn tool_budget_refuses_repeated_calls() {
        let mut agent = test_agent();
        agent.agent.max_tool_steps = 8;
        agent.agent.tool_budget_per_tool = 1;
        let llm = ScriptLlm {
            replies: Mutex::new(vec![
                tool_reply("c1", "tavily_search"),
                tool_reply("c2", "tavily_search"),
                AssistantReply {
                    content: Some("summary".to_string()),
                    tool_calls: vec![],
                },
            ]),
        };
        let tools = FakeTools {
            calls: Mutex::new(Vec::new()),
        };
        let sink = RecordingSink {
            steps: Mutex::new(Vec::new()),
        };
        let outcome = run_turn_with(&agent, vec![], "q", None, &llm, &tools, &sink)
            .await
            .expect("turn");
        assert_eq!(outcome.text, "summary");
        // Second call refused: only one real execution.
        let calls = tools.calls.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(calls.len(), 1);
        let steps = sink.steps.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(steps.len(), 2);
        assert!(steps[1].summary.contains("budget reached"));
    }

    #[tokio::test]
    async fn streamed_tokens_reach_the_sink() {
        let agent = test_agent();
        struct TokenLlm;
        impl LlmBackend for TokenLlm {
            async fn chat(
                &self,
                _messages: &[ChatMessage],
                _tools: &[ToolDefinition],
                stream: Option<&dyn crate::llm_client::StreamSink>,
            ) -> Result<AssistantReply, String> {
                let sink = stream.expect("stream sink");
                sink.on_token("Hel");
                sink.on_token("lo");
                Ok(AssistantReply {
                    content: Some("Hello".to_string()),
                    tool_calls: vec![],
                })
            }
        }
        struct TokenSink {
            tokens: Mutex<Vec<String>>,
        }
        impl StepSink for TokenSink {
            fn on_step(&self, _step: ToolStep) {}
            fn on_token(&self, delta: &str) {
                self.tokens
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(delta.to_string());
            }
        }
        let tools = FakeTools {
            calls: Mutex::new(Vec::new()),
        };
        let sink = TokenSink {
            tokens: Mutex::new(Vec::new()),
        };
        let outcome = run_turn_with(&agent, vec![], "q", None, &TokenLlm, &tools, &sink)
            .await
            .expect("turn");
        assert_eq!(outcome.text, "Hello");
        let tokens = sink.tokens.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(tokens.concat(), "Hello");
    }
}
