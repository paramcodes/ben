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

use crate::providers::types::{Provider, ProviderError, ProviderRequest};

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

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use futures_util::stream;

    use super::{mpsc, spawn_provider_stream};
    use crate::{
        agent::message::Message,
        app::event::AppEvent,
        providers::types::{
            CompletionReason, Provider, ProviderCancellation, ProviderEvent, ProviderRequest,
            ProviderStream,
        },
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
}
