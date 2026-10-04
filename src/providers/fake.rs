use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

use futures_util::stream;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

use super::types::{
    Provider, ProviderCancellation, ProviderError, ProviderEvent, ProviderRequest, ProviderStream,
};

/// Deterministic provider for local tests and scripted agent scenarios.
pub struct FakeProvider {
    responses: Vec<Vec<Result<ProviderEvent, ProviderError>>>,
    repeat_last_response: bool,
    response_index: Arc<Mutex<usize>>,
    delay: Duration,
    requests: Arc<Mutex<Vec<ProviderRequest>>>,
}

impl FakeProvider {
    pub fn new(events: Vec<Result<ProviderEvent, ProviderError>>) -> Self {
        Self::with_delay(events, Duration::ZERO)
    }

    pub fn with_delay(events: Vec<Result<ProviderEvent, ProviderError>>, delay: Duration) -> Self {
        Self {
            responses: vec![events],
            repeat_last_response: true,
            response_index: Arc::new(Mutex::new(0)),
            delay,
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Returns one scripted event sequence per provider request.
    pub fn with_responses(responses: Vec<Vec<Result<ProviderEvent, ProviderError>>>) -> Self {
        Self {
            responses,
            repeat_last_response: false,
            response_index: Arc::new(Mutex::new(0)),
            delay: Duration::ZERO,
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn requests(&self) -> Vec<ProviderRequest> {
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl Provider for FakeProvider {
    fn stream(
        &self,
        request: ProviderRequest,
        cancellation: ProviderCancellation,
    ) -> ProviderStream {
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(request);
        let response_number = {
            let mut index = self
                .response_index
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let number = *index;
            *index = index.saturating_add(1);
            number
        };
        let events = self
            .responses
            .get(response_number)
            .or_else(|| {
                self.repeat_last_response
                    .then(|| self.responses.last())
                    .flatten()
            })
            .cloned()
            .unwrap_or_default();
        let state = FakeStreamState {
            events: events.into(),
            delay: self.delay,
            cancellation,
            finished: false,
        };
        Box::pin(stream::unfold(state, |mut state| async move {
            if state.finished {
                return None;
            }
            if state.cancellation.is_cancelled() {
                state.finished = true;
                return Some((Err(ProviderError::Cancelled), state));
            }
            if !state.delay.is_zero() {
                tokio::select! {
                    _ = state.cancellation.cancelled() => {
                        state.finished = true;
                        return Some((Err(ProviderError::Cancelled), state));
                    }
                    _ = sleep(state.delay) => {}
                }
            }
            if state.cancellation.is_cancelled() {
                state.finished = true;
                return Some((Err(ProviderError::Cancelled), state));
            }
            let event = state.events.pop_front()?;
            if event.is_err() {
                state.finished = true;
            }
            Some((event, state))
        }))
    }
}

struct FakeStreamState {
    events: VecDeque<Result<ProviderEvent, ProviderError>>,
    delay: Duration,
    cancellation: CancellationToken,
    finished: bool,
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures_util::StreamExt;
    use tokio_util::sync::CancellationToken;

    use super::FakeProvider;
    use crate::{
        agent::message::ToolCallId,
        providers::types::{
            CompletionReason, Provider, ProviderError, ProviderEvent, ProviderRequest, ToolCall,
        },
    };

    fn request() -> ProviderRequest {
        ProviderRequest {
            model: "fake-model".into(),
            messages: Vec::new(),
            tools: Vec::new(),
            max_output_tokens: Some(32),
        }
    }

    #[tokio::test]
    async fn emits_a_scripted_text_response_and_records_request() {
        let provider = FakeProvider::new(vec![
            Ok(ProviderEvent::TextDelta("hello".into())),
            Ok(ProviderEvent::Completed(CompletionReason::EndTurn)),
        ]);
        let events = provider
            .stream(request(), CancellationToken::new())
            .collect::<Vec<_>>()
            .await;

        assert_eq!(
            events,
            vec![
                Ok(ProviderEvent::TextDelta("hello".into())),
                Ok(ProviderEvent::Completed(CompletionReason::EndTurn)),
            ]
        );
        assert_eq!(provider.requests(), vec![request()]);
    }

    #[tokio::test]
    async fn emits_a_scripted_tool_call_sequence() {
        let id = ToolCallId::new("call-1");
        let provider = FakeProvider::new(vec![
            Ok(ProviderEvent::ToolCallStarted {
                id: id.clone(),
                name: "read_file".into(),
            }),
            Ok(ProviderEvent::ToolCallArgumentsDelta {
                id: id.clone(),
                delta: "{}".into(),
            }),
            Ok(ProviderEvent::ToolCallCompleted(ToolCall {
                id,
                name: "read_file".into(),
                arguments: "{}".into(),
            })),
            Ok(ProviderEvent::Completed(CompletionReason::ToolCalls)),
        ]);
        let events = provider
            .stream(request(), CancellationToken::new())
            .collect::<Vec<_>>()
            .await;

        assert_eq!(events.len(), 4);
        assert!(matches!(events[2], Ok(ProviderEvent::ToolCallCompleted(_))));
        assert_eq!(
            events[3],
            Ok(ProviderEvent::Completed(CompletionReason::ToolCalls))
        );
    }

    #[tokio::test]
    async fn emits_scripted_provider_errors() {
        let provider = FakeProvider::new(vec![Err(ProviderError::Transport)]);
        let events = provider
            .stream(request(), CancellationToken::new())
            .collect::<Vec<_>>()
            .await;

        assert_eq!(events, vec![Err(ProviderError::Transport)]);
    }

    #[tokio::test]
    async fn cancellation_interrupts_delay_and_discards_remaining_events() {
        let provider = FakeProvider::with_delay(
            vec![
                Ok(ProviderEvent::TextDelta("first".into())),
                Ok(ProviderEvent::TextDelta("must not arrive".into())),
            ],
            Duration::from_millis(100),
        );
        let cancellation = CancellationToken::new();
        let mut stream = provider.stream(request(), cancellation.clone());

        assert_eq!(
            stream.next().await,
            Some(Ok(ProviderEvent::TextDelta("first".into())))
        );
        cancellation.cancel();
        assert_eq!(stream.next().await, Some(Err(ProviderError::Cancelled)));
        assert_eq!(stream.next().await, None);
    }
}
