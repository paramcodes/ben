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
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::{
    runtime::Builder,
    sync::mpsc,
    time::{self, MissedTickBehavior},
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
