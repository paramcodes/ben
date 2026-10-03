use crossterm::event::KeyEvent;

/// Input and internal messages that can change application state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEvent {
    Key(KeyEvent),
    Submit,
    AppendOutput(String),
    ToolStatus(String),
    Quit,
}
