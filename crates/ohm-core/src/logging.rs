//! Logging setup shared by the CLI, the desktop backend and the test harness.
//!
//! Logs go to `stderr` (so the CLI stays pipeable) and, when a [`ConfigPaths`]
//! is provided, to a daily rolling file in `<config>/logs`.
//!
//! Every hardware write is additionally recorded in the audit log — see
//! `ohm_runtime::audit`. Logging is local only: OpenHardwareOS sends nothing
//! anywhere.

use std::str::FromStr;

use serde::{Deserialize, Serialize};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::error::{OhmError, Result};
use crate::paths::ConfigPaths;

/// Log verbosity, surfaced in Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub const ALL: [LogLevel; 5] = [
        LogLevel::Error,
        LogLevel::Warn,
        LogLevel::Info,
        LogLevel::Debug,
        LogLevel::Trace,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }

    /// Verbosity as understood by [`tracing_subscriber::EnvFilter`].
    pub fn as_filter(&self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for LogLevel {
    type Err = OhmError;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "error" | "err" => Ok(Self::Error),
            "warn" | "warning" => Ok(Self::Warn),
            "info" => Ok(Self::Info),
            "debug" => Ok(Self::Debug),
            "trace" => Ok(Self::Trace),
            other => Err(OhmError::Config(format!(
                "unknown log level `{other}` (expected one of error, warn, info, debug, trace)"
            ))),
        }
    }
}

/// Keeps the non-blocking log writer alive for as long as the app runs.
#[derive(Debug, Default)]
pub struct LogGuard {
    _file: Option<tracing_appender::non_blocking::WorkerGuard>,
}

/// Initialise global logging. Calling it twice is harmless: the second call is
/// a no-op, which keeps tests and the desktop backend simple.
///
/// `RUST_LOG` takes precedence over `level` when set.
pub fn init(level: LogLevel, paths: Option<&ConfigPaths>) -> Result<LogGuard> {
    let filter = match std::env::var("RUST_LOG") {
        Ok(value) if !value.trim().is_empty() => EnvFilter::new(value),
        _ => EnvFilter::try_new(level.as_filter()).unwrap_or_else(|_| EnvFilter::new("info")),
    };

    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_writer(std::io::stderr);

    let (file_layer, guard) = match paths {
        Some(paths) => {
            paths.ensure()?;
            let appender = tracing_appender::rolling::daily(paths.logs_dir(), "openhardwareos.log");
            let (writer, guard) = tracing_appender::non_blocking(appender);
            let layer = tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_target(true)
                .with_writer(writer);
            (Some(layer), Some(guard))
        }
        None => (None, None),
    };

    let registry = tracing_subscriber::registry()
        .with(filter)
        .with(stderr_layer);
    let result = match file_layer {
        Some(layer) => registry.with(layer).try_init(),
        None => registry.try_init(),
    };

    match result {
        Ok(()) => Ok(LogGuard { _file: guard }),
        Err(_already_initialised) => Ok(LogGuard { _file: guard }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_levels() {
        assert_eq!("WARN".parse::<LogLevel>().unwrap(), LogLevel::Warn);
        assert_eq!("debug".parse::<LogLevel>().unwrap(), LogLevel::Debug);
        assert!("loud".parse::<LogLevel>().is_err());
    }

    #[test]
    fn default_is_info() {
        assert_eq!(LogLevel::default(), LogLevel::Info);
        assert_eq!(LogLevel::default().as_str(), "info");
        assert_eq!(LogLevel::ALL.len(), 5);
    }

    #[test]
    fn init_without_paths_is_safe_twice() {
        init(LogLevel::Error, None).unwrap();
        init(LogLevel::Error, None).unwrap();
    }

    #[test]
    fn init_writes_log_files_when_paths_given() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = ConfigPaths::from_root(tmp.path());
        let _guard = init(LogLevel::Info, Some(&paths)).unwrap();
        tracing::info!("hello from the test suite");
        assert!(paths.logs_dir().is_dir());
    }
}
