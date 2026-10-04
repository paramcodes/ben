#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    System,
    User,
    Assistant,
    ToolCall,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ToolCallId(String);

impl ToolCallId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub role: MessageRole,
    pub content: String,
    pub tool_call_id: Option<ToolCallId>,
    pub tool_name: Option<String>,
    pub tool_arguments: Option<String>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self::new(MessageRole::System, content, None)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::new(MessageRole::User, content, None)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(MessageRole::Assistant, content, None)
    }

    pub fn tool_result(call_id: ToolCallId, content: impl Into<String>) -> Self {
        Self::new(MessageRole::Tool, content, Some(call_id))
    }

    pub fn assistant_tool_call(
        call_id: ToolCallId,
        name: impl Into<String>,
        arguments: impl Into<String>,
    ) -> Self {
        Self {
            role: MessageRole::ToolCall,
            content: String::new(),
            tool_call_id: Some(call_id),
            tool_name: Some(name.into()),
            tool_arguments: Some(arguments.into()),
        }
    }

    fn new(
        role: MessageRole,
        content: impl Into<String>,
        tool_call_id: Option<ToolCallId>,
    ) -> Self {
        Self {
            role,
            content: content.into(),
            tool_call_id,
            tool_name: None,
            tool_arguments: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Message, MessageRole, ToolCallId};

    #[test]
    fn messages_preserve_roles_and_tool_call_identity() {
        let call_id = ToolCallId::new("call-1");
        let user = Message::user("inspect this workspace");
        let tool_result = Message::tool_result(call_id.clone(), "found two files");

        assert_eq!(user.role, MessageRole::User);
        assert_eq!(tool_result.role, MessageRole::Tool);
        assert_eq!(tool_result.tool_call_id, Some(call_id));
        assert_ne!(ToolCallId::new("call-1"), ToolCallId::new("call-2"));
    }
}
