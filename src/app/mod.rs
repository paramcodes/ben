pub mod event;
pub mod terminal;
pub mod update;

use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use crossterm::event::{self as terminal_event, Event as CrosstermEvent};
use futures_util::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::{
    runtime::Builder,
    sync::mpsc,
    task::JoinHandle,
    time::{self, MissedTickBehavior},
};
use tokio_util::sync::CancellationToken;

use crate::{
    agent::Agent,
    policy::approval::ApprovalResolution,
    providers::types::{Provider, ProviderError, ProviderRequest},
};

use self::{
    event::{AppEvent, map_key},
    update::{AppState, update},
};

struct InputShutdown(Arc<AtomicBool>);

impl Drop for InputShutdown {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Starts the TUI runtime and returns after the user requests quit.
pub fn run() -> io::Result<()> {
    let runtime = Builder::new_multi_thread().enable_all().build()?;
    runtime.block_on(run_loop())
}

async fn run_loop() -> io::Result<()> {
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let (sender, mut receiver) = mpsc::channel::<io::Result<AppEvent>>(64);
    let stop_input = Arc::new(AtomicBool::new(false));
    let _input_shutdown = InputShutdown(Arc::clone(&stop_input));
    let input_stop = Arc::clone(&stop_input);
    let input_task = tokio::task::spawn_blocking(move || {
        while !input_stop.load(Ordering::Relaxed) {
            match terminal_event::poll(Duration::from_millis(50)) {
                Ok(true) => match terminal_event::read() {
                    Ok(CrosstermEvent::Key(key)) => {
                        if let Some(event) = map_key(key)
                            && sender.blocking_send(Ok(event)).is_err()
                        {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(error) => {
                        let _ = sender.blocking_send(Err(error));
                        break;
                    }
                },
                Ok(false) => {}
                Err(error) => {
                    let _ = sender.blocking_send(Err(error));
                    break;
                }
            }
        }
    });

    let mut state = AppState::default();
    let mut redraw = time::interval(Duration::from_millis(100));
    redraw.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut dirty = true;

    loop {
        if dirty {
            terminal.draw(|frame| crate::ui::render(frame, &state))?;
        }

        tokio::select! {
            message = receiver.recv() => match message {
                Some(Ok(event)) => {
                    let next = update(state.clone(), event);
                    dirty = next != state;
                    state = next;
                }
                Some(Err(error)) => {
                    stop_input.store(true, Ordering::Relaxed);
                    let _ = input_task.await;
                    return Err(error);
                }
                None => break,
            },
            _ = redraw.tick() => dirty = true,
        }

        if state.should_exit {
            break;
        }
    }

    stop_input.store(true, Ordering::Relaxed);
    input_task
        .await
        .map_err(|error| io::Error::other(format!("input task failed: {error}")))?;
    terminal.show_cursor()?;
    Ok(())
}

/// Runs provider streaming away from the TUI loop and forwards typed events to it.
pub fn spawn_provider_stream(
    provider: std::sync::Arc<dyn Provider>,
    request: ProviderRequest,
    cancellation: CancellationToken,
    sender: mpsc::Sender<io::Result<AppEvent>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        if sender.send(Ok(AppEvent::ProviderStarted)).await.is_err() {
            return;
        }
        let mut stream = provider.stream(request, cancellation.clone());
        loop {
            tokio::select! {
                _ = cancellation.cancelled() => {
                    let _ = sender.send(Ok(AppEvent::ProviderCancelled)).await;
                    return;
                }
                event = stream.next() => match event {
                    Some(Ok(event)) => {
                        let completed = matches!(event, crate::providers::types::ProviderEvent::Completed(_));
                        if sender.send(Ok(AppEvent::ProviderEvent(event))).await.is_err() || completed {
                            return;
                        }
                    }
                    Some(Err(ProviderError::Cancelled)) => {
                        let _ = sender.send(Ok(AppEvent::ProviderCancelled)).await;
                        return;
                    }
                    Some(Err(error)) => {
                        let _ = sender.send(Ok(AppEvent::ProviderFailed(error))).await;
                        return;
                    }
                    None => {
                        let _ = sender.send(Ok(AppEvent::ProviderFailed(ProviderError::InvalidResponse))).await;
                        return;
                    }
                }
            }
        }
    })
}

/// Delivers user approval decisions from the TUI to the agent turn that is waiting for one.
pub struct ApprovalBridge {
    decisions: mpsc::Sender<ApprovalResolution>,
}

impl ApprovalBridge {
    /// Registers a fresh decision channel with `agent` and returns the TUI-side handle.
    pub fn attach(agent: &mut Agent) -> Self {
        let (decisions, pending) = mpsc::channel(1);
        agent.set_approvals(pending);
        Self { decisions }
    }

    /// Hands the decision the user just made to the agent, at most once per decision.
    pub fn forward(&self, state: &mut AppState) -> bool {
        match state.approval.take_resolution() {
            Some(resolution) => self.decisions.try_send(resolution).is_ok(),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use futures_util::stream;

    use super::update::{AppState, Screen, update};
    use super::{ApprovalBridge, mpsc, spawn_provider_stream};
    use crate::{
        agent::{
            Agent, StopReason,
            limits::AgentLimits,
            message::{Message, ToolCallId},
        },
        app::event::AppEvent,
        policy::approval::{ApprovalDecision, PendingAction},
        providers::{
            fake::FakeProvider,
            types::{
                CompletionReason, Provider, ProviderCancellation, ProviderEvent, ProviderRequest,
                ProviderStream, ToolCall,
            },
        },
        tools::{Tool, ToolExecutionError, ToolMetadata, ToolRegistry, ToolRequest, ToolResult},
    };
    use tokio_util::sync::CancellationToken;

    struct DelayedProvider;

    impl Provider for DelayedProvider {
        fn stream(
            &self,
            _request: ProviderRequest,
            _cancellation: ProviderCancellation,
        ) -> ProviderStream {
            Box::pin(stream::unfold(0, |index| async move {
                if index == 2 {
                    return None;
                }
                tokio::time::sleep(Duration::from_millis(150)).await;
                let event = if index == 0 {
                    ProviderEvent::TextDelta("answer".into())
                } else {
                    ProviderEvent::Completed(CompletionReason::EndTurn)
                };
                Some((Ok(event), index + 1))
            }))
        }
    }

    #[tokio::test]
    async fn input_events_remain_responsive_during_delayed_provider_streaming() {
        let (sender, mut receiver) = mpsc::channel(8);
        let request = ProviderRequest {
            model: "test-model".into(),
            messages: vec![Message::user("hello")],
            tools: Vec::new(),
            max_output_tokens: None,
        };
        let task = spawn_provider_stream(
            Arc::new(DelayedProvider),
            request,
            CancellationToken::new(),
            sender.clone(),
        );

        assert!(matches!(
            receiver.recv().await.unwrap(),
            Ok(AppEvent::ProviderStarted)
        ));
        sender
            .send(Ok(AppEvent::Key(KeyEvent::new(
                KeyCode::Char('x'),
                KeyModifiers::NONE,
            ))))
            .await
            .unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_millis(50), receiver.recv())
                .await
                .unwrap()
                .unwrap(),
            Ok(AppEvent::Key(_))
        ));
        assert!(matches!(
            tokio::time::timeout(Duration::from_millis(250), receiver.recv())
                .await
                .unwrap()
                .unwrap(),
            Ok(AppEvent::ProviderEvent(ProviderEvent::TextDelta(text))) if text == "answer"
        ));
        assert!(matches!(
            receiver.recv().await.unwrap(),
            Ok(AppEvent::ProviderEvent(ProviderEvent::Completed(_)))
        ));
        task.await.unwrap();
    }

    /// Counts executions so tests can prove a side effect ran zero or one times.
    struct RecordingTool {
        executions: Arc<AtomicUsize>,
    }

    impl Tool for RecordingTool {
        fn metadata(&self) -> ToolMetadata {
            ToolMetadata {
                name: "record_action".into(),
                description: "Records an approved action".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {"target": {"type": "string"}},
                    "required": ["target"],
                    "additionalProperties": false
                }),
            }
        }

        fn execute<'a>(
            &'a self,
            _request: ToolRequest,
        ) -> futures_util::future::BoxFuture<'a, Result<ToolResult, ToolExecutionError>> {
            Box::pin(async { Err(ToolExecutionError::Failed) })
        }

        fn requires_approval(&self) -> bool {
            true
        }

        fn pending_action(
            &self,
            request: &ToolRequest,
        ) -> Result<PendingAction, ToolExecutionError> {
            PendingAction::new("record_action", request.arguments.clone())
                .map_err(|_| ToolExecutionError::Refused("invalid tool arguments".to_owned()))
        }

        fn execute_approved<'a>(
            &'a self,
            request: ToolRequest,
            resolution: &'a crate::policy::approval::ApprovalResolution,
        ) -> futures_util::future::BoxFuture<'a, Result<ToolResult, ToolExecutionError>> {
            Box::pin(async move {
                if resolution.decision != ApprovalDecision::ApproveOnce
                    || resolution.action.arguments().get("target").is_none()
                {
                    return Err(ToolExecutionError::Refused("not approved".to_owned()));
                }
                self.executions.fetch_add(1, Ordering::SeqCst);
                Ok(ToolResult {
                    call_id: request.call_id,
                    content: "recorded".into(),
                    is_error: false,
                })
            })
        }
    }

    /// Builds a turn that asks for approval before running one side-effecting tool call.
    fn approval_turn(
        executions: Arc<AtomicUsize>,
    ) -> (
        tokio::task::JoinHandle<StopReason>,
        ApprovalBridge,
        mpsc::Receiver<io::Result<AppEvent>>,
    ) {
        let id = ToolCallId::new("call-approve");
        let arguments = r#"{"target":"notes.txt"}"#.to_owned();
        let provider = Arc::new(FakeProvider::with_responses(vec![
            vec![
                Ok(ProviderEvent::ToolCallStarted {
                    id: id.clone(),
                    name: "record_action".into(),
                }),
                Ok(ProviderEvent::ToolCallCompleted(ToolCall {
                    id,
                    name: "record_action".into(),
                    arguments,
                })),
                Ok(ProviderEvent::Completed(CompletionReason::ToolCalls)),
            ],
            vec![
                Ok(ProviderEvent::TextDelta("done".into())),
                Ok(ProviderEvent::Completed(CompletionReason::EndTurn)),
            ],
        ]));
        let mut registry = ToolRegistry::default();
        registry
            .register(Arc::new(RecordingTool { executions }))
            .unwrap();

        let mut agent = Agent::new(
            provider,
            "test-model",
            Vec::new(),
            AgentLimits {
                max_turns: 2,
                max_tool_calls: 4,
                max_history_messages: 8,
                max_output_tokens: Some(64),
            },
        );
        agent.set_tool_registry(Arc::new(registry), 1024);
        let approvals = ApprovalBridge::attach(&mut agent);
        let (sender, receiver) = mpsc::channel(32);
        let task = tokio::spawn(async move {
            agent
                .run_turn("record it", CancellationToken::new(), sender)
                .await
        });
        (task, approvals, receiver)
    }

    /// Drains turn events into application state the way the TUI loop does.
    async fn drain_until_approval(
        state: &mut AppState,
        receiver: &mut mpsc::Receiver<io::Result<AppEvent>>,
    ) {
        while let Some(event) = receiver.recv().await {
            let Ok(event) = event else { continue };
            if matches!(event, AppEvent::ApprovalRequested(_)) {
                *state = update(state.clone(), event);
                return;
            }
        }
        panic!("the turn ended before asking for approval");
    }

    fn decide(state: &mut AppState, decision: ApprovalDecision) -> bool {
        let fingerprint = state
            .approval
            .pending()
            .expect("an action must be pending")
            .fingerprint();
        *state = update(
            state.clone(),
            AppEvent::ApprovalDecision {
                fingerprint,
                decision,
            },
        );
        state.screen == Screen::Conversation
    }

    #[tokio::test]
    async fn approved_tui_decision_runs_the_action_once() {
        let executions = Arc::new(AtomicUsize::new(0));
        let (task, approvals, mut events) = approval_turn(executions.clone());
        let mut state = AppState::default();

        drain_until_approval(&mut state, &mut events).await;
        assert_eq!(state.screen, Screen::Approval);
        assert!(decide(&mut state, ApprovalDecision::ApproveOnce));
        assert!(approvals.forward(&mut state));
        assert!(
            !approvals.forward(&mut state),
            "a recorded decision must not be delivered twice"
        );

        assert_eq!(
            task.await.unwrap(),
            StopReason::Completed(CompletionReason::EndTurn)
        );
        assert_eq!(executions.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn rejected_tui_decision_leaves_the_action_unexecuted() {
        let executions = Arc::new(AtomicUsize::new(0));
        let (task, approvals, mut events) = approval_turn(executions.clone());
        let mut state = AppState::default();

        drain_until_approval(&mut state, &mut events).await;
        assert!(decide(&mut state, ApprovalDecision::Reject));
        assert!(approvals.forward(&mut state));

        assert_eq!(
            task.await.unwrap(),
            StopReason::Completed(CompletionReason::EndTurn)
        );
        assert_eq!(executions.load(Ordering::SeqCst), 0);
    }
}
