mod telemetry;

use std::{ffi::OsStr, process::ExitCode};

#[derive(Debug, thiserror::Error)]
enum StartupError {
    #[error("diagnostics initialization failed")]
    Telemetry(#[from] telemetry::TelemetryError),
    #[cfg(debug_assertions)]
    #[error("simulated startup failure")]
    Simulated,
}

impl StartupError {
    fn kind(&self) -> &'static str {
        match self {
            Self::Telemetry(_) => "telemetry_initialization",
            #[cfg(debug_assertions)]
            Self::Simulated => "simulated_startup_failure",
        }
    }

    fn action(&self) -> &'static str {
        match self {
            Self::Telemetry(_) => {
                "error: diagnostics are already initialized; restart the application and retry."
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

    if std::env::args_os().nth(1).as_deref() == Some(OsStr::new("--version")) {
        println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
    }

    Ok(())
}
