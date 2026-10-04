use futures_util::future::BoxFuture;

use crate::agent::message::ToolCallId;
use crate::policy::approval::{ApprovalResolution, PendingAction};

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
    /// Policy reason an approved action could not run; reported back to the model.
    #[error("{0}")]
    Refused(String),
}

pub trait Tool: Send + Sync {
    fn metadata(&self) -> ToolMetadata;

    fn execute<'a>(
        &'a self,
        request: ToolRequest,
    ) -> BoxFuture<'a, Result<ToolResult, ToolExecutionError>>;

    /// Tools that write files or start a process must opt in so the agent gates them.
    fn requires_approval(&self) -> bool {
        false
    }

    /// Builds the exact action the user reviews. Only called for approval-gated tools.
    fn pending_action(&self, request: &ToolRequest) -> Result<PendingAction, ToolExecutionError> {
        let _ = request;
        Err(ToolExecutionError::Failed)
    }

    /// Runs a reviewed action once, revalidating the decision before any side effect.
    fn execute_approved<'a>(
        &'a self,
        request: ToolRequest,
        resolution: &'a ApprovalResolution,
    ) -> BoxFuture<'a, Result<ToolResult, ToolExecutionError>> {
        let _ = (request, resolution);
        Box::pin(async { Err(ToolExecutionError::Failed) })
    }
}
