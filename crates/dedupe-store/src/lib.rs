//! `SQLite` WAL persistence adapter.

pub mod database;
pub mod duplicates;
pub mod history;
pub mod journal;
pub mod permanent_delete;
pub mod plans;
pub mod projects;
pub mod scan;
pub mod transactions;

pub use database::{Database, DatabaseMaintenance};
pub use duplicates::DuplicateRepository;
pub use history::{FileHistoryPage, FileHistoryRecord, HistoryRepository};
pub use journal::SqliteTransactionJournal;
pub use permanent_delete::{PermanentDeleteRepository, SqlitePermanentDeleteJournal};
pub use plans::{LatestPlanContext, PlanRepository, PlanSummary, PlannedQuarantineItem};
pub use projects::{ProjectRecord, ProjectRepository, ProjectRootRecord};
pub use scan::{
    ScanControlMonitor, ScanControlRequest, ScanRepository, ScanResumeSpec, ScanSessionRecord,
    SqliteScanSink,
};
pub use transactions::{QuarantineEntryRecord, TransactionRepository};
