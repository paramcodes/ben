use futures_util::future::BoxFuture;

use crate::agent::message::ToolCallId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolMetadata {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolRequest {
    pub call_id: ToolCallId,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub call_id: ToolCallId,
    pub content: String,
    pub is_error: bool,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ToolExecutionError {
    #[error("tool arguments are invalid")]
    InvalidArguments,
    #[error("tool execution failed")]
    Failed,
}

pub trait Tool: Send + Sync {
    fn metadata(&self) -> ToolMetadata;

    fn execute<'a>(
        &'a self,
        request: ToolRequest,
    ) -> BoxFuture<'a, Result<ToolResult, ToolExecutionError>>;
}
