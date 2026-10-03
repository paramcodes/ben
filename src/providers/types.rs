use std::pin::Pin;

use futures_util::Stream;
use tokio_util::sync::CancellationToken;

use crate::agent::message::{Message, ToolCallId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSpecification {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub id: ToolCallId,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpecification>,
    pub max_output_tokens: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionReason {
    EndTurn,
    ToolCalls,
    LengthLimit,
    ContentFilter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderEvent {
    TextDelta(String),
    ToolCallStarted { id: ToolCallId, name: String },
    ToolCallArgumentsDelta { id: ToolCallId, delta: String },
    ToolCallCompleted(ToolCall),
    Usage(Usage),
    Completed(CompletionReason),
}

impl ProviderEvent {
    pub fn call_id(&self) -> Option<&ToolCallId> {
        match self {
            Self::ToolCallStarted { id, .. } | Self::ToolCallArgumentsDelta { id, .. } => Some(id),
            Self::ToolCallCompleted(call) => Some(&call.id),
            Self::TextDelta(_) | Self::Usage(_) | Self::Completed(_) => None,
        }
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ProviderError {
    #[error("provider operation was cancelled")]
    Cancelled,
    #[error("provider transport failed")]
    Transport,
    #[error("provider request timed out")]
    Timeout,
    #[error("provider returned HTTP status {status}")]
    HttpStatus { status: u16 },
    #[error("provider returned API error (HTTP {status})")]
    Api { status: u16, code: Option<String> },
    #[error("provider request could not be constructed")]
    InvalidRequest,
    #[error("provider returned an invalid response")]
    InvalidResponse,
    #[error("provider rejected the request")]
    Rejected,
}

pub type ProviderStream =
    Pin<Box<dyn Stream<Item = Result<ProviderEvent, ProviderError>> + Send + 'static>>;

pub type ProviderCancellation = CancellationToken;

pub trait Provider: Send + Sync {
    fn stream(
        &self,
        request: ProviderRequest,
        cancellation: ProviderCancellation,
    ) -> ProviderStream;
}

#[cfg(test)]
mod tests {
    use super::{ProviderEvent, ToolCall, Usage};
    use crate::agent::message::ToolCallId;

    #[test]
    fn tool_stream_events_preserve_call_identity_and_order() {
        let id = ToolCallId::new("call-7");
        let events = [
            ProviderEvent::ToolCallStarted {
                id: id.clone(),
                name: "read_file".into(),
            },
            ProviderEvent::ToolCallArgumentsDelta {
                id: id.clone(),
                delta: "{\"path\":".into(),
            },
            ProviderEvent::ToolCallCompleted(ToolCall {
                id: id.clone(),
                name: "read_file".into(),
                arguments: "{\"path\":\"src/main.rs\"}".into(),
            }),
            ProviderEvent::Usage(Usage {
                input_tokens: 20,
                output_tokens: 8,
            }),
            ProviderEvent::Completed(super::CompletionReason::ToolCalls),
        ];

        assert!(matches!(events[0], ProviderEvent::ToolCallStarted { .. }));
        assert!(matches!(
            events[1],
            ProviderEvent::ToolCallArgumentsDelta { .. }
        ));
        assert!(matches!(events[2], ProviderEvent::ToolCallCompleted(_)));
        assert!(matches!(events[3], ProviderEvent::Usage(_)));
        assert!(matches!(events[4], ProviderEvent::Completed(_)));
        assert_eq!(events[0].call_id(), Some(&id));
        assert_eq!(events[1].call_id(), Some(&id));
        assert_eq!(events[2].call_id(), Some(&id));
    }
}
