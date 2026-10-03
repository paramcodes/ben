pub mod agent;
pub mod app;
mod cli;
pub mod config;
pub mod providers;
mod telemetry;
pub mod ui;

use std::{io::IsTerminal, process::ExitCode};

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
            tracing::error!(error_kind = error.kind(), "application startup failed");
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

    if std::io::stdout().is_terminal() {
        app::terminal::with_terminal(app::terminal::CrosstermControl, app::run)?;
    }

    Ok(())
}
