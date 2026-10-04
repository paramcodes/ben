use std::{
    collections::HashMap,
    fmt,
    path::{Path, PathBuf},
    time::Duration,
};

use crate::policy::{
    approval::{ApprovalDecision, ApprovalResolution, PendingAction},
    paths::{PathIntent, PathResolutionError, WorkspacePath},
};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 256 * 1024;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum CommandPolicyError {
    #[error("command argv must not be empty")]
    EmptyArgv,
    #[error("command program must not be empty")]
    EmptyProgram,
    #[error("command was not approved")]
    NotApproved,
    #[error("approval action does not match command request")]
    ActionMismatch,
    #[error("working directory resolves outside the workspace")]
    OutsideWorkspace,
    #[error("working directory is invalid or does not exist")]
    InvalidCwd,
}

impl From<PathResolutionError> for CommandPolicyError {
    fn from(err: PathResolutionError) -> Self {
        match err {
            PathResolutionError::OutsideWorkspace => CommandPolicyError::OutsideWorkspace,
            PathResolutionError::NotFound
            | PathResolutionError::NotDirectory
            | PathResolutionError::AbsolutePath
            | PathResolutionError::Traversal
            | PathResolutionError::WorkspaceUnavailable
            | PathResolutionError::Io => CommandPolicyError::InvalidCwd,
        }
    }
}

pub fn format_argv_display(argv: &[impl AsRef<str>]) -> String {
    argv.iter()
        .map(|arg| {
            let s = arg.as_ref();
            if s.is_empty() {
                "\"\"".to_string()
            } else if s.chars().all(|c| {
                c.is_ascii_alphanumeric()
                    || matches!(c, '-' | '_' | '.' | '/' | ':' | '=' | '@' | '+' | ',')
            }) {
                s.to_string()
            } else {
                let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
                format!("\"{escaped}\"")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRequest {
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub timeout: Duration,
    pub max_output_bytes: usize,
    pub env: HashMap<String, String>,
    pub clear_env: bool,
}

impl CommandRequest {
    pub fn new(
        argv: impl IntoIterator<Item = impl Into<String>>,
        cwd: impl Into<PathBuf>,
    ) -> Result<Self, CommandPolicyError> {
        let argv: Vec<String> = argv.into_iter().map(Into::into).collect();
        if argv.is_empty() {
            return Err(CommandPolicyError::EmptyArgv);
        }
        if argv[0].trim().is_empty() {
            return Err(CommandPolicyError::EmptyProgram);
        }
        Ok(Self {
            argv,
            cwd: cwd.into(),
            timeout: DEFAULT_TIMEOUT,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            env: HashMap::new(),
            clear_env: false,
        })
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_max_output_bytes(mut self, max_output_bytes: usize) -> Self {
        self.max_output_bytes = max_output_bytes;
        self
    }

    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    pub fn with_envs(
        mut self,
        envs: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        for (key, val) in envs {
            self.env.insert(key.into(), val.into());
        }
        self
    }

    pub fn with_clear_env(mut self, clear: bool) -> Self {
        self.clear_env = clear;
        self
    }

    pub fn program(&self) -> &str {
        &self.argv[0]
    }

    pub fn args(&self) -> &[String] {
        &self.argv[1..]
    }

    pub fn normalized_display(&self) -> String {
        format_argv_display(&self.argv)
    }

    pub fn to_pending_action(
        &self,
        root: impl AsRef<Path>,
    ) -> Result<PendingAction, CommandPolicyError> {
        let root = root.as_ref();
        let (_resolved, relative) = resolve_cwd(root, &self.cwd)?;

        let display = self.normalized_display();
        let args = self.args().to_vec();
        PendingAction::new(
            "run_command",
            serde_json::json!({
                "command": display,
                "argv": self.argv,
                "args": args,
                "cwd": relative,
                "path": relative,
                "timeout_ms": self.timeout.as_millis() as u64,
                "max_output_bytes": self.max_output_bytes,
            }),
        )
        .map_err(|_| CommandPolicyError::ActionMismatch)
    }
}

impl fmt::Display for CommandRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.normalized_display())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub timeout: Duration,
    pub max_output_bytes: usize,
    pub env: HashMap<String, String>,
    pub clear_env: bool,
}

impl ValidatedCommand {
    pub fn argv(&self) -> Vec<String> {
        let mut argv = Vec::with_capacity(1 + self.args.len());
        argv.push(self.program.clone());
        argv.extend(self.args.clone());
        argv
    }

    pub fn normalized_display(&self) -> String {
        format_argv_display(&self.argv())
    }
}

#[derive(Debug, Clone)]
pub struct CommandPolicy {
    workspace_root: PathBuf,
    default_timeout: Duration,
    max_output_bytes: usize,
}

impl CommandPolicy {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            default_timeout: DEFAULT_TIMEOUT,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        }
    }

    pub fn with_default_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = timeout;
        self
    }

    pub fn with_max_output_bytes(mut self, max_bytes: usize) -> Self {
        self.max_output_bytes = max_bytes;
        self
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn to_pending_action(
        &self,
        request: &CommandRequest,
    ) -> Result<PendingAction, CommandPolicyError> {
        request.to_pending_action(&self.workspace_root)
    }

    pub fn authorize(
        &self,
        request: &CommandRequest,
        resolution: &ApprovalResolution,
    ) -> Result<ValidatedCommand, CommandPolicyError> {
        if resolution.decision != ApprovalDecision::ApproveOnce {
            return Err(CommandPolicyError::NotApproved);
        }
        if resolution.action.tool() != "run_command" {
            return Err(CommandPolicyError::ActionMismatch);
        }

        let expected_action = request.to_pending_action(&self.workspace_root)?;
        if resolution.action != expected_action {
            let matches = self.verify_action_matches(request, &resolution.action)?;
            if !matches {
                return Err(CommandPolicyError::ActionMismatch);
            }
        }

        let (resolved, _) = resolve_cwd(&self.workspace_root, &request.cwd)?;
        Ok(ValidatedCommand {
            program: request.program().to_string(),
            args: request.args().to_vec(),
            cwd: resolved.path().to_path_buf(),
            timeout: request.timeout,
            max_output_bytes: request.max_output_bytes,
            env: request.env.clone(),
            clear_env: request.clear_env,
        })
    }

    fn verify_action_matches(
        &self,
        request: &CommandRequest,
        action: &PendingAction,
    ) -> Result<bool, CommandPolicyError> {
        let Some(args) = action.arguments().as_object() else {
            return Ok(false);
        };
        if action.tool() != "run_command" {
            return Ok(false);
        }

        let (_, relative_cwd) = resolve_cwd(&self.workspace_root, &request.cwd)?;
        let action_target = action.target().unwrap_or(".");
        if action_target != relative_cwd {
            return Ok(false);
        }

        if let Some(argv_val) = args.get("argv").and_then(serde_json::Value::as_array) {
            let parsed_argv: Vec<String> = argv_val
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(String::from)
                .collect();
            if parsed_argv != request.argv {
                return Ok(false);
            }
        } else if let Some(cmd_val) = args.get("command").and_then(serde_json::Value::as_str) {
            if cmd_val != request.normalized_display() && cmd_val != request.program() {
                return Ok(false);
            }
        } else {
            return Ok(false);
        }
        Ok(true)
    }
}

fn resolve_cwd(root: &Path, cwd: &Path) -> Result<(WorkspacePath, String), CommandPolicyError> {
    let resolved = WorkspacePath::resolve(root, cwd, PathIntent::Existing)?;
    let relative = resolved
        .path()
        .strip_prefix(resolved.root())
        .map(|p| {
            let s = p.to_string_lossy().replace('\\', "/");
            if s.is_empty() { ".".to_owned() } else { s }
        })
        .map_err(|_| CommandPolicyError::OutsideWorkspace)?;
    Ok((resolved, relative))
}

#[cfg(test)]
mod tests {
    use std::{fs, time::Duration};

    use tempfile::tempdir;

    use super::{
        CommandPolicy, CommandPolicyError, CommandRequest, DEFAULT_MAX_OUTPUT_BYTES,
        DEFAULT_TIMEOUT, format_argv_display,
    };
    use crate::policy::approval::{
        ApprovalDecision, ApprovalResolution, ApprovalState, PendingAction,
    };

    #[test]
    fn empty_argv_and_empty_program_rejected() {
        assert_eq!(
            CommandRequest::new(Vec::<String>::new(), "."),
            Err(CommandPolicyError::EmptyArgv)
        );
        assert_eq!(
            CommandRequest::new(["   "], "."),
            Err(CommandPolicyError::EmptyProgram)
        );
    }

    #[test]
    fn normalized_display_formats_argv_cleanly() {
        assert_eq!(
            format_argv_display(&["cargo", "test", "--lib"]),
            "cargo test --lib"
        );
        assert_eq!(
            format_argv_display(&["echo", "hello world"]),
            "echo \"hello world\""
        );
        assert_eq!(
            format_argv_display(&["grep", "", "test.rs"]),
            "grep \"\" test.rs"
        );
        assert_eq!(
            format_argv_display(&["echo", "escaped \"quotes\" and \\slash"]),
            "echo \"escaped \\\"quotes\\\" and \\\\slash\""
        );
    }

    #[test]
    fn request_builder_methods_work() {
        let req = CommandRequest::new(["git", "status"], ".")
            .unwrap()
            .with_timeout(Duration::from_secs(10))
            .with_max_output_bytes(4096)
            .with_env("GIT_AUTHOR_NAME", "Ben")
            .with_clear_env(true);

        assert_eq!(req.program(), "git");
        assert_eq!(req.args(), &["status"]);
        assert_eq!(req.timeout, Duration::from_secs(10));
        assert_eq!(req.max_output_bytes, 4096);
        assert_eq!(
            req.env.get("GIT_AUTHOR_NAME").map(String::as_str),
            Some("Ben")
        );
        assert!(req.clear_env);
    }

    #[test]
    fn pending_action_contains_expected_metadata() {
        let dir = tempdir().unwrap();
        let sub = dir.path().join("subdir");
        fs::create_dir_all(&sub).unwrap();

        let req = CommandRequest::new(["ls", "-la"], "subdir").unwrap();
        let action = req.to_pending_action(dir.path()).unwrap();

        assert_eq!(action.tool(), "run_command");
        assert_eq!(action.target(), Some("subdir"));
        let args = action.arguments();
        assert_eq!(args["command"], "ls -la");
        assert_eq!(args["cwd"], "subdir");
        assert_eq!(args["timeout_ms"], DEFAULT_TIMEOUT.as_millis() as u64);
        assert_eq!(args["max_output_bytes"], DEFAULT_MAX_OUTPUT_BYTES);
    }

    #[test]
    fn command_authorization_succeeds_with_approve_once() {
        let dir = tempdir().unwrap();
        let policy = CommandPolicy::new(dir.path());
        let req = CommandRequest::new(["cargo", "check"], ".").unwrap();
        let action = policy.to_pending_action(&req).unwrap();

        let mut approval_state = ApprovalState::default();
        let fingerprint = approval_state.request(action);
        let resolution = approval_state
            .decide(fingerprint, ApprovalDecision::ApproveOnce)
            .unwrap()
            .clone();

        let validated = policy.authorize(&req, &resolution).unwrap();
        assert_eq!(validated.program, "cargo");
        assert_eq!(validated.args, vec!["check".to_string()]);
        assert_eq!(validated.cwd, dir.path().canonicalize().unwrap());
    }

    #[test]
    fn command_authorization_fails_if_rejected_or_cancelled() {
        let dir = tempdir().unwrap();
        let policy = CommandPolicy::new(dir.path());
        let req = CommandRequest::new(["cargo", "check"], ".").unwrap();
        let action = policy.to_pending_action(&req).unwrap();

        let mut approval_state = ApprovalState::default();
        let fp1 = approval_state.request(action.clone());
        let reject_res = approval_state
            .decide(fp1, ApprovalDecision::Reject)
            .unwrap()
            .clone();
        assert_eq!(
            policy.authorize(&req, &reject_res),
            Err(CommandPolicyError::NotApproved)
        );

        let fp2 = approval_state.request(action);
        let cancel_res = approval_state
            .decide(fp2, ApprovalDecision::Cancel)
            .unwrap()
            .clone();
        assert_eq!(
            policy.authorize(&req, &cancel_res),
            Err(CommandPolicyError::NotApproved)
        );
    }

    #[test]
    fn command_authorization_fails_on_action_mismatch() {
        let dir = tempdir().unwrap();
        let policy = CommandPolicy::new(dir.path());
        let req1 = CommandRequest::new(["cargo", "check"], ".").unwrap();
        let req2 = CommandRequest::new(["cargo", "test"], ".").unwrap();
        let action2 = policy.to_pending_action(&req2).unwrap();

        let resolution = ApprovalResolution {
            action: action2,
            decision: ApprovalDecision::ApproveOnce,
        };
        assert_eq!(
            policy.authorize(&req1, &resolution),
            Err(CommandPolicyError::ActionMismatch)
        );

        let wrong_tool = ApprovalResolution {
            action: PendingAction::new("other_tool", serde_json::json!({})).unwrap(),
            decision: ApprovalDecision::ApproveOnce,
        };
        assert_eq!(
            policy.authorize(&req1, &wrong_tool),
            Err(CommandPolicyError::ActionMismatch)
        );
    }

    #[test]
    fn cwd_outside_workspace_or_nonexistent_rejected() {
        let dir = tempdir().unwrap();
        let policy = CommandPolicy::new(dir.path());

        let outside_req = CommandRequest::new(["ls"], "../outside").unwrap();
        assert_eq!(
            policy.to_pending_action(&outside_req),
            Err(CommandPolicyError::InvalidCwd)
        );

        let missing_req = CommandRequest::new(["ls"], "nonexistent_dir").unwrap();
        assert_eq!(
            policy.to_pending_action(&missing_req),
            Err(CommandPolicyError::InvalidCwd)
        );
    }
}
