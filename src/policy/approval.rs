use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAction {
    tool: String,
    arguments: Value,
}

impl PendingAction {
    pub fn new(tool: impl Into<String>, arguments: Value) -> Result<Self, ApprovalError> {
        let tool = tool.into().trim().to_owned();
        if tool.is_empty() || !arguments.is_object() {
            return Err(ApprovalError::InvalidAction);
        }
        Ok(Self {
            tool,
            arguments: normalize_value(None, arguments),
        })
    }

    pub fn tool(&self) -> &str {
        &self.tool
    }

    pub fn arguments(&self) -> &Value {
        &self.arguments
    }

    pub fn fingerprint(&self) -> ActionFingerprint {
        let encoded = serde_json::to_vec(&(&self.tool, &self.arguments))
            .expect("serializing a normalized JSON action cannot fail");
        let mut first = DefaultHasher::new();
        first.write(b"ben-action-fingerprint-v1");
        first.write(&encoded);
        let mut second = DefaultHasher::new();
        second.write(b"ben-action-fingerprint-v1-secondary");
        second.write(&encoded);
        ActionFingerprint {
            digest: [first.finish(), second.finish()],
            generation: 0,
        }
    }
}

fn normalize_value(key: Option<&str>, value: Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .map(|(key, value)| {
                    let normalized = normalize_value(Some(&key), value);
                    (key, normalized)
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|value| normalize_value(key, value))
                .collect(),
        ),
        Value::String(value) if key.is_some_and(is_path_argument) => {
            Value::String(normalize_path(&value))
        }
        value => value,
    }
}

fn is_path_argument(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "path" | "cwd" | "working_dir" | "working_directory" | "directory"
    )
}

fn normalize_path(path: &str) -> String {
    let path = path.replace('\\', "/");
    let absolute = path.starts_with('/');
    let mut components: Vec<&str> = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." if components.last().is_some_and(|part| *part != "..") => {
                components.pop();
            }
            ".." if !absolute => components.push(component),
            ".." => {}
            other => components.push(other),
        }
    }
    let joined = components.join("/");
    if absolute {
        format!("/{joined}")
    } else if joined.is_empty() && !path.is_empty() {
        ".".to_owned()
    } else {
        joined
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActionFingerprint {
    digest: [u64; 2],
    generation: u64,
}

impl std::fmt::Display for ActionFingerprint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{:016x}{:016x}{:016x}",
            self.digest[0], self.digest[1], self.generation
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    ApproveOnce,
    Reject,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalError {
    InvalidAction,
    NoPendingAction,
    StaleAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingApproval {
    action: PendingAction,
    fingerprint: ActionFingerprint,
}

impl PendingApproval {
    fn new(action: PendingAction, generation: u64) -> Self {
        let mut fingerprint = action.fingerprint();
        fingerprint.generation = generation;
        Self {
            action,
            fingerprint,
        }
    }

    pub fn action(&self) -> &PendingAction {
        &self.action
    }

    pub fn fingerprint(&self) -> ActionFingerprint {
        self.fingerprint
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalResolution {
    pub action: PendingAction,
    pub decision: ApprovalDecision,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApprovalState {
    pending: Option<PendingApproval>,
    resolution: Option<ApprovalResolution>,
    next_generation: u64,
}

impl ApprovalState {
    pub fn request(&mut self, action: PendingAction) -> ActionFingerprint {
        let generation = self.next_generation;
        self.next_generation = self.next_generation.saturating_add(1);
        let pending = PendingApproval::new(action, generation);
        let fingerprint = pending.fingerprint();
        self.pending = Some(pending);
        self.resolution = None;
        fingerprint
    }

    pub fn pending(&self) -> Option<&PendingApproval> {
        self.pending.as_ref()
    }

    pub fn resolution(&self) -> Option<&ApprovalResolution> {
        self.resolution.as_ref()
    }

    /// Takes the one-shot decision result so a caller cannot consume it twice.
    pub fn take_resolution(&mut self) -> Option<ApprovalResolution> {
        self.resolution.take()
    }

    pub fn decide(
        &mut self,
        fingerprint: ActionFingerprint,
        decision: ApprovalDecision,
    ) -> Result<&ApprovalResolution, ApprovalError> {
        let Some(pending) = self.pending.as_ref() else {
            return Err(ApprovalError::NoPendingAction);
        };
        if pending.fingerprint() != fingerprint {
            return Err(ApprovalError::StaleAction);
        }
        let pending = self
            .pending
            .take()
            .expect("pending action was checked above");
        self.resolution = Some(ApprovalResolution {
            action: pending.action,
            decision,
        });
        Ok(self
            .resolution
            .as_ref()
            .expect("resolution was just assigned"))
    }
}

#[cfg(test)]
mod tests {
    use super::{ApprovalDecision, ApprovalError, ApprovalState, PendingAction};

    fn action(command: &str, args: &[&str], path: &str) -> PendingAction {
        PendingAction::new(
            "run_command",
            serde_json::json!({
                "command": command,
                "args": args,
                "path": path,
            }),
        )
        .unwrap()
    }

    #[test]
    fn command_arguments_and_path_changes_invalidate_prior_approval() {
        let variants = [
            action("cargo test", &["--lib"], "src/./main.rs"),
            action("cargo check", &["--lib"], "src/main.rs"),
            action("cargo test", &["--all"], "src/main.rs"),
            action("cargo test", &["--lib"], "src/other.rs"),
        ];
        let first = variants[0].fingerprint();
        assert_eq!(
            first,
            action("cargo test", &["--lib"], "src/main.rs").fingerprint()
        );
        assert_ne!(variants[1].fingerprint(), first);
        assert_ne!(variants[2].fingerprint(), first);
        assert_ne!(variants[3].fingerprint(), first);

        let mut state = ApprovalState::default();
        state.request(variants[0].clone());
        let stale_fingerprint = state.pending().unwrap().fingerprint();
        state.request(variants[1].clone());
        assert_eq!(
            state.decide(stale_fingerprint, ApprovalDecision::ApproveOnce),
            Err(ApprovalError::StaleAction)
        );
        assert_eq!(state.pending().unwrap().action(), &variants[1]);
    }

    #[test]
    fn approval_rejection_and_cancellation_apply_once() {
        for decision in [
            ApprovalDecision::ApproveOnce,
            ApprovalDecision::Reject,
            ApprovalDecision::Cancel,
        ] {
            let mut state = ApprovalState::default();
            state.request(action("cargo test", &["--lib"], "src/main.rs"));
            let fingerprint = state.pending().unwrap().fingerprint();
            assert!(state.decide(fingerprint, decision).is_ok());
            assert_eq!(
                state.decide(fingerprint, decision),
                Err(ApprovalError::NoPendingAction)
            );
            assert!(state.take_resolution().is_some());
            assert!(state.take_resolution().is_none());
        }
    }

    #[test]
    fn a_new_approval_request_gets_a_fresh_one_shot_fingerprint() {
        let action = action("cargo test", &["--lib"], "src/main.rs");
        let mut state = ApprovalState::default();
        state.request(action.clone());
        let first = state.pending().unwrap().fingerprint();
        state.request(action);
        let second = state.pending().unwrap().fingerprint();

        assert_ne!(first, second);
        assert_eq!(
            state.decide(first, ApprovalDecision::ApproveOnce),
            Err(ApprovalError::StaleAction)
        );
    }
}
