use std::sync::Arc;

use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    agent::{limits::AgentLimits, message::Message},
    app::event::AppEvent,
    providers::types::{
        CompletionReason, Provider, ProviderError, ProviderEvent, ProviderRequest,
        ToolSpecification,
    },
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
    provider: Arc<dyn Provider>,
    model: String,
    tools: Vec<ToolSpecification>,
    limits: AgentLimits,
    history: Vec<Message>,
    turns_started: usize,
}

impl Agent {
    pub fn new(
        provider: Arc<dyn Provider>,
        model: impl Into<String>,
        tools: Vec<ToolSpecification>,
        limits: AgentLimits,
    ) -> Self {
        Self {
            provider,
            model: model.into(),
            tools,
            limits,
            history: Vec::new(),
            turns_started: 0,
        }
    }

    pub fn history(&self) -> &[Message] {
        &self.history
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
        let request_messages = if self.limits.max_history_messages == 0 {
            vec![user_message]
        } else {
            self.push_history(user_message);
            self.history.clone()
        };
        let request = ProviderRequest {
            model: self.model.clone(),
            messages: request_messages,
            tools: self.tools.clone(),
            max_output_tokens: self.limits.max_output_tokens,
        };
        if sender.send(Ok(AppEvent::ProviderStarted)).await.is_err() {
            return StopReason::ChannelClosed;
        }

        let mut stream = self.provider.stream(request, cancellation.clone());
        let mut text = String::new();
        let mut tool_calls = 0usize;
        loop {
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => {
                    let _ = sender.send(Ok(AppEvent::ProviderCancelled)).await;
                    return StopReason::Cancelled;
                }
                item = stream.next() => match item {
                    Some(Ok(event)) => {
                        if matches!(event, ProviderEvent::ToolCallStarted { .. }) {
                            tool_calls = tool_calls.saturating_add(1);
                            if tool_calls > self.limits.max_tool_calls {
                                return StopReason::MaxToolCalls;
                            }
                        }
                        if let ProviderEvent::TextDelta(delta) = &event {
                            text.push_str(delta);
                        }
                        let completed = match event {
                            ProviderEvent::Completed(reason) => Some(reason),
                            _ => None,
                        };
                        if sender.send(Ok(AppEvent::ProviderEvent(event))).await.is_err() {
                            return StopReason::ChannelClosed;
                        }
                        if let Some(reason) = completed {
                            let had_text = !text.is_empty();
                            if had_text {
                                self.push_history(Message::assistant(text));
                            }
                            return if reason == CompletionReason::ToolCalls {
                                StopReason::ToolCalls
                            } else if !had_text {
                                StopReason::EmptyResponse
                            } else {
                                StopReason::Completed(reason)
                            };
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use super::{Agent, StopReason};
    use crate::{
        agent::limits::AgentLimits,
        app::event::AppEvent,
        providers::{
            fake::FakeProvider,
            types::{CompletionReason, ProviderError, ProviderEvent},
        },
    };

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
}
