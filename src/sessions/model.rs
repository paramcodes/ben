//! Versioned, serializable session records.
//!
//! These types are the only durable form of a conversation. They deliberately
//! have no field for an API key, a provider stream, or terminal state: the
//! record is built from messages and bounded tool summaries, so nothing else
//! can reach the file.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::agent::message::{Message, MessageRole, ToolCallId};

/// Schema version written into every new record. Loads of other versions fail
/// until a migration is written deliberately.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum SessionError {
    #[error(
        "session schema version {found} is not supported (this build reads version {supported})"
    )]
    UnsupportedVersion { found: u32, supported: u32 },
    #[error("session id is empty")]
    InvalidId,
}

/// Wall-clock milliseconds since the Unix epoch, or `None` on a clock set
/// before 1970.
pub fn now_ms() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
}

/// One durable conversation entry. System messages are absent by design: the
/// workspace context is rebuilt on every request rather than persisted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum SessionMessage {
    User {
        text: String,
    },
    Assistant {
        text: String,
    },
    ToolCall {
        call_id: String,
        tool: String,
        arguments: String,
    },
    ToolResult {
        call_id: String,
        content: String,
    },
}

/// What a tool call did, without the tool's output payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCallSummary {
    pub call_id: String,
    pub tool: String,
    pub outcome: ToolOutcome,
    /// Workspace-relative files the call changed, for the changed-file summary.
    pub changed_paths: Vec<String>,
    /// Size of the result before it entered history; the text itself stays in
    /// the message list.
    pub output_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolOutcome {
    Succeeded,
    Failed,
    /// The user declined the action, or policy stopped it before any change.
    Refused,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionRecord {
    pub version: u32,
    pub id: String,
    pub model: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub messages: Vec<SessionMessage>,
    pub tool_calls: Vec<ToolCallSummary>,
}

impl SessionRecord {
    pub fn new(
        id: impl Into<String>,
        model: impl Into<String>,
        at_ms: u64,
        messages: Vec<SessionMessage>,
        tool_calls: Vec<ToolCallSummary>,
    ) -> Result<Self, SessionError> {
        let record = Self {
            version: SCHEMA_VERSION,
            id: id.into(),
            model: model.into(),
            created_at_ms: at_ms,
            updated_at_ms: at_ms,
            messages,
            tool_calls,
        };
        record.validate()?;
        Ok(record)
    }

    /// Rejects records this build cannot read instead of guessing at their shape.
    pub fn validate(&self) -> Result<(), SessionError> {
        if self.version != SCHEMA_VERSION {
            return Err(SessionError::UnsupportedVersion {
                found: self.version,
                supported: SCHEMA_VERSION,
            });
        }
        if self.id.trim().is_empty() {
            return Err(SessionError::InvalidId);
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }
}

/// Keeps the durable entries of an agent history and drops system context.
pub fn session_messages(history: &[Message]) -> Vec<SessionMessage> {
    history
        .iter()
        .filter_map(|message| match message.role {
            MessageRole::System => None,
            MessageRole::User => Some(SessionMessage::User {
                text: message.content.clone(),
            }),
            MessageRole::Assistant => Some(SessionMessage::Assistant {
                text: message.content.clone(),
            }),
            MessageRole::ToolCall => Some(SessionMessage::ToolCall {
                call_id: call_id(message),
                tool: message.tool_name.clone().unwrap_or_default(),
                arguments: message.tool_arguments.clone().unwrap_or_default(),
            }),
            MessageRole::Tool => Some(SessionMessage::ToolResult {
                call_id: call_id(message),
                content: message.content.clone(),
            }),
        })
        .collect()
}

impl SessionMessage {
    /// Rebuilds an agent message so a resumed turn can reuse the saved history.
    pub fn to_message(&self) -> Message {
        match self {
            Self::User { text } => Message::user(text.clone()),
            Self::Assistant { text } => Message::assistant(text.clone()),
            Self::ToolCall {
                call_id,
                tool,
                arguments,
            } => Message::assistant_tool_call(
                ToolCallId::new(call_id.clone()),
                tool.clone(),
                arguments.clone(),
            ),
            Self::ToolResult { call_id, content } => {
                Message::tool_result(ToolCallId::new(call_id.clone()), content.clone())
            }
        }
    }
}

fn call_id(message: &Message) -> String {
    message
        .tool_call_id
        .as_ref()
        .map_or_else(String::new, |id| id.as_str().to_owned())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{
        SCHEMA_VERSION, SessionError, SessionMessage, SessionRecord, ToolCallSummary, ToolOutcome,
        session_messages,
    };
    use crate::{
        agent::message::{Message, MessageRole, ToolCallId},
        config::Config,
    };

    fn history() -> Vec<Message> {
        vec![
            Message::system("workspace context that is rebuilt every request"),
            Message::user("update the notes"),
            Message::assistant_tool_call(
                ToolCallId::new("call-1"),
                "propose_edit",
                r#"{"path":"notes.txt"}"#,
            ),
            Message::tool_result(ToolCallId::new("call-1"), r#"{"path":"notes.txt"}"#),
            Message::assistant("updated notes.txt"),
        ]
    }

    fn summaries() -> Vec<ToolCallSummary> {
        vec![ToolCallSummary {
            call_id: "call-1".into(),
            tool: "propose_edit".into(),
            outcome: ToolOutcome::Succeeded,
            changed_paths: vec!["notes.txt".into()],
            output_bytes: 20,
        }]
    }

    fn record() -> SessionRecord {
        SessionRecord::new(
            "session-1",
            "test-model",
            1_700_000_000_000,
            session_messages(&history()),
            summaries(),
        )
        .unwrap()
    }

    #[test]
    fn session_record_round_trips_through_json() {
        let original = record();
        let text = original.to_json().unwrap();

        let loaded = SessionRecord::from_json(&text).unwrap();

        assert_eq!(loaded, original);
        assert_eq!(loaded.version, SCHEMA_VERSION);
        assert_eq!(loaded.messages.len(), 4);
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["version"], 1);
        assert_eq!(value["id"], "session-1");
        assert_eq!(value["created_at_ms"], 1_700_000_000_000u64);
        assert_eq!(value["messages"][0]["role"], "user");
        assert_eq!(value["tool_calls"][0]["outcome"], "succeeded");
    }

    #[test]
    fn system_context_is_rebuilt_rather_than_persisted() {
        let messages = session_messages(&history());

        assert!(!messages.iter().any(|message| matches!(
            message,
            SessionMessage::User { text } if text.contains("workspace context")
        )));
        assert_eq!(messages.len(), 4);
        assert_eq!(
            messages[0],
            SessionMessage::User {
                text: "update the notes".into()
            }
        );
        assert_eq!(session_messages(&history()), messages);
    }

    #[test]
    fn saved_messages_restore_agent_history_for_a_resumed_turn() {
        let restored: Vec<Message> = record()
            .messages
            .iter()
            .map(SessionMessage::to_message)
            .collect();

        assert_eq!(restored.len(), 4);
        assert_eq!(restored[0], Message::user("update the notes"));
        assert_eq!(restored[1].role, MessageRole::ToolCall);
        assert_eq!(restored[1].tool_call_id, Some(ToolCallId::new("call-1")));
        assert_eq!(restored[2].role, MessageRole::Tool);
        assert_eq!(restored[3], Message::assistant("updated notes.txt"));
    }

    #[test]
    fn serialized_record_never_contains_the_configured_secret() {
        let sentinel = "sk-session-secret-must-not-be-stored";
        let mut values = HashMap::new();
        values.insert("OPENAI_API_KEY".to_owned(), sentinel.to_owned());
        values.insert("BEN_MODEL".to_owned(), "test-model".to_owned());
        values.insert("BEN_DATA_DIR".to_owned(), "/srv/ben-data".to_owned());
        let config = Config::from_values(&values, None).unwrap();

        let saved = SessionRecord::new(
            "session-secret",
            config.model.clone(),
            1_700_000_000_000,
            session_messages(&history()),
            summaries(),
        )
        .unwrap();
        let text = saved.to_json().unwrap();

        assert!(!text.contains(sentinel), "session file leaked the API key");
        assert!(!text.contains("api_key"), "{text}");
        assert!(!text.contains(config.api_key().expose_secret()), "{text}");
        let document = serde_json::from_str::<serde_json::Value>(&text).unwrap();
        let mut keys: Vec<&str> = document
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "created_at_ms",
                "id",
                "messages",
                "model",
                "tool_calls",
                "updated_at_ms",
                "version"
            ],
            "a saved session must expose no field beyond the durable schema"
        );
    }

    #[test]
    fn unsupported_schema_version_is_rejected_on_load() {
        let text = r#"{"version":99,"id":"session-1","model":"test-model",
            "created_at_ms":0,"updated_at_ms":0,"messages":[],"tool_calls":[]}"#;

        let text = text.replace('\n', "");
        let loaded = SessionRecord::from_json(&text).unwrap();

        assert_eq!(
            loaded.validate(),
            Err(SessionError::UnsupportedVersion {
                found: 99,
                supported: SCHEMA_VERSION
            })
        );
    }

    #[test]
    fn records_need_an_id() {
        assert_eq!(
            SessionRecord::new("  ", "test-model", 0, Vec::new(), Vec::new()),
            Err(SessionError::InvalidId)
        );
    }

    #[test]
    fn unknown_fields_are_refused_instead_of_dropped() {
        let text = r#"{"version":1,"id":"session-1","model":"test-model","created_at_ms":0,
            "updated_at_ms":0,"messages":[],"tool_calls":[],"api_key":"sk-leak"}"#;

        assert!(SessionRecord::from_json(&text.replace('\n', "")).is_err());
    }
}
