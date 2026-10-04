use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};

use crate::{
    policy::{
        approval::{ApprovalResolution, PendingAction},
        commands::{CommandPolicy, CommandPolicyError, CommandRequest, ValidatedCommand},
    },
    tools::{Tool, ToolExecutionError, ToolMetadata, ToolRequest, ToolResult},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub truncated: bool,
}

impl CommandOutput {
    pub fn success(&self) -> bool {
        self.exit_code == Some(0)
    }

    pub fn is_overflow(&self) -> bool {
        self.truncated
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RunCommandError {
    #[error("command policy error: {0}")]
    Policy(#[from] CommandPolicyError),
    #[error("failed to spawn child process: {0}")]
    Spawn(std::io::Error),
    #[error("command timed out after {0:?}")]
    Timeout(Duration),
    #[error("failed to wait on child process: {0}")]
    Wait(std::io::Error),
    #[error("io error during stream capture: {0}")]
    Io(#[from] std::io::Error),
}

pub struct RunningCommand {
    child: tokio::process::Child,
    stdout: Option<tokio::process::ChildStdout>,
    stderr: Option<tokio::process::ChildStderr>,
    timeout: Duration,
    max_output_bytes: usize,
}

impl RunningCommand {
    pub fn id(&self) -> Option<u32> {
        self.child.id()
    }

    pub async fn terminate(&mut self) -> Result<(), std::io::Error> {
        self.child.start_kill()?;
        let _ = self.child.wait().await;
        Ok(())
    }

    pub async fn wait(mut self) -> Result<CommandOutput, RunCommandError> {
        let stdout = self.stdout.take();
        let stderr = self.stderr.take();
        let timeout = self.timeout;
        let max_output_bytes = self.max_output_bytes;

        let execution = async {
            let stdout_capture = async {
                if let Some(stdout) = stdout {
                    capture_bounded_stream(stdout, max_output_bytes).await
                } else {
                    Ok((Vec::new(), false))
                }
            };
            let stderr_capture = async {
                if let Some(stderr) = stderr {
                    capture_bounded_stream(stderr, max_output_bytes).await
                } else {
                    Ok((Vec::new(), false))
                }
            };

            let (stdout_res, stderr_res, status_res) =
                tokio::join!(stdout_capture, stderr_capture, self.child.wait());

            let (stdout_bytes, stdout_truncated) = stdout_res?;
            let (stderr_bytes, stderr_truncated) = stderr_res?;
            let status = status_res.map_err(RunCommandError::Wait)?;

            let stdout = String::from_utf8_lossy(&stdout_bytes).into_owned();
            let stderr = String::from_utf8_lossy(&stderr_bytes).into_owned();
            let truncated = stdout_truncated || stderr_truncated;

            Ok::<CommandOutput, RunCommandError>(CommandOutput {
                stdout,
                stderr,
                exit_code: status.code(),
                truncated,
            })
        };

        match tokio::time::timeout(timeout, execution).await {
            Ok(result) => result,
            Err(_) => {
                let _ = self.child.start_kill();
                let _ = self.child.wait().await;
                Err(RunCommandError::Timeout(timeout))
            }
        }
    }
}

async fn capture_bounded_stream<R: tokio::io::AsyncRead + Unpin>(
    mut reader: R,
    max_bytes: usize,
) -> Result<(Vec<u8>, bool), std::io::Error> {
    use tokio::io::AsyncReadExt;

    let mut buf = Vec::new();
    let mut truncated = false;
    let mut chunk = [0u8; 4096];

    loop {
        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        if buf.len() < max_bytes {
            let available = max_bytes - buf.len();
            if n <= available {
                buf.extend_from_slice(&chunk[..n]);
            } else {
                buf.extend_from_slice(&chunk[..available]);
                truncated = true;
            }
        } else {
            truncated = true;
        }
    }

    Ok((buf, truncated))
}

pub fn spawn_command(command: &ValidatedCommand) -> Result<RunningCommand, RunCommandError> {
    let mut cmd = tokio::process::Command::new(&command.program);
    cmd.args(&command.args);
    cmd.current_dir(&command.cwd);
    cmd.kill_on_drop(true);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.stdin(std::process::Stdio::null());

    if command.clear_env {
        cmd.env_clear();
    }
    for (key, val) in &command.env {
        cmd.env(key, val);
    }

    let mut child = cmd.spawn().map_err(RunCommandError::Spawn)?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    Ok(RunningCommand {
        child,
        stdout,
        stderr,
        timeout: command.timeout,
        max_output_bytes: command.max_output_bytes,
    })
}

pub async fn execute_command(command: &ValidatedCommand) -> Result<CommandOutput, RunCommandError> {
    spawn_command(command)?.wait().await
}

pub async fn run_command(
    root: impl AsRef<Path>,
    request: &CommandRequest,
    resolution: &ApprovalResolution,
) -> Result<CommandOutput, RunCommandError> {
    let policy = CommandPolicy::new(root.as_ref());
    let validated = policy.authorize(request, resolution)?;
    execute_command(&validated).await
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunCommandInput {
    pub argv: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
}

pub struct RunCommandTool {
    policy: CommandPolicy,
}

impl RunCommandTool {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            policy: CommandPolicy::new(workspace_root),
        }
    }

    pub fn policy(&self) -> &CommandPolicy {
        &self.policy
    }

    /// Parses a model-supplied call into a policy request before any approval is asked for.
    fn command_request(&self, request: &ToolRequest) -> Result<CommandRequest, ToolExecutionError> {
        let input: RunCommandInput =
            serde_json::from_value(request.arguments.clone()).map_err(|error| {
                ToolExecutionError::Refused(format!("invalid tool arguments: {error}"))
            })?;
        CommandRequest::new(input.argv, input.cwd.unwrap_or_else(|| ".".to_owned()))
            .map_err(|error| ToolExecutionError::Refused(error.to_string()))
    }
}

impl Tool for RunCommandTool {
    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: "run_command".into(),
            description: "Execute a bounded command in the workspace after approval.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "argv": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Command and arguments as an array of strings (no shell interpolation)"
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Optional working directory relative to workspace root"
                    }
                },
                "required": ["argv"],
                "additionalProperties": false
            }),
        }
    }

    fn execute<'a>(
        &'a self,
        _request: ToolRequest,
    ) -> BoxFuture<'a, Result<ToolResult, ToolExecutionError>> {
        Box::pin(async move { Err(ToolExecutionError::Failed) })
    }

    fn requires_approval(&self) -> bool {
        true
    }

    fn pending_action(&self, request: &ToolRequest) -> Result<PendingAction, ToolExecutionError> {
        let command = self.command_request(request)?;
        self.policy
            .to_pending_action(&command)
            .map_err(|error| ToolExecutionError::Refused(error.to_string()))
    }

    fn execute_approved<'a>(
        &'a self,
        request: ToolRequest,
        resolution: &'a ApprovalResolution,
    ) -> BoxFuture<'a, Result<ToolResult, ToolExecutionError>> {
        Box::pin(async move {
            let command = self.command_request(&request)?;
            let validated = self
                .policy
                .authorize(&command, resolution)
                .map_err(|error| ToolExecutionError::Refused(error.to_string()))?;
            let output = execute_command(&validated)
                .await
                .map_err(|error| ToolExecutionError::Refused(error.to_string()))?;
            Ok(ToolResult {
                call_id: request.call_id,
                content: serde_json::to_string(&output).unwrap_or_else(|_| {
                    "{\"error\":\"command output could not be encoded\"}".into()
                }),
                is_error: !output.success(),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::Duration,
    };

    use tempfile::tempdir;

    use super::{RunCommandError, RunCommandTool, execute_command, run_command, spawn_command};
    use crate::{
        agent::message::ToolCallId,
        policy::{
            approval::{ApprovalDecision, ApprovalResolution, ApprovalState, PendingAction},
            commands::{CommandPolicy, CommandPolicyError, CommandRequest, ValidatedCommand},
        },
        tools::{Tool, ToolExecutionError, ToolRequest},
    };

    fn create_fake_executable(dir: &Path) -> PathBuf {
        let script_path = dir.join("fake_runner.sh");
        let script_content = r#"#!/bin/sh
case "$1" in
  stdout)
    printf "hello from fake stdout\n"
    ;;
  stderr)
    printf "error on fake stderr\n" >&2
    ;;
  exit_code)
    exit "$2"
    ;;
  sleep)
    sleep "$2"
    ;;
  record_pid_and_sleep)
    echo $$ > "$2"
    sleep "$3"
    ;;
  overflow)
    for i in $(seq 1 1000); do
      printf "0123456789"
    done
    ;;
  cwd)
    pwd -P
    ;;
  env)
    printf "%s" "$TEST_VAR"
    ;;
  *)
    echo "unknown subcommand: $1" >&2
    exit 1
    ;;
esac
"#;
        fs::write(&script_path, script_content).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&script_path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script_path, perms).unwrap();
        }
        script_path
    }

    fn make_command(
        script: &Path,
        args: &[&str],
        cwd: &Path,
        timeout: Duration,
        max_output_bytes: usize,
    ) -> ValidatedCommand {
        ValidatedCommand {
            program: script.to_str().unwrap().to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            cwd: cwd.to_path_buf(),
            timeout,
            max_output_bytes,
            env: std::collections::HashMap::new(),
            clear_env: false,
        }
    }

    #[tokio::test]
    async fn fake_executable_captures_stdout() {
        let dir = tempdir().unwrap();
        let script = create_fake_executable(dir.path());
        let cmd = make_command(
            &script,
            &["stdout"],
            dir.path(),
            Duration::from_secs(5),
            1024,
        );

        let output = execute_command(&cmd).await.unwrap();
        assert_eq!(output.stdout, "hello from fake stdout\n");
        assert_eq!(output.stderr, "");
        assert_eq!(output.exit_code, Some(0));
        assert!(output.success());
        assert!(!output.truncated);
    }

    #[tokio::test]
    async fn fake_executable_captures_stderr() {
        let dir = tempdir().unwrap();
        let script = create_fake_executable(dir.path());
        let cmd = make_command(
            &script,
            &["stderr"],
            dir.path(),
            Duration::from_secs(5),
            1024,
        );

        let output = execute_command(&cmd).await.unwrap();
        assert_eq!(output.stdout, "");
        assert_eq!(output.stderr, "error on fake stderr\n");
        assert_eq!(output.exit_code, Some(0));
        assert!(output.success());
        assert!(!output.truncated);
    }

    #[tokio::test]
    async fn fake_executable_nonzero_exit() {
        let dir = tempdir().unwrap();
        let script = create_fake_executable(dir.path());
        let cmd = make_command(
            &script,
            &["exit_code", "42"],
            dir.path(),
            Duration::from_secs(5),
            1024,
        );

        let output = execute_command(&cmd).await.unwrap();
        assert_eq!(output.exit_code, Some(42));
        assert!(!output.success());
    }

    #[tokio::test]
    async fn fake_executable_timeout_terminates_child() {
        let dir = tempdir().unwrap();
        let script = create_fake_executable(dir.path());
        let cmd = make_command(
            &script,
            &["sleep", "10"],
            dir.path(),
            Duration::from_millis(50),
            1024,
        );

        let result = execute_command(&cmd).await;
        match result {
            Err(RunCommandError::Timeout(d)) => assert_eq!(d, Duration::from_millis(50)),
            other => panic!("expected timeout error, got {other:?}"),
        }
    }

    fn is_process_alive(pid: u32) -> bool {
        #[cfg(unix)]
        {
            unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
        }
        #[cfg(not(unix))]
        {
            let status_file = format!("/proc/{pid}/status");
            if let Ok(content) = fs::read_to_string(&status_file) {
                !content
                    .lines()
                    .any(|l| l.starts_with("State:") && l.contains('Z'))
            } else {
                false
            }
        }
    }

    #[tokio::test]
    async fn fake_executable_cancellation_terminates_child_no_child_survives() {
        let dir = tempdir().unwrap();
        let script = create_fake_executable(dir.path());
        let pid_file = dir.path().join("child.pid");
        let cmd = make_command(
            &script,
            &["record_pid_and_sleep", pid_file.to_str().unwrap(), "30"],
            dir.path(),
            Duration::from_secs(10),
            1024,
        );

        let handle = tokio::spawn(async move { execute_command(&cmd).await });

        let mut pid: Option<u32> = None;
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            if pid_file.exists()
                && let Ok(content) = fs::read_to_string(&pid_file)
                && let Ok(p) = content.trim().parse::<u32>()
            {
                pid = Some(p);
                break;
            }
        }
        let pid = pid.expect("child should have recorded PID");
        assert!(
            is_process_alive(pid),
            "child process should be alive before cancellation"
        );

        // Cancel the task running the command
        handle.abort();
        let _ = handle.await;

        // Verify no child survives cancellation
        let mut alive = true;
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            if !is_process_alive(pid) {
                alive = false;
                break;
            }
        }
        assert!(!alive, "process with pid {pid} survived cancellation!");
    }

    #[tokio::test]
    async fn fake_executable_output_overflow_is_bounded_and_truncated() {
        let dir = tempdir().unwrap();
        let script = create_fake_executable(dir.path());
        let cmd = make_command(
            &script,
            &["overflow"],
            dir.path(),
            Duration::from_secs(5),
            100,
        );

        let output = execute_command(&cmd).await.unwrap();
        assert_eq!(output.stdout.len(), 100);
        assert!(output.truncated);
        assert!(output.is_overflow());
        assert_eq!(output.exit_code, Some(0));
    }

    #[tokio::test]
    async fn fake_executable_respects_cwd() {
        let dir = tempdir().unwrap();
        let script = create_fake_executable(dir.path());
        let sub = dir.path().join("sub_workspace");
        fs::create_dir_all(&sub).unwrap();

        let cmd = make_command(&script, &["cwd"], &sub, Duration::from_secs(5), 1024);

        let output = execute_command(&cmd).await.unwrap();
        let expected_cwd = sub.canonicalize().unwrap().to_str().unwrap().to_string();
        assert_eq!(output.stdout.trim(), expected_cwd);
        assert_eq!(output.exit_code, Some(0));
    }

    #[tokio::test]
    async fn fake_executable_receives_explicit_environment() {
        let dir = tempdir().unwrap();
        let script = create_fake_executable(dir.path());
        let mut cmd = make_command(&script, &["env"], dir.path(), Duration::from_secs(5), 1024);
        cmd.env
            .insert("TEST_VAR".to_string(), "hello_from_env".to_string());

        let output = execute_command(&cmd).await.unwrap();
        assert_eq!(output.stdout.trim(), "hello_from_env");
        assert_eq!(output.exit_code, Some(0));
    }

    #[tokio::test]
    async fn explicit_terminate_kills_running_command() {
        let dir = tempdir().unwrap();
        let script = create_fake_executable(dir.path());
        let cmd = make_command(
            &script,
            &["sleep", "10"],
            dir.path(),
            Duration::from_secs(10),
            1024,
        );

        let mut running = spawn_command(&cmd).unwrap();
        let pid = running.id().expect("running command should have pid");
        assert!(is_process_alive(pid));

        running.terminate().await.unwrap();
        assert!(!is_process_alive(pid));
    }

    #[tokio::test]
    async fn run_command_with_approval_resolution() {
        let dir = tempdir().unwrap();
        let script = create_fake_executable(dir.path());
        let req = CommandRequest::new([script.to_str().unwrap(), "stdout"], ".").unwrap();

        let policy = CommandPolicy::new(dir.path());
        let action = policy.to_pending_action(&req).unwrap();

        let mut approval_state = ApprovalState::default();
        let fp = approval_state.request(action.clone());
        let resolution = approval_state
            .decide(fp, ApprovalDecision::ApproveOnce)
            .unwrap()
            .clone();

        let output = run_command(dir.path(), &req, &resolution).await.unwrap();
        assert_eq!(output.stdout, "hello from fake stdout\n");

        let fp2 = approval_state.request(action);
        let reject_res = approval_state
            .decide(fp2, ApprovalDecision::Reject)
            .unwrap()
            .clone();
        let err = run_command(dir.path(), &req, &reject_res)
            .await
            .unwrap_err();
        match err {
            RunCommandError::Policy(CommandPolicyError::NotApproved) => {}
            other => panic!("expected NotApproved, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_command_rejects_file_as_cwd() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("file.txt");
        fs::write(&file, "contents").unwrap();

        let req = CommandRequest::new(["ls"], "file.txt").unwrap();
        let mut approval_state = ApprovalState::default();
        let dummy_action = PendingAction::new("run_command", serde_json::json!({})).unwrap();
        let fp = approval_state.request(dummy_action);
        let resolution = approval_state
            .decide(fp, ApprovalDecision::ApproveOnce)
            .unwrap()
            .clone();

        let err = run_command(dir.path(), &req, &resolution)
            .await
            .unwrap_err();
        match err {
            RunCommandError::Policy(CommandPolicyError::InvalidCwd) => {}
            other => panic!("expected InvalidCwd, got {other:?}"),
        }
    }

    /// Builds a call that runs the fake executable and records its pid, proving it spawned.
    fn command_tool_request(script: &Path, marker: &Path) -> ToolRequest {
        ToolRequest {
            call_id: ToolCallId::new("call-cmd"),
            name: "run_command".into(),
            arguments: serde_json::json!({
                "argv": [script, "record_pid_and_sleep", marker, "0"],
            }),
        }
    }

    fn approve(action: &PendingAction) -> ApprovalResolution {
        let mut state = ApprovalState::default();
        let fingerprint = state.request(action.clone());
        state
            .decide(fingerprint, ApprovalDecision::ApproveOnce)
            .unwrap()
            .clone()
    }

    #[tokio::test]
    async fn approved_command_tool_runs_the_reviewed_argv() {
        let dir = tempdir().unwrap();
        let script = create_fake_executable(dir.path());
        let marker = dir.path().join("child.pid");
        let tool = RunCommandTool::new(dir.path());
        let request = command_tool_request(&script, &marker);
        let action = tool.pending_action(&request).unwrap();

        assert_eq!(action.tool(), "run_command");
        assert_eq!(action.target(), Some("."));
        assert!(action.arguments()["argv"].is_array());

        let result = tool
            .execute_approved(request, &approve(&action))
            .await
            .unwrap();

        assert_eq!(result.call_id.as_str(), "call-cmd");
        assert!(!result.is_error);
        assert!(
            result.content.contains("\"exit_code\":0"),
            "{}",
            result.content
        );
        assert!(
            marker.exists(),
            "an approved command must spawn its process"
        );
    }

    #[tokio::test]
    async fn rejected_command_tool_never_spawns_a_process() {
        let dir = tempdir().unwrap();
        let script = create_fake_executable(dir.path());
        let marker = dir.path().join("child.pid");
        let tool = RunCommandTool::new(dir.path());
        let request = command_tool_request(&script, &marker);
        let action = tool.pending_action(&request).unwrap();
        let mut state = ApprovalState::default();
        let fingerprint = state.request(action.clone());
        let rejection = state
            .decide(fingerprint, ApprovalDecision::Reject)
            .unwrap()
            .clone();

        let error = tool
            .execute_approved(request, &rejection)
            .await
            .unwrap_err();

        assert!(matches!(error, ToolExecutionError::Refused(_)), "{error:?}");
        assert!(!marker.exists());
    }

    #[tokio::test]
    async fn command_tool_refuses_an_approval_for_different_argv() {
        let dir = tempdir().unwrap();
        let script = create_fake_executable(dir.path());
        let marker = dir.path().join("child.pid");
        let tool = RunCommandTool::new(dir.path());
        let reviewed = tool
            .pending_action(&command_tool_request(&script, &marker))
            .unwrap();
        let requested = ToolRequest {
            call_id: ToolCallId::new("call-cmd"),
            name: "run_command".into(),
            arguments: serde_json::json!({ "argv": [script, "stdout"] }),
        };

        let error = tool
            .execute_approved(requested, &approve(&reviewed))
            .await
            .unwrap_err();

        assert!(matches!(error, ToolExecutionError::Refused(_)), "{error:?}");
        assert!(!marker.exists());
    }

    #[tokio::test]
    async fn command_tool_never_runs_without_approval() {
        let dir = tempdir().unwrap();
        let script = create_fake_executable(dir.path());
        let marker = dir.path().join("child.pid");
        let tool = RunCommandTool::new(dir.path());

        assert!(tool.requires_approval());
        assert_eq!(
            tool.execute(command_tool_request(&script, &marker)).await,
            Err(ToolExecutionError::Failed)
        );
        assert!(!marker.exists());
    }
}
