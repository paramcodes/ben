//! Durable session records and their local store.
//!
//! A session file is the conversation that survives a restart. It never holds
//! credentials, live provider streams, or terminal state.

pub mod model;
pub mod store;

pub use model::{
    SCHEMA_VERSION, SessionError, SessionMessage, SessionRecord, ToolCallSummary, ToolOutcome,
    now_ms, session_messages,
};
pub use store::{SessionStore, SessionStoreError};
