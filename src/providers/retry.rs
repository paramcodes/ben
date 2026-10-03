use std::{sync::Arc, time::Duration};

use futures_util::{StreamExt, stream};
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

use crate::agent::message::MessageRole;

use super::types::{
    Provider, ProviderCancellation, ProviderError, ProviderRequest, ProviderStream,
};

const MAX_RETRIES: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryCheckpoint {
    BeforeSideEffect,
    SideEffectCompleted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    pub max_retries: usize,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 2,
            initial_backoff: Duration::from_millis(25),
            max_backoff: Duration::from_millis(250),
        }
    }
}

pub struct RetryingProvider {
    inner: Arc<dyn Provider>,
    policy: RetryPolicy,
}

impl RetryingProvider {
    pub fn new(inner: Arc<dyn Provider>, policy: RetryPolicy) -> Self {
        Self { inner, policy }
    }

    pub fn stream_with_checkpoint(
        &self,
        request: ProviderRequest,
        cancellation: ProviderCancellation,
        checkpoint: RetryCheckpoint,
    ) -> ProviderStream {
        let checkpoint = if checkpoint == RetryCheckpoint::SideEffectCompleted
            || request
                .messages
                .iter()
                .any(|message| message.role == MessageRole::Tool)
        {
            RetryCheckpoint::SideEffectCompleted
        } else {
            RetryCheckpoint::BeforeSideEffect
        };
        let stream = self.inner.stream(request.clone(), cancellation.clone());
        let state = RetryStreamState {
            inner: Arc::clone(&self.inner),
            request,
            cancellation,
            policy: self.policy,
            checkpoint,
            retries: 0,
            saw_event: false,
            finished: false,
            stream,
        };
        Box::pin(stream::unfold(state, |mut state| async move {
            loop {
                if state.finished {
                    return None;
                }
                let item = tokio::select! {
                    _ = state.cancellation.cancelled() => {
                        state.finished = true;
                        return Some((Err(ProviderError::Cancelled), state));
                    }
                    item = state.stream.next() => item,
                };
                match item {
                    Some(Ok(event)) => {
                        state.saw_event = true;
                        return Some((Ok(event), state));
                    }
                    Some(Err(error))
                        if retryable(&error)
                            && !state.saw_event
                            && state.checkpoint == RetryCheckpoint::BeforeSideEffect
                            && state.retries < state.policy.max_retries.min(MAX_RETRIES) =>
                    {
                        state.retries += 1;
                        let delay = retry_delay(state.policy, state.retries);
                        tokio::select! {
                            _ = state.cancellation.cancelled() => {
                                state.finished = true;
                                return Some((Err(ProviderError::Cancelled), state));
                            }
                            _ = sleep(delay) => {}
                        }
                        state.stream = state
                            .inner
                            .stream(state.request.clone(), state.cancellation.clone());
                    }
                    Some(Err(error)) => {
                        state.finished = true;
                        return Some((Err(error), state));
                    }
                    None => {
                        return None;
                    }
                }
            }
        }))
    }
}

impl Provider for RetryingProvider {
    fn stream(
        &self,
        request: ProviderRequest,
        cancellation: ProviderCancellation,
    ) -> ProviderStream {
        self.stream_with_checkpoint(request, cancellation, RetryCheckpoint::BeforeSideEffect)
    }
}

fn retryable(error: &ProviderError) -> bool {
    matches!(error, ProviderError::Transport | ProviderError::Timeout)
}

fn retry_delay(policy: RetryPolicy, retry_number: usize) -> Duration {
    let multiplier = 1_u32 << retry_number.saturating_sub(1).min(20);
    policy
        .initial_backoff
        .saturating_mul(multiplier)
        .min(policy.max_backoff)
}

struct RetryStreamState {
    inner: Arc<dyn Provider>,
    request: ProviderRequest,
    cancellation: CancellationToken,
    policy: RetryPolicy,
    checkpoint: RetryCheckpoint,
    retries: usize,
    saw_event: bool,
    finished: bool,
    stream: ProviderStream,
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
        time::Duration,
    };

    use futures_util::{StreamExt, stream};
    use tokio_util::sync::CancellationToken;

    use super::{RetryCheckpoint, RetryPolicy, RetryingProvider};
    use crate::{
        agent::message::{Message, ToolCallId},
        providers::types::{
            CompletionReason, Provider, ProviderCancellation, ProviderError, ProviderEvent,
            ProviderRequest, ProviderStream,
        },
    };

    struct SequencedProvider {
        scripts: Mutex<VecDeque<Vec<Result<ProviderEvent, ProviderError>>>>,
        requests: Mutex<Vec<ProviderRequest>>,
    }

    impl SequencedProvider {
        fn new(scripts: Vec<Vec<Result<ProviderEvent, ProviderError>>>) -> Self {
            Self {
                scripts: Mutex::new(scripts.into()),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn request_count(&self) -> usize {
            self.requests.lock().unwrap().len()
        }
    }

    impl Provider for SequencedProvider {
        fn stream(
            &self,
            request: ProviderRequest,
            _cancellation: ProviderCancellation,
        ) -> ProviderStream {
            self.requests.lock().unwrap().push(request);
            let script = self.scripts.lock().unwrap().pop_front().unwrap_or_default();
            Box::pin(stream::iter(script))
        }
    }

    fn request(messages: Vec<Message>) -> ProviderRequest {
        ProviderRequest {
            model: "test-model".into(),
            messages,
            tools: Vec::new(),
            max_output_tokens: None,
        }
    }

    fn policy() -> RetryPolicy {
        RetryPolicy {
            max_retries: 2,
            initial_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(2),
        }
    }

    #[tokio::test]
    async fn retries_transient_connection_failure_then_succeeds() {
        let transport = Arc::new(SequencedProvider::new(vec![
            vec![Err(ProviderError::Transport)],
            vec![
                Ok(ProviderEvent::TextDelta("ok".into())),
                Ok(ProviderEvent::Completed(CompletionReason::EndTurn)),
            ],
        ]));
        let provider = RetryingProvider::new(transport.clone(), policy());
        let events = provider
            .stream_with_checkpoint(
                request(vec![Message::user("hello")]),
                CancellationToken::new(),
                RetryCheckpoint::BeforeSideEffect,
            )
            .collect::<Vec<_>>()
            .await;

        assert_eq!(transport.request_count(), 2);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0], Ok(ProviderEvent::TextDelta("ok".into())));
    }

    #[tokio::test]
    async fn does_not_retry_permanent_provider_error() {
        let transport = Arc::new(SequencedProvider::new(vec![
            vec![Err(ProviderError::Api {
                status: 400,
                code: Some("invalid_request".into()),
            })],
            vec![Ok(ProviderEvent::Completed(CompletionReason::EndTurn))],
        ]));
        let provider = RetryingProvider::new(transport.clone(), policy());
        let events = provider
            .stream_with_checkpoint(
                request(vec![Message::user("hello")]),
                CancellationToken::new(),
                RetryCheckpoint::BeforeSideEffect,
            )
            .collect::<Vec<_>>()
            .await;

        assert_eq!(transport.request_count(), 1);
        assert_eq!(
            events,
            vec![Err(ProviderError::Api {
                status: 400,
                code: Some("invalid_request".into()),
            })]
        );
    }

    #[tokio::test]
    async fn never_replays_request_after_tool_result_checkpoint() {
        let transport = Arc::new(SequencedProvider::new(vec![
            vec![Err(ProviderError::Transport)],
            vec![Ok(ProviderEvent::Completed(CompletionReason::EndTurn))],
        ]));
        let provider = RetryingProvider::new(transport.clone(), policy());
        let request = request(vec![
            Message::user("read file"),
            Message::tool_result(ToolCallId::new("call-1"), "file contents"),
        ]);
        let events = provider
            .stream_with_checkpoint(
                request,
                CancellationToken::new(),
                RetryCheckpoint::SideEffectCompleted,
            )
            .collect::<Vec<_>>()
            .await;

        assert_eq!(transport.request_count(), 1);
        assert_eq!(events, vec![Err(ProviderError::Transport)]);
    }

    #[tokio::test]
    async fn does_not_replay_after_streaming_has_started() {
        let transport = Arc::new(SequencedProvider::new(vec![
            vec![
                Ok(ProviderEvent::TextDelta("partial".into())),
                Err(ProviderError::Transport),
            ],
            vec![Ok(ProviderEvent::Completed(CompletionReason::EndTurn))],
        ]));
        let provider = RetryingProvider::new(transport.clone(), policy());
        let events = provider
            .stream_with_checkpoint(
                request(vec![Message::user("hello")]),
                CancellationToken::new(),
                RetryCheckpoint::BeforeSideEffect,
            )
            .collect::<Vec<_>>()
            .await;

        assert_eq!(transport.request_count(), 1);
        assert_eq!(
            events,
            vec![
                Ok(ProviderEvent::TextDelta("partial".into())),
                Err(ProviderError::Transport),
            ]
        );
    }

    #[tokio::test]
    async fn stops_after_configured_retry_limit() {
        let transport = Arc::new(SequencedProvider::new(vec![
            vec![Err(ProviderError::Transport)],
            vec![Err(ProviderError::Transport)],
            vec![Err(ProviderError::Transport)],
            vec![Ok(ProviderEvent::Completed(CompletionReason::EndTurn))],
        ]));
        let provider = RetryingProvider::new(transport.clone(), policy());
        let events = provider
            .stream_with_checkpoint(
                request(vec![Message::user("hello")]),
                CancellationToken::new(),
                RetryCheckpoint::BeforeSideEffect,
            )
            .collect::<Vec<_>>()
            .await;

        assert_eq!(transport.request_count(), 3);
        assert_eq!(events, vec![Err(ProviderError::Transport)]);
    }
}
