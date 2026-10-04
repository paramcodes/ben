use std::{collections::HashMap, sync::Arc};

use crate::agent::message::ToolCallId;
use crate::policy::approval::{ApprovalResolution, PendingAction};
use crate::tools::types::{Tool, ToolExecutionError, ToolMetadata, ToolRequest, ToolResult};

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ToolRegistryError {
    #[error("tool name is empty or its schema is invalid")]
    InvalidMetadata,
    #[error("a tool with this name is already registered")]
    DuplicateName,
    #[error("tool does not require approval")]
    ApprovalNotRequired,
    #[error("unknown tool: {name}")]
    UnknownTool { name: String },
    #[error(transparent)]
    Execution(#[from] ToolExecutionError),
}

struct RegisteredTool {
    metadata: ToolMetadata,
    tool: Arc<dyn Tool>,
}

#[derive(Default)]
pub struct ToolRegistry {
    tools: HashMap<String, RegisteredTool>,
}

impl ToolRegistry {
    pub fn register(&mut self, tool: Arc<dyn Tool>) -> Result<(), ToolRegistryError> {
        let metadata = tool.metadata();
        if metadata.name.trim().is_empty() || !metadata.input_schema.is_object() {
            return Err(ToolRegistryError::InvalidMetadata);
        }
        if self.tools.contains_key(&metadata.name) {
            return Err(ToolRegistryError::DuplicateName);
        }
        self.tools
            .insert(metadata.name.clone(), RegisteredTool { metadata, tool });
        Ok(())
    }

    /// Returns metadata in name order so provider requests are deterministic.
    pub fn schemas(&self) -> Vec<ToolMetadata> {
        let mut schemas: Vec<_> = self
            .tools
            .values()
            .map(|registered| registered.metadata.clone())
            .collect();
        schemas.sort_by(|left, right| left.name.cmp(&right.name));
        schemas
    }

    pub async fn execute(&self, request: ToolRequest) -> Result<ToolResult, ToolRegistryError> {
        self.tool(&request.name)?
            .tool
            .execute(request)
            .await
            .map_err(ToolRegistryError::Execution)
    }

    /// Executes a request, verifies its call identity, and caps the returned content bytes.
    pub async fn execute_bounded(
        &self,
        request: ToolRequest,
        max_output_bytes: usize,
    ) -> Result<ToolResult, ToolRegistryError> {
        let expected_call_id = request.call_id.clone();
        bound_result(
            self.execute(request).await?,
            expected_call_id,
            max_output_bytes,
        )
    }

    /// Reports whether a tool must be approved before it can change anything.
    pub fn requires_approval(&self, name: &str) -> bool {
        self.tool(name)
            .is_ok_and(|registered| registered.tool.requires_approval())
    }

    /// Builds the exact action the user reviews before an approval-gated tool runs.
    pub fn pending_action(
        &self,
        request: &ToolRequest,
    ) -> Result<PendingAction, ToolRegistryError> {
        let registered = self.tool(&request.name)?;
        if !registered.tool.requires_approval() {
            return Err(ToolRegistryError::ApprovalNotRequired);
        }
        Ok(registered.tool.pending_action(request)?)
    }

    /// Executes an approved action once, revalidating the decision and capping output bytes.
    pub async fn execute_approved(
        &self,
        request: ToolRequest,
        resolution: &ApprovalResolution,
        max_output_bytes: usize,
    ) -> Result<ToolResult, ToolRegistryError> {
        let expected_call_id = request.call_id.clone();
        let registered = self.tool(&request.name)?;
        if !registered.tool.requires_approval() {
            return Err(ToolRegistryError::ApprovalNotRequired);
        }
        let result = registered
            .tool
            .execute_approved(request, resolution)
            .await?;
        bound_result(result, expected_call_id, max_output_bytes)
    }

    fn tool(&self, name: &str) -> Result<&RegisteredTool, ToolRegistryError> {
        self.tools
            .get(name)
            .ok_or_else(|| ToolRegistryError::UnknownTool {
                name: name.to_owned(),
            })
    }
}

fn bound_result(
    mut result: ToolResult,
    expected_call_id: ToolCallId,
    max_output_bytes: usize,
) -> Result<ToolResult, ToolRegistryError> {
    if result.call_id != expected_call_id {
        return Err(ToolExecutionError::Failed.into());
    }
    result.content = truncate_output(&result.content, max_output_bytes);
    Ok(result)
}

fn truncate_output(content: &str, max_bytes: usize) -> String {
    if content.len() <= max_bytes {
        return content.to_owned();
    }
    const MARKER: &str = "[truncated]";
    if max_bytes <= MARKER.len() {
        let mut boundary = max_bytes.min(MARKER.len());
        while !MARKER.is_char_boundary(boundary) {
            boundary -= 1;
        }
        return MARKER[..boundary].to_owned();
    }
    let mut boundary = max_bytes - MARKER.len();
    while !content.is_char_boundary(boundary) {
        boundary -= 1;
    }
    format!("{}{}", &content[..boundary], MARKER)
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use futures_util::future::BoxFuture;

    use super::{Tool, ToolMetadata, ToolRegistry, ToolRegistryError, ToolRequest, ToolResult};
    use crate::{
        agent::message::ToolCallId,
        policy::approval::{ApprovalDecision, ApprovalResolution, PendingAction},
        tools::ToolExecutionError,
    };

    struct EchoTool {
        name: &'static str,
    }

    impl Tool for EchoTool {
        fn metadata(&self) -> ToolMetadata {
            ToolMetadata {
                name: self.name.into(),
                description: format!("Run {}", self.name),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}}
                }),
            }
        }

        fn execute<'a>(
            &'a self,
            request: ToolRequest,
        ) -> BoxFuture<'a, Result<ToolResult, ToolExecutionError>> {
            Box::pin(async move {
                Ok(ToolResult {
                    call_id: request.call_id,
                    content: request.arguments.to_string(),
                    is_error: false,
                })
            })
        }
    }

    /// Stands in for a tool that may only run after an approval decision.
    struct GatedTool {
        executions: Arc<AtomicUsize>,
    }

    impl Tool for GatedTool {
        fn metadata(&self) -> ToolMetadata {
            ToolMetadata {
                name: "gated".into(),
                description: "Runs only after approval".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {"target": {"type": "string"}}
                }),
            }
        }

        fn execute<'a>(
            &'a self,
            _request: ToolRequest,
        ) -> BoxFuture<'a, Result<ToolResult, ToolExecutionError>> {
            Box::pin(async { Err(ToolExecutionError::Failed) })
        }

        fn requires_approval(&self) -> bool {
            true
        }

        fn pending_action(
            &self,
            request: &ToolRequest,
        ) -> Result<PendingAction, ToolExecutionError> {
            PendingAction::new("gated", request.arguments.clone())
                .map_err(|_| ToolExecutionError::Refused("invalid action".to_owned()))
        }

        fn execute_approved<'a>(
            &'a self,
            request: ToolRequest,
            resolution: &'a ApprovalResolution,
        ) -> BoxFuture<'a, Result<ToolResult, ToolExecutionError>> {
            Box::pin(async move {
                if resolution.decision != ApprovalDecision::ApproveOnce {
                    return Err(ToolExecutionError::Refused("not approved".to_owned()));
                }
                self.executions.fetch_add(1, Ordering::SeqCst);
                Ok(ToolResult {
                    call_id: request.call_id,
                    content: "approved output".into(),
                    is_error: false,
                })
            })
        }
    }

    fn gated_request(name: &str) -> ToolRequest {
        ToolRequest {
            call_id: ToolCallId::new("call-gated"),
            name: name.into(),
            arguments: serde_json::json!({ "target": "notes.txt" }),
        }
    }

    fn decision(action: &PendingAction, decision: ApprovalDecision) -> ApprovalResolution {
        ApprovalResolution {
            action: action.clone(),
            decision,
        }
    }

    fn tool(name: &'static str) -> Arc<dyn Tool> {
        Arc::new(EchoTool { name })
    }

    #[test]
    fn registers_tools_and_lists_schemas_deterministically() {
        let mut registry = ToolRegistry::default();
        registry.register(tool("z_tool")).unwrap();
        registry.register(tool("a_tool")).unwrap();

        let schemas = registry.schemas();

        assert_eq!(
            schemas
                .iter()
                .map(|schema| schema.name.as_str())
                .collect::<Vec<_>>(),
            ["a_tool", "z_tool"]
        );
        assert_eq!(
            schemas[0].input_schema["properties"]["path"]["type"],
            "string"
        );
    }

    #[test]
    fn rejects_duplicate_tool_names() {
        let mut registry = ToolRegistry::default();
        registry.register(tool("same")).unwrap();

        assert_eq!(
            registry.register(tool("same")),
            Err(ToolRegistryError::DuplicateName)
        );
    }

    #[tokio::test]
    async fn unknown_tool_name_returns_structured_error() {
        let registry = ToolRegistry::default();
        let request = ToolRequest {
            call_id: ToolCallId::new("call-1"),
            name: "missing".into(),
            arguments: serde_json::json!({}),
        };

        assert_eq!(
            registry.execute(request).await,
            Err(ToolRegistryError::UnknownTool {
                name: "missing".into()
            })
        );
    }

    #[tokio::test]
    async fn executes_registered_tool_with_typed_request_and_result() {
        let mut registry = ToolRegistry::default();
        registry.register(tool("echo")).unwrap();
        let call_id = ToolCallId::new("call-7");
        let request = ToolRequest {
            call_id: call_id.clone(),
            name: "echo".into(),
            arguments: serde_json::json!({"path":"src/main.rs"}),
        };

        let result = registry.execute(request).await.unwrap();

        assert_eq!(result.call_id, call_id);
        assert_eq!(result.content, r#"{"path":"src/main.rs"}"#);
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn approval_gated_tool_runs_only_through_the_approved_path() {
        let executions = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::default();
        registry
            .register(Arc::new(GatedTool {
                executions: Arc::clone(&executions),
            }))
            .unwrap();
        assert!(registry.requires_approval("gated"));
        let action = registry.pending_action(&gated_request("gated")).unwrap();

        let refused = registry
            .execute_approved(
                gated_request("gated"),
                &decision(&action, ApprovalDecision::Reject),
                1024,
            )
            .await;
        assert_eq!(
            refused,
            Err(ToolRegistryError::Execution(ToolExecutionError::Refused(
                "not approved".into()
            )))
        );
        assert_eq!(executions.load(Ordering::SeqCst), 0);

        let result = registry
            .execute_approved(
                gated_request("gated"),
                &decision(&action, ApprovalDecision::ApproveOnce),
                8,
            )
            .await
            .unwrap();
        assert!(
            result.content.len() <= 8 && result.content.contains("truncat"),
            "approved output must be bounded, got {:?}",
            result.content
        );
        assert_eq!(executions.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn unapproved_execution_of_a_gated_tool_fails_closed() {
        let mut registry = ToolRegistry::default();
        registry
            .register(Arc::new(GatedTool {
                executions: Arc::new(AtomicUsize::new(0)),
            }))
            .unwrap();

        assert_eq!(
            registry.execute(gated_request("gated")).await,
            Err(ToolRegistryError::Execution(ToolExecutionError::Failed))
        );
    }

    #[tokio::test]
    async fn approval_paths_are_rejected_for_read_only_and_unknown_tools() {
        let mut registry = ToolRegistry::default();
        registry.register(tool("echo")).unwrap();

        assert!(!registry.requires_approval("echo"));
        assert!(!registry.requires_approval("missing"));
        assert_eq!(
            registry.pending_action(&gated_request("echo")),
            Err(ToolRegistryError::ApprovalNotRequired)
        );
        let action = PendingAction::new("echo", serde_json::json!({})).unwrap();
        assert_eq!(
            registry
                .execute_approved(
                    gated_request("echo"),
                    &decision(&action, ApprovalDecision::ApproveOnce),
                    1024,
                )
                .await,
            Err(ToolRegistryError::ApprovalNotRequired)
        );
        assert_eq!(
            registry.pending_action(&gated_request("missing")),
            Err(ToolRegistryError::UnknownTool {
                name: "missing".into()
            })
        );
    }
}
