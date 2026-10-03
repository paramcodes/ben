use serde::Serialize;

use crate::{
    agent::message::MessageRole,
    providers::types::{ProviderRequest, ToolSpecification},
};

#[derive(Debug, thiserror::Error)]
pub enum RequestMappingError {
    #[error("tool result is missing its call identity")]
    MissingToolCallId,
    #[error("request could not be serialized")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Serialize)]
struct ResponsesRequest {
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    input: Vec<InputItem>,
    tools: Vec<FunctionTool>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum InputItem {
    Message {
        role: &'static str,
        content: String,
    },
    FunctionCallOutput {
        #[serde(rename = "type")]
        item_type: &'static str,
        call_id: String,
        output: String,
    },
}

#[derive(Serialize)]
struct FunctionTool {
    #[serde(rename = "type")]
    tool_type: &'static str,
    name: String,
    description: String,
    parameters: serde_json::Value,
}

pub fn map_request(request: &ProviderRequest) -> Result<serde_json::Value, RequestMappingError> {
    let mut instructions = Vec::new();
    let mut input = Vec::new();
    for message in &request.messages {
        match message.role {
            MessageRole::System => instructions.push(message.content.as_str()),
            MessageRole::User => input.push(InputItem::Message {
                role: "user",
                content: message.content.clone(),
            }),
            MessageRole::Assistant => input.push(InputItem::Message {
                role: "assistant",
                content: message.content.clone(),
            }),
            MessageRole::Tool => {
                let call_id = message
                    .tool_call_id
                    .as_ref()
                    .ok_or(RequestMappingError::MissingToolCallId)?;
                input.push(InputItem::FunctionCallOutput {
                    item_type: "function_call_output",
                    call_id: call_id.as_str().to_owned(),
                    output: message.content.clone(),
                });
            }
        }
    }

    let body = ResponsesRequest {
        model: request.model.clone(),
        instructions: (!instructions.is_empty()).then(|| instructions.join("\n\n")),
        input,
        tools: request.tools.iter().map(map_tool).collect(),
        stream: true,
        max_output_tokens: request.max_output_tokens,
    };
    Ok(serde_json::to_value(body)?)
}

fn map_tool(tool: &ToolSpecification) -> FunctionTool {
    FunctionTool {
        tool_type: "function",
        name: tool.name.clone(),
        description: tool.description.clone(),
        parameters: tool.input_schema.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::map_request;
    use crate::{
        agent::message::Message,
        providers::types::{ProviderRequest, ToolSpecification},
    };
    use serde_json::{Value, json};

    fn request(messages: Vec<Message>, tools: Vec<ToolSpecification>) -> ProviderRequest {
        ProviderRequest {
            model: "gpt-4.1".into(),
            messages,
            tools,
            max_output_tokens: Some(256),
        }
    }

    #[test]
    fn maps_text_input_and_instructions_to_responses_json() {
        let request = request(
            vec![Message::system("Be concise."), Message::user("Say hello.")],
            vec![],
        );

        let mapped = map_request(&request).unwrap();

        assert_eq!(
            mapped,
            json!({
                "model": "gpt-4.1",
                "instructions": "Be concise.",
                "input": [{"role": "user", "content": "Say hello."}],
                "tools": [],
                "stream": true,
                "max_output_tokens": 256
            })
        );
    }

    #[test]
    fn maps_function_tool_schemas_to_responses_format() {
        let tools = vec![
            ToolSpecification {
                name: "list_files".into(),
                description: "List workspace files".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "required": ["path"],
                    "additionalProperties": false
                }),
            },
            ToolSpecification {
                name: "read_file".into(),
                description: "Read a workspace file".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "required": ["path"],
                    "additionalProperties": false
                }),
            },
        ];

        let mapped = map_request(&request(vec![Message::user("inspect files")], tools)).unwrap();

        assert_eq!(
            mapped["tools"],
            json!([
                {
                    "type": "function",
                    "name": "list_files",
                    "description": "List workspace files",
                    "parameters": {
                        "type": "object",
                        "properties": {"path": {"type": "string"}},
                        "required": ["path"],
                        "additionalProperties": false
                    }
                },
                {
                    "type": "function",
                    "name": "read_file",
                    "description": "Read a workspace file",
                    "parameters": {
                        "type": "object",
                        "properties": {"path": {"type": "string"}},
                        "required": ["path"],
                        "additionalProperties": false
                    }
                }
            ])
        );
    }

    #[test]
    fn maps_tool_results_with_the_original_call_id() {
        let request = request(
            vec![Message::tool_result(
                crate::agent::message::ToolCallId::new("call-1"),
                "file contents",
            )],
            vec![],
        );

        let mapped = map_request(&request).unwrap();

        assert_eq!(
            mapped["input"],
            json!([{
                "type": "function_call_output",
                "call_id": "call-1",
                "output": "file contents"
            }])
        );
    }

    #[test]
    fn mapped_payload_is_a_json_object() {
        let mapped = map_request(&request(vec![Message::user("hello")], vec![])).unwrap();

        assert!(matches!(mapped, Value::Object(_)));
    }
}
