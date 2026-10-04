use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    agent::{
        limits::AgentLimits,
        message::{Message, ToolCallId},
    },
    app::event::AppEvent,
    context::{
        budget::{BoundedContext, ContextItem, TokenEstimator, assemble_workspace_context},
        instructions::{InstructionError, load_instructions},
    },
    providers::retry::{RetryCheckpoint, RetryPolicy, RetryingProvider},
    providers::types::{
        CompletionReason, Provider, ProviderError, ProviderEvent, ProviderRequest, ToolCall,
        ToolSpecification,
    },
    tools::{ToolRegistry, ToolRegistryError, ToolRequest},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    Completed(CompletionReason),
    ToolCalls,
    EmptyResponse,
    MaxTurns,
    MaxToolCalls,
    Cancelled,
    ProviderError(ProviderError),
    InvalidResponse,
    ChannelClosed,
}

pub struct Agent {
    provider: RetryingProvider,
    model: String,
    tools: Vec<ToolSpecification>,
    limits: AgentLimits,
    history: Vec<Message>,
    turns_started: usize,
    side_effect_checkpoint: bool,
    context: Option<BoundedContext>,
    tool_registry: Option<Arc<ToolRegistry>>,
    max_tool_result_bytes: usize,
}

impl Agent {
    pub fn new(
        provider: Arc<dyn Provider>,
        model: impl Into<String>,
        tools: Vec<ToolSpecification>,
        limits: AgentLimits,
    ) -> Self {
        Self {
            provider: RetryingProvider::new(provider, RetryPolicy::default()),
            model: model.into(),
            tools,
            limits,
            history: Vec::new(),
            turns_started: 0,
            side_effect_checkpoint: false,
            context: None,
            tool_registry: None,
            max_tool_result_bytes: 64 * 1024,
        }
    }

    pub fn history(&self) -> &[Message] {
        &self.history
    }

    /// Registers read-only tools and exposes their schemas in provider requests.
    pub fn set_tool_registry(&mut self, registry: Arc<ToolRegistry>, max_result_bytes: usize) {
        self.tools = registry
            .schemas()
            .into_iter()
            .map(|metadata| ToolSpecification {
                name: metadata.name,
                description: metadata.description,
                input_schema: metadata.input_schema,
            })
            .collect();
        self.max_tool_result_bytes = max_result_bytes;
        self.tool_registry = Some(registry);
    }

    /// Adds already bounded repository context to each provider request.
    pub fn set_context(&mut self, context: BoundedContext) {
        self.context = (!context.content.is_empty()).then_some(context);
    }

    /// Loads repository instructions, combines caller-selected task context, and applies a budget.
    pub fn load_workspace_context(
        &mut self,
        workspace_root: impl AsRef<std::path::Path>,
        current_path: impl AsRef<std::path::Path>,
        relevant_context: &[ContextItem],
        token_budget: usize,
        estimator: &dyn TokenEstimator,
    ) -> Result<BoundedContext, InstructionError> {
        let instructions = load_instructions(workspace_root, current_path)?;
        let context =
            assemble_workspace_context(&instructions, relevant_context, token_budget, estimator);
        self.context = Some(context.clone());
        Ok(context)
    }

    /// Marks that an external action completed during this turn.
    pub fn mark_side_effect_completed(&mut self) {
        self.side_effect_checkpoint = true;
    }

    pub async fn run_turn(
        &mut self,
        user_message: impl Into<String>,
        cancellation: CancellationToken,
        sender: mpsc::Sender<std::io::Result<AppEvent>>,
    ) -> StopReason {
        if self.turns_started >= self.limits.max_turns {
            return StopReason::MaxTurns;
        }
        self.turns_started += 1;
        let user_message = Message::user(user_message);
        let mut transient_history = vec![user_message.clone()];
        if self.limits.max_history_messages > 0 {
            self.push_history(user_message);
        }
        let mut tool_calls = 0usize;
        loop {
            let mut request_messages = if self.limits.max_history_messages == 0 {
                transient_history.clone()
            } else {
                self.history.clone()
            };
            if let Some(context) = &self.context {
                request_messages.insert(0, Message::system(context.content.clone()));
            }
            let request = ProviderRequest {
                model: self.model.clone(),
                messages: request_messages,
                tools: self.tools.clone(),
                max_output_tokens: self.limits.max_output_tokens,
            };
            if sender.send(Ok(AppEvent::ProviderStarted)).await.is_err() {
                return StopReason::ChannelClosed;
            }
            let checkpoint = if std::mem::take(&mut self.side_effect_checkpoint) {
                RetryCheckpoint::SideEffectCompleted
            } else {
                RetryCheckpoint::BeforeSideEffect
            };
            let mut stream =
                self.provider
                    .stream_with_checkpoint(request, cancellation.clone(), checkpoint);
            let mut text = String::new();
            let mut started = HashMap::<ToolCallId, String>::new();
            let mut completed_calls = Vec::<ToolCall>::new();
            let reason = loop {
                tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => {
                        let _ = sender.send(Ok(AppEvent::ProviderCancelled)).await;
                        return StopReason::Cancelled;
                    }
                    item = stream.next() => match item {
                        Some(Ok(event)) => {
                            match &event {
                                ProviderEvent::ToolCallStarted { id, name } => {
                                    tool_calls = tool_calls.saturating_add(1);
                                    if tool_calls > self.limits.max_tool_calls {
                                        return StopReason::MaxToolCalls;
                                    }
                                    if self.tool_registry.is_some() && started.insert(id.clone(), name.clone()).is_some() {
                                        return StopReason::InvalidResponse;
                                    }
                                }
                                ProviderEvent::ToolCallArgumentsDelta { id, .. }
                                    if self.tool_registry.is_some() && !started.contains_key(id) =>
                                {
                                    return StopReason::InvalidResponse;
                                }
                                ProviderEvent::ToolCallCompleted(call) if self.tool_registry.is_some() => {
                                    if started.get(&call.id) != Some(&call.name)
                                        || completed_calls.iter().any(|existing| existing.id == call.id)
                                    {
                                        return StopReason::InvalidResponse;
                                    }
                                    completed_calls.push(call.clone());
                                }
                                _ => {}
                            }
                            if let ProviderEvent::TextDelta(delta) = &event {
                                text.push_str(delta);
                            }
                            if let ProviderEvent::Completed(reason) = event {
                                if sender.send(Ok(AppEvent::ProviderEvent(ProviderEvent::Completed(reason)))).await.is_err() {
                                    return StopReason::ChannelClosed;
                                }
                                break reason;
                            }
                            if sender.send(Ok(AppEvent::ProviderEvent(event))).await.is_err() {
                                return StopReason::ChannelClosed;
                            }
                        }
                        Some(Err(ProviderError::Cancelled)) => {
                            let _ = sender.send(Ok(AppEvent::ProviderCancelled)).await;
                            return StopReason::Cancelled;
                        }
                        Some(Err(error)) => {
                            let _ = sender.send(Ok(AppEvent::ProviderFailed(error.clone()))).await;
                            return StopReason::ProviderError(error);
                        }
                        None => {
                            let error = ProviderError::InvalidResponse;
                            let _ = sender.send(Ok(AppEvent::ProviderFailed(error))).await;
                            return StopReason::InvalidResponse;
                        }
                    }
                }
            };
            let had_text = !text.is_empty();
            if reason != CompletionReason::ToolCalls {
                if had_text {
                    self.record_turn_message(&mut transient_history, Message::assistant(text));
                }
                return if !had_text {
                    StopReason::EmptyResponse
                } else {
                    StopReason::Completed(reason)
                };
            }
            let Some(registry) = self.tool_registry.clone() else {
                if had_text {
                    self.record_turn_message(&mut transient_history, Message::assistant(text));
                }
                return StopReason::ToolCalls;
            };
            if completed_calls.is_empty() || completed_calls.len() != started.len() {
                return StopReason::InvalidResponse;
            }
            if had_text {
                self.record_turn_message(&mut transient_history, Message::assistant(text));
            }
            let mut seen = HashSet::new();
            for call in &completed_calls {
                if !seen.insert(call.id.clone()) {
                    return StopReason::InvalidResponse;
                }
                self.record_turn_message(
                    &mut transient_history,
                    Message::assistant_tool_call(
                        call.id.clone(),
                        call.name.clone(),
                        call.arguments.clone(),
                    ),
                );
            }
            for call in completed_calls {
                if sender
                    .send(Ok(AppEvent::ToolStatus(format!("Running {}", call.name))))
                    .await
                    .is_err()
                {
                    return StopReason::ChannelClosed;
                }
                let arguments = serde_json::from_str::<serde_json::Value>(&call.arguments);
                let tool_result = match arguments {
                    Ok(arguments) if arguments.is_object() => {
                        let execution = registry.execute_bounded(
                            ToolRequest {
                                call_id: call.id.clone(),
                                name: call.name.clone(),
                                arguments,
                            },
                            self.max_tool_result_bytes,
                        );
                        tokio::select! {
                            biased;
                            _ = cancellation.cancelled() => {
                                let _ = sender.send(Ok(AppEvent::ProviderCancelled)).await;
                                return StopReason::Cancelled;
                            }
                            result = execution => match result {
                                Ok(result) => result.content,
                                Err(ToolRegistryError::UnknownTool { .. }) => bounded_tool_error("unknown tool", self.max_tool_result_bytes),
                                Err(_) => bounded_tool_error("tool execution failed", self.max_tool_result_bytes),
                            }
                        }
                    }
                    _ => bounded_tool_error("invalid tool arguments", self.max_tool_result_bytes),
                };
                self.record_turn_message(
                    &mut transient_history,
                    Message::tool_result(call.id, tool_result),
                );
            }
        }
    }

    fn record_turn_message(&mut self, transient_history: &mut Vec<Message>, message: Message) {
        if self.limits.max_history_messages == 0 {
            transient_history.push(message);
        } else {
            self.push_history(message);
        }
    }

    fn push_history(&mut self, message: Message) {
        let limit = self.limits.max_history_messages;
        if limit == 0 {
            self.history.clear();
            return;
        }
        self.history.push(message);
        if self.history.len() > limit {
            let excess = self.history.len() - limit;
            self.history.drain(..excess);
        }
    }
}

fn bounded_tool_error(message: &str, max_bytes: usize) -> String {
    let mut boundary = message.len().min(max_bytes);
    while !message.is_char_boundary(boundary) {
        boundary -= 1;
    }
    message[..boundary].to_owned()
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use futures_util::future::BoxFuture;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use super::{Agent, StopReason};
    use crate::{
        agent::message::ToolCallId,
        agent::{limits::AgentLimits, message::MessageRole},
        app::event::AppEvent,
        context::budget::{BoundedContext, ByteFallbackEstimator, ContextItem},
        providers::{
            fake::FakeProvider,
            types::{CompletionReason, ProviderError, ProviderEvent, ToolCall},
        },
        tools::{Tool, ToolExecutionError, ToolMetadata, ToolRegistry, ToolRequest, ToolResult},
    };

    struct FixedTool {
        content: String,
        executions: Arc<AtomicUsize>,
    }

    impl Tool for FixedTool {
        fn metadata(&self) -> ToolMetadata {
            ToolMetadata {
                name: "echo".into(),
                description: "Test tool".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {"value": {"type": "string"}},
                    "required": ["value"],
                    "additionalProperties": false
                }),
            }
        }

        fn execute<'a>(
            &'a self,
            request: ToolRequest,
        ) -> BoxFuture<'a, Result<ToolResult, ToolExecutionError>> {
            Box::pin(async move {
                self.executions.fetch_add(1, Ordering::SeqCst);
                Ok(ToolResult {
                    call_id: request.call_id,
                    content: self.content.clone(),
                    is_error: false,
                })
            })
        }
    }

    fn registry(content: &str, executions: Arc<AtomicUsize>) -> Arc<ToolRegistry> {
        let mut registry = ToolRegistry::default();
        registry
            .register(Arc::new(FixedTool {
                content: content.into(),
                executions,
            }))
            .unwrap();
        Arc::new(registry)
    }

    fn tool_response(
        call_id: &str,
        name: &str,
        arguments: &str,
    ) -> Vec<Result<ProviderEvent, ProviderError>> {
        let id = ToolCallId::new(call_id);
        vec![
            Ok(ProviderEvent::ToolCallStarted {
                id: id.clone(),
                name: name.into(),
            }),
            Ok(ProviderEvent::ToolCallArgumentsDelta {
                id: id.clone(),
                delta: arguments.into(),
            }),
            Ok(ProviderEvent::ToolCallCompleted(ToolCall {
                id,
                name: name.into(),
                arguments: arguments.into(),
            })),
            Ok(ProviderEvent::Completed(CompletionReason::ToolCalls)),
        ]
    }

    fn limits(max_turns: usize, max_tool_calls: usize) -> AgentLimits {
        AgentLimits {
            max_turns,
            max_tool_calls,
            max_history_messages: 8,
            max_output_tokens: Some(64),
        }
    }

    #[tokio::test]
    async fn final_answer_is_forwarded_and_saved_to_history() {
        let provider = Arc::new(FakeProvider::new(vec![
            Ok(ProviderEvent::TextDelta("answer".into())),
            Ok(ProviderEvent::Completed(CompletionReason::EndTurn)),
        ]));
        let mut agent = Agent::new(provider.clone(), "test-model", Vec::new(), limits(2, 3));
        let (sender, mut receiver) = mpsc::channel(8);

        let result = agent
            .run_turn("question", CancellationToken::new(), sender)
            .await;

        assert_eq!(result, StopReason::Completed(CompletionReason::EndTurn));
        assert_eq!(agent.history().len(), 2);
        assert!(matches!(
            receiver.recv().await,
            Some(Ok(AppEvent::ProviderStarted))
        ));
        assert!(
            matches!(receiver.recv().await, Some(Ok(AppEvent::ProviderEvent(ProviderEvent::TextDelta(text)))) if text == "answer")
        );
        assert_eq!(provider.requests()[0].model, "test-model");
    }

    #[tokio::test]
    async fn repository_context_is_sent_as_system_message_without_entering_history() {
        let provider = Arc::new(FakeProvider::new(vec![Ok(ProviderEvent::Completed(
            CompletionReason::EndTurn,
        ))]));
        let mut agent = Agent::new(provider.clone(), "test-model", Vec::new(), limits(2, 3));
        agent.set_context(BoundedContext {
            content: "Repository instructions\nBudget notice: omitted README.md".into(),
            ..BoundedContext::default()
        });
        let (sender, _receiver) = mpsc::channel(8);

        let _ = agent
            .run_turn("question", CancellationToken::new(), sender)
            .await;

        let messages = &provider.requests()[0].messages;
        assert_eq!(messages[0].role, MessageRole::System);
        assert!(messages[0].content.contains("omitted README.md"));
        assert_eq!(messages[1].role, MessageRole::User);
        assert_eq!(agent.history().len(), 1);
    }

    #[tokio::test]
    async fn loads_workspace_instructions_and_task_context_under_a_budget() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("AGENTS.md"),
            "Keep edits within the workspace.",
        )
        .unwrap();
        let provider = Arc::new(FakeProvider::new(vec![Ok(ProviderEvent::Completed(
            CompletionReason::EndTurn,
        ))]));
        let mut agent = Agent::new(provider.clone(), "test-model", Vec::new(), limits(2, 3));
        let context = agent
            .load_workspace_context(
                root.path(),
                ".",
                &[ContextItem {
                    source: "Current task".into(),
                    content: "Inspect src/lib.rs".into(),
                }],
                10_000,
                &ByteFallbackEstimator,
            )
            .unwrap();
        assert_eq!(context.included_sources.len(), 2);
        let (sender, _receiver) = mpsc::channel(8);
        let _ = agent
            .run_turn("question", CancellationToken::new(), sender)
            .await;

        let system = &provider.requests()[0].messages[0];
        assert_eq!(system.role, MessageRole::System);
        assert!(system.content.contains("Keep edits within the workspace."));
        assert!(system.content.contains("Inspect src/lib.rs"));
    }

    #[tokio::test]
    async fn provider_error_is_forwarded_and_reported() {
        let provider = Arc::new(FakeProvider::new(vec![Err(ProviderError::Transport)]));
        let mut agent = Agent::new(provider, "test-model", Vec::new(), limits(2, 3));
        let (sender, mut receiver) = mpsc::channel(8);

        assert_eq!(
            agent
                .run_turn("question", CancellationToken::new(), sender)
                .await,
            StopReason::ProviderError(ProviderError::Transport)
        );
        assert!(matches!(
            receiver.recv().await,
            Some(Ok(AppEvent::ProviderStarted))
        ));
        assert!(matches!(
            receiver.recv().await,
            Some(Ok(AppEvent::ProviderFailed(ProviderError::Transport)))
        ));
    }

    #[tokio::test]
    async fn empty_completed_response_has_an_explicit_stop_reason() {
        let provider = Arc::new(FakeProvider::new(vec![Ok(ProviderEvent::Completed(
            CompletionReason::EndTurn,
        ))]));
        let mut agent = Agent::new(provider, "test-model", Vec::new(), limits(2, 3));
        let (sender, _receiver) = mpsc::channel(8);

        assert_eq!(
            agent
                .run_turn("question", CancellationToken::new(), sender)
                .await,
            StopReason::EmptyResponse
        );
    }

    #[tokio::test]
    async fn max_turns_prevents_another_provider_request() {
        let provider = Arc::new(FakeProvider::new(vec![Ok(ProviderEvent::Completed(
            CompletionReason::EndTurn,
        ))]));
        let mut agent = Agent::new(provider.clone(), "test-model", Vec::new(), limits(1, 3));
        let (sender, _receiver) = mpsc::channel(8);

        assert_eq!(
            agent
                .run_turn("one", CancellationToken::new(), sender.clone())
                .await,
            StopReason::EmptyResponse
        );
        assert_eq!(
            agent
                .run_turn("two", CancellationToken::new(), sender)
                .await,
            StopReason::MaxTurns
        );
        assert_eq!(provider.requests().len(), 1);
    }

    #[tokio::test]
    async fn history_limit_is_applied_to_each_provider_request() {
        let provider = Arc::new(FakeProvider::new(vec![
            Ok(ProviderEvent::TextDelta("answer".into())),
            Ok(ProviderEvent::Completed(CompletionReason::EndTurn)),
        ]));
        let mut agent = Agent::new(
            provider.clone(),
            "test-model",
            Vec::new(),
            AgentLimits {
                max_history_messages: 1,
                ..limits(3, 3)
            },
        );
        let (sender, _receiver) = mpsc::channel(8);

        assert_eq!(
            agent
                .run_turn("first", CancellationToken::new(), sender.clone())
                .await,
            StopReason::Completed(CompletionReason::EndTurn)
        );
        assert_eq!(
            agent
                .run_turn("second", CancellationToken::new(), sender)
                .await,
            StopReason::Completed(CompletionReason::EndTurn)
        );

        assert_eq!(provider.requests()[1].messages.len(), 1);
        assert_eq!(provider.requests()[1].messages[0].content, "second");
    }

    #[tokio::test]
    async fn too_many_tool_calls_stop_the_current_turn() {
        let provider = Arc::new(FakeProvider::new(vec![
            Ok(ProviderEvent::ToolCallStarted {
                id: crate::agent::message::ToolCallId::new("one"),
                name: "read_file".into(),
            }),
            Ok(ProviderEvent::ToolCallStarted {
                id: crate::agent::message::ToolCallId::new("two"),
                name: "read_file".into(),
            }),
        ]));
        let mut agent = Agent::new(provider, "test-model", Vec::new(), limits(2, 1));
        let (sender, _receiver) = mpsc::channel(8);

        assert_eq!(
            agent
                .run_turn("question", CancellationToken::new(), sender)
                .await,
            StopReason::MaxToolCalls
        );
    }

    #[tokio::test]
    async fn cancellation_stops_fake_provider_and_returns_cancelled_reason() {
        let provider = Arc::new(FakeProvider::with_delay(
            vec![
                Ok(ProviderEvent::TextDelta("partial".into())),
                Ok(ProviderEvent::TextDelta("must not arrive".into())),
                Ok(ProviderEvent::Completed(CompletionReason::EndTurn)),
            ],
            Duration::from_millis(50),
        ));
        let mut agent = Agent::new(provider, "test-model", Vec::new(), limits(2, 3));
        let cancellation = CancellationToken::new();
        let turn_cancellation = cancellation.clone();
        let (sender, mut receiver) = mpsc::channel(8);
        let task =
            tokio::spawn(
                async move { agent.run_turn("question", turn_cancellation, sender).await },
            );

        assert!(matches!(
            receiver.recv().await,
            Some(Ok(AppEvent::ProviderStarted))
        ));
        assert!(matches!(
            receiver.recv().await,
            Some(Ok(AppEvent::ProviderEvent(ProviderEvent::TextDelta(text)))) if text == "partial"
        ));
        cancellation.cancel();
        assert_eq!(task.await.unwrap(), StopReason::Cancelled);
        assert!(matches!(
            receiver.recv().await,
            Some(Ok(AppEvent::ProviderCancelled))
        ));
        assert!(receiver.try_recv().is_err());
    }

    #[tokio::test]
    async fn side_effect_checkpoint_prevents_provider_replay() {
        let provider = Arc::new(FakeProvider::new(vec![Err(ProviderError::Transport)]));
        let mut agent = Agent::new(provider.clone(), "test-model", Vec::new(), limits(2, 3));
        agent.mark_side_effect_completed();
        let (sender, _receiver) = mpsc::channel(8);

        assert_eq!(
            agent
                .run_turn("continue", CancellationToken::new(), sender)
                .await,
            StopReason::ProviderError(ProviderError::Transport)
        );
        assert_eq!(provider.requests().len(), 1);
    }

    #[tokio::test]
    async fn executes_a_registered_tool_and_sends_call_and_result_on_followup_request() {
        let id = "call-read";
        let arguments = r#"{"value":"hello"}"#;
        let provider = Arc::new(FakeProvider::with_responses(vec![
            tool_response(id, "echo", arguments),
            vec![
                Ok(ProviderEvent::TextDelta("done".into())),
                Ok(ProviderEvent::Completed(CompletionReason::EndTurn)),
            ],
        ]));
        let executions = Arc::new(AtomicUsize::new(0));
        let mut agent = Agent::new(provider.clone(), "test-model", Vec::new(), limits(2, 4));
        agent.set_tool_registry(registry("tool result", executions.clone()), 64);
        let (sender, _receiver) = mpsc::channel(32);

        assert_eq!(
            agent
                .run_turn("inspect", CancellationToken::new(), sender)
                .await,
            StopReason::Completed(CompletionReason::EndTurn)
        );

        assert_eq!(executions.load(Ordering::SeqCst), 1);
        let requests = provider.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].tools[0].name, "echo");
        let followup = &requests[1].messages;
        let assistant_call = followup
            .iter()
            .find(|message| message.role == MessageRole::ToolCall)
            .unwrap();
        assert_eq!(assistant_call.tool_call_id.as_ref().unwrap().as_str(), id);
        assert!(
            matches!(followup.iter().find(|message| message.role == MessageRole::Tool), Some(message) if message.content == "tool result" && message.tool_call_id.as_ref().unwrap().as_str() == id)
        );
        assert!(
            agent
                .history()
                .iter()
                .any(|message| message.content == "done")
        );
    }

    #[tokio::test]
    async fn executes_multiple_completed_calls_within_the_configured_limit() {
        let mut first = tool_response("call-one", "echo", r#"{"value":"one"}"#);
        let mut second = tool_response("call-two", "echo", r#"{"value":"two"}"#);
        first.pop();
        second.pop();
        first.extend(second);
        first.push(Ok(ProviderEvent::Completed(CompletionReason::ToolCalls)));
        let provider = Arc::new(FakeProvider::with_responses(vec![
            first,
            vec![Ok(ProviderEvent::Completed(CompletionReason::EndTurn))],
        ]));
        let executions = Arc::new(AtomicUsize::new(0));
        let mut agent = Agent::new(provider.clone(), "test-model", Vec::new(), limits(2, 2));
        agent.set_tool_registry(registry("result", executions.clone()), 64);
        let (sender, _receiver) = mpsc::channel(32);

        assert_eq!(
            agent
                .run_turn("inspect", CancellationToken::new(), sender)
                .await,
            StopReason::EmptyResponse
        );

        assert_eq!(executions.load(Ordering::SeqCst), 2);
        let requests = provider.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[1]
                .messages
                .iter()
                .filter(|message| message.role == MessageRole::ToolCall)
                .count(),
            2
        );
        assert_eq!(
            requests[1]
                .messages
                .iter()
                .filter(|message| message.role == MessageRole::Tool)
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn publishes_registered_list_read_and_search_schemas() {
        let root =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workspace");
        let mut registry = ToolRegistry::default();
        registry
            .register(Arc::new(crate::tools::list_files::ListFilesTool::new(
                &root, 100, 10_000,
            )))
            .unwrap();
        registry
            .register(Arc::new(crate::tools::read_file::ReadFileTool::new(
                &root, 10_000, 100_000,
            )))
            .unwrap();
        registry
            .register(Arc::new(crate::tools::search::SearchTool::new(
                &root, 100, 10_000, 100_000,
            )))
            .unwrap();
        let provider = Arc::new(FakeProvider::new(vec![Ok(ProviderEvent::Completed(
            CompletionReason::EndTurn,
        ))]));
        let mut agent = Agent::new(provider.clone(), "test-model", Vec::new(), limits(2, 3));
        agent.set_tool_registry(Arc::new(registry), 10_000);
        let (sender, _receiver) = mpsc::channel(8);

        let _ = agent
            .run_turn("inspect", CancellationToken::new(), sender)
            .await;

        let requests = provider.requests();
        let tools: Vec<_> = requests[0]
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect();
        assert_eq!(tools, ["list_files", "read_file", "search"]);
    }

    #[tokio::test]
    async fn malformed_arguments_and_unknown_tools_are_not_executed() {
        for (name, arguments, expected_error) in [
            ("echo", "{broken", "invalid tool arguments"),
            ("missing", "{}", "unknown tool"),
        ] {
            let provider = Arc::new(FakeProvider::with_responses(vec![
                tool_response("call-invalid", name, arguments),
                vec![Ok(ProviderEvent::Completed(CompletionReason::EndTurn))],
            ]));
            let executions = Arc::new(AtomicUsize::new(0));
            let mut agent = Agent::new(provider.clone(), "test-model", Vec::new(), limits(2, 4));
            agent.set_tool_registry(registry("unused", executions.clone()), 128);
            let (sender, _receiver) = mpsc::channel(32);

            let _ = agent
                .run_turn("inspect", CancellationToken::new(), sender)
                .await;

            assert_eq!(executions.load(Ordering::SeqCst), 0);
            assert!(
                provider.requests()[1]
                    .messages
                    .iter()
                    .any(|message| message.role == MessageRole::Tool
                        && message.content.contains(expected_error))
            );
        }
    }

    #[tokio::test]
    async fn oversized_tool_output_is_truncated_before_followup_request() {
        let provider = Arc::new(FakeProvider::with_responses(vec![
            tool_response("call-large", "echo", r#"{"value":"x"}"#),
            vec![Ok(ProviderEvent::Completed(CompletionReason::EndTurn))],
        ]));
        let mut agent = Agent::new(provider.clone(), "test-model", Vec::new(), limits(2, 4));
        agent.set_tool_registry(
            registry(&"x".repeat(100), Arc::new(AtomicUsize::new(0))),
            16,
        );
        let (sender, _receiver) = mpsc::channel(32);

        let _ = agent
            .run_turn("inspect", CancellationToken::new(), sender)
            .await;

        let requests = provider.requests();
        let result = requests[1]
            .messages
            .iter()
            .find(|message| message.role == MessageRole::Tool)
            .unwrap();
        assert!(result.content.len() <= 16);
        assert!(result.content.contains("truncated"));
    }

    #[tokio::test]
    async fn mismatched_completed_call_id_is_rejected_without_execution() {
        let started = ToolCallId::new("call-started");
        let events = vec![
            Ok(ProviderEvent::ToolCallStarted {
                id: started.clone(),
                name: "echo".into(),
            }),
            Ok(ProviderEvent::ToolCallCompleted(ToolCall {
                id: ToolCallId::new("call-other"),
                name: "echo".into(),
                arguments: r#"{"value":"x"}"#.into(),
            })),
            Ok(ProviderEvent::Completed(CompletionReason::ToolCalls)),
        ];
        let provider = Arc::new(FakeProvider::with_responses(vec![events]));
        let executions = Arc::new(AtomicUsize::new(0));
        let mut agent = Agent::new(provider.clone(), "test-model", Vec::new(), limits(2, 4));
        agent.set_tool_registry(registry("unused", executions.clone()), 64);
        let (sender, _receiver) = mpsc::channel(32);

        assert_eq!(
            agent
                .run_turn("inspect", CancellationToken::new(), sender)
                .await,
            StopReason::InvalidResponse
        );
        assert_eq!(executions.load(Ordering::SeqCst), 0);
        assert_eq!(provider.requests().len(), 1);
    }
}
