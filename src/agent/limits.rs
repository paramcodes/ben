/// Resource bounds applied to each agent conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentLimits {
    pub max_turns: usize,
    pub max_tool_calls: usize,
    pub max_history_messages: usize,
    pub max_output_tokens: Option<u32>,
}

impl Default for AgentLimits {
    fn default() -> Self {
        Self {
            max_turns: 32,
            max_tool_calls: 8,
            max_history_messages: 64,
            max_output_tokens: None,
        }
    }
}
