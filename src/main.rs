pub mod agent;
pub mod app;
mod cli;
pub mod config;
pub mod context;
pub mod policy;
pub mod providers;
pub mod sessions;
mod telemetry;
pub mod tools;
pub mod ui;

use std::{io::IsTerminal, process::ExitCode};

use crate::{app::Startup, cli::SessionAction, sessions::SessionStore};

#[derive(Debug, thiserror::Error)]
enum StartupError {
    #[error("diagnostics initialization failed")]
    Telemetry(#[from] telemetry::TelemetryError),
    #[error("terminal setup failed")]
    Terminal(#[from] std::io::Error),
    #[error("command line options are invalid")]
    Cli(#[from] cli::CliError),
    #[error("configuration is invalid")]
    Config(#[from] config::ConfigError),
    #[error("workspace could not be selected")]
    Workspace(std::io::Error),
    #[error("saved session could not be used: {0}")]
    Session(#[from] sessions::SessionStoreError),
    #[error("saved session {0} was not found")]
    UnknownSession(String),
    #[cfg(debug_assertions)]
    #[error("simulated startup failure")]
    Simulated,
}

impl StartupError {
    fn kind(&self) -> &'static str {
        match self {
            Self::Telemetry(_) => "telemetry_initialization",
            Self::Terminal(_) => "terminal_setup",
            Self::Cli(_) => "cli_options",
            Self::Config(_) => "configuration",
            Self::Workspace(_) => "workspace_selection",
            Self::Session(_) => "session_storage",
            Self::UnknownSession(_) => "unknown_session",
            #[cfg(debug_assertions)]
            Self::Simulated => "simulated_startup_failure",
        }
    }

    fn action(&self) -> &'static str {
        match self {
            Self::Telemetry(_) => {
                "error: diagnostics are already initialized; restart the application and retry."
            }
            Self::Terminal(_) => "error: terminal setup failed; check terminal access and retry.",
            Self::Cli(_) => {
                "error: command line options could not be read; run ben --help and retry."
            }
            Self::Config(config::ConfigError::MissingApiKey | config::ConfigError::EmptyApiKey) => {
                "error: OPENAI_API_KEY is required; set it in your environment and retry."
            }
            Self::Config(_) => {
                "error: configuration values are invalid; check BEN_MODEL and BEN_* limits and retry."
            }
            Self::Workspace(_) => {
                "error: workspace could not be selected; check the directory and retry."
            }
            Self::Session(_) => {
                "error: the saved session could not be used; run `ben sessions list` and retry."
            }
            Self::UnknownSession(_) => {
                "error: that session is not stored on this machine; run `ben sessions list` and retry."
            }
            #[cfg(debug_assertions)]
            Self::Simulated => {
                "error: startup failed; review the diagnostic output and correct the reported issue."
            }
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(
                error_kind = error.kind(),
                error_detail = %error,
                "application startup failed"
            );
            eprintln!("{error}");
            eprintln!("{}", error.action());
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), StartupError> {
    telemetry::initialize()?;
    tracing::debug!(target: "ben::startup", "startup initialized");

    #[cfg(debug_assertions)]
    if std::env::var_os("BEN_TEST_STARTUP_ERROR").is_some() {
        return Err(StartupError::Simulated);
    }

    let options = cli::parse()?;
    let config = config::Config::load(options.model.as_deref())?;
    std::env::set_current_dir(&options.workspace).map_err(StartupError::Workspace)?;
    tracing::debug!(
        model = %config.model,
        context_token_limit = config.context_token_limit,
        max_tool_calls = config.max_tool_calls,
        max_tool_output_bytes = config.max_tool_output_bytes,
        "configuration loaded"
    );

    let store = SessionStore::for_config(&config);
    tracing::debug!(sessions_root = %store.root().display(), "session store ready");

    let startup = match options.session {
        SessionAction::New => Startup::New,
        SessionAction::List => {
            list_sessions(&store);
            return Ok(());
        }
        SessionAction::Resume { id } => Startup::Resume(store.load(&id)?),
        SessionAction::Clear { id } => {
            // Refuse an unknown or unsafe id before opening the terminal, so the
            // user gets an actionable message instead of a prompt for a
            // session that cannot exist.
            if !store.exists(&id)? {
                return Err(StartupError::UnknownSession(id));
            }
            Startup::Clear(id)
        }
    };

    if std::io::stdout().is_terminal() {
        app::terminal::with_terminal(app::terminal::CrosstermControl, || app::run(store, startup))?;
    }

    Ok(())
}

/// Prints stored session identifiers without entering the alternate screen, so
/// the output can be captured by other tools.
fn list_sessions(store: &SessionStore) {
    let ids = match store.list_ids() {
        Ok(ids) => ids,
        Err(error) => {
            eprintln!("error: stored sessions could not be listed: {error}");
            return;
        }
    };
    if ids.is_empty() {
        println!("No saved sessions.");
        return;
    }
    for id in ids {
        println!("{id}");
    }
}
