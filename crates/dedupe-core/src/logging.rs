//! Local structured and human-readable application logging.

use std::path::Path;

use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::{DedupeError, Result};

/// Guards that keep non-blocking log writers alive and flush them on drop.
#[derive(Debug)]
pub struct LogGuards {
    _json: tracing_appender::non_blocking::WorkerGuard,
    _text: tracing_appender::non_blocking::WorkerGuard,
}

/// Initialize daily JSONL and text logs under an explicit local directory.
///
/// File content is never accepted by this API; callers log only operation metadata and paths.
pub fn init(log_directory: &Path) -> Result<LogGuards> {
    std::fs::create_dir_all(log_directory)
        .map_err(|error| DedupeError::io("tạo thư mục nhật ký", log_directory, error))?;
    let json_appender = tracing_appender::rolling::daily(log_directory, "safe-dedupe.jsonl");
    let text_appender = tracing_appender::rolling::daily(log_directory, "safe-dedupe.log");
    let (json_writer, json_guard) = tracing_appender::non_blocking(json_appender);
    let (text_writer, text_guard) = tracing_appender::non_blocking(text_appender);
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_ansi(false)
                .with_writer(json_writer),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .compact()
                .with_ansi(false)
                .with_writer(text_writer),
        )
        .try_init()
        .map_err(|error| DedupeError::State(format!("Nhật ký đã được khởi tạo: {error}")))?;
    Ok(LogGuards {
        _json: json_guard,
        _text: text_guard,
    })
}
