//! Safety-first duplicate-file detection and recoverable mutation engine.

pub mod control;
pub mod dry_run;
pub mod duplicate_detector;
pub mod error;
pub mod filters;
pub mod full_hash;
pub mod keep_policy;
pub mod logging;
pub mod metadata;
pub mod model;
pub mod path_normalization;
pub mod permanent_delete;
pub mod ports;
pub mod progress;
pub mod project_manager;
pub mod quarantine;
pub mod quick_hash;
pub mod recovery;
pub mod restore;
pub mod scanner;
pub mod scheduler;
pub mod transaction_journal;

pub use error::{DedupeError, Result};
