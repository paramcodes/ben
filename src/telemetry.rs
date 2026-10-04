use std::time::Instant;
use thiserror::Error;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Error)]
pub enum TelemetryError {
    #[error("a global tracing subscriber is already installed")]
    SubscriberAlreadyInstalled,
}

/// Monotonic clock for benchmark time boundaries.
#[derive(Debug, Clone)]
#[expect(dead_code)]
pub struct Stopwatch {
    start: Instant,
}

impl Stopwatch {
    #[expect(dead_code)]
    pub fn start() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    #[expect(dead_code)]
    pub fn elapsed(&self) -> std::time::Duration {
        self.start.elapsed()
    }
}

pub fn initialize() -> Result<(), TelemetryError> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init()
        .map_err(|_| TelemetryError::SubscriberAlreadyInstalled)
}
