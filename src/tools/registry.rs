use std::{collections::HashMap, sync::Arc};

use crate::tools::types::{Tool, ToolExecutionError, ToolMetadata, ToolRequest, ToolResult};

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ToolRegistryError {
    #[error("tool name is empty or its schema is invalid")]
    InvalidMetadata,
    #[error("a tool with this name is already registered")]
    DuplicateName,
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
        let registered =
            self.tools
                .get(&request.name)
                .ok_or_else(|| ToolRegistryError::UnknownTool {
                    name: request.name.clone(),
                })?;
        registered
            .tool
            .execute(request)
            .await
            .map_err(ToolRegistryError::Execution)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use futures_util::future::BoxFuture;

    use super::{Tool, ToolMetadata, ToolRegistry, ToolRegistryError, ToolRequest, ToolResult};
    use crate::{agent::message::ToolCallId, tools::ToolExecutionError};

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
}
