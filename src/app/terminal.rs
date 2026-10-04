#[cfg(test)]
mod tests {
    use super::{TerminalControl, with_terminal};
    use std::io;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct State {
        events: Arc<Mutex<Vec<&'static str>>>,
        enter_fails: bool,
    }

    struct FakeControl(State);

    impl TerminalControl for FakeControl {
        fn enter(&mut self) -> io::Result<()> {
            self.0.events.lock().unwrap().push("enter");
            if self.0.enter_fails {
                Err(io::Error::other("enter failed"))
            } else {
                Ok(())
            }
        }

        fn restore(&mut self) -> io::Result<()> {
            self.0.events.lock().unwrap().push("restore");
            Ok(())
        }
    }

    fn events(state: &State) -> Vec<&'static str> {
        state.events.lock().unwrap().clone()
    }

    #[test]
    fn restores_after_success() {
        let state = State::default();
        let during_run = state.clone();
        with_terminal(FakeControl(state.clone()), move || {
            during_run.events.lock().unwrap().push("run");
            Ok(())
        })
        .unwrap();
        assert_eq!(events(&state), ["enter", "run", "restore"]);
    }

    #[test]
    fn restores_after_operation_error() {
        let state = State::default();
        let result = with_terminal(FakeControl(state.clone()), || {
            Err(io::Error::other("operation failed"))
        });
        assert_eq!(result.unwrap_err().to_string(), "operation failed");
        assert_eq!(events(&state), ["enter", "restore"]);
    }

    #[test]
    fn restores_during_panic_unwind() {
        let state = State::default();
        let during_run = state.clone();
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _ = with_terminal(FakeControl(state.clone()), move || -> io::Result<()> {
                during_run.events.lock().unwrap().push("run");
                panic!("operation panicked");
            });
        }));
        assert!(result.is_err());
        assert_eq!(events(&state), ["enter", "run", "restore"]);
    }

    #[test]
    fn attempts_restore_after_partially_failing_enter() {
        let state = State {
            enter_fails: true,
            ..State::default()
        };
        let result = with_terminal(FakeControl(state.clone()), || Ok(()));
        assert_eq!(result.unwrap_err().to_string(), "enter failed");
        assert_eq!(events(&state), ["enter", "restore"]);
    }
}

use std::io;

use crossterm::{
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};

/// Operations needed to enter and restore the interactive terminal state.
pub trait TerminalControl {
    fn enter(&mut self) -> io::Result<()>;
    fn restore(&mut self) -> io::Result<()>;
}

/// Crossterm implementation of terminal raw mode and alternate-screen control.
#[derive(Debug, Default, Clone, Copy)]
pub struct CrosstermControl;

impl TerminalControl for CrosstermControl {
    fn enter(&mut self) -> io::Result<()> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)
    }

    fn restore(&mut self) -> io::Result<()> {
        let raw_mode_result = disable_raw_mode();
        let alternate_screen_result = execute!(io::stdout(), LeaveAlternateScreen);

        match (raw_mode_result, alternate_screen_result) {
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
        }
    }
}

/// Restores terminal state when dropped unless restoration has already run.
pub struct TerminalGuard<T: TerminalControl> {
    control: T,
    armed: bool,
}

impl<T: TerminalControl> TerminalGuard<T> {
    pub fn new(control: T) -> Self {
        Self {
            control,
            armed: true,
        }
    }

    pub fn enter(&mut self) -> io::Result<()> {
        self.control.enter()
    }

    /// Restores the terminal state exactly once. A failed restoration keeps
    /// the guard armed so `Drop` can still retry it.
    pub fn restore(&mut self) -> io::Result<()> {
        if !self.armed {
            return Ok(());
        }
        match self.control.restore() {
            Ok(()) => {
                self.armed = false;
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
}

impl<T: TerminalControl> Drop for TerminalGuard<T> {
    fn drop(&mut self) {
        if self.armed {
            self.armed = false;
            let _ = self.control.restore();
        }
    }
}

/// Runs an operation with the terminal configured and always attempts restoration.
pub fn with_terminal<T, F>(control: T, run: F) -> io::Result<()>
where
    T: TerminalControl,
    F: FnOnce() -> io::Result<()>,
{
    let mut guard = TerminalGuard::new(control);
    guard.enter()?;

    let operation_result = run();
    let restore_result = guard.restore();

    match operation_result {
        Err(error) => Err(error),
        Ok(()) => restore_result,
    }
}
