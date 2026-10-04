//! Durable session records and, later, their local store.
//!
//! A session file is the conversation that survives a restart. It never holds
//! credentials, live provider streams, or terminal state.

pub mod model;

pub use model::{
    SCHEMA_VERSION, SessionError, SessionMessage, SessionRecord, ToolCallSummary, ToolOutcome,
    now_ms, session_messages,
};
