//! Paginated, durable file-processing history built from existing scan and transaction evidence.

use std::path::PathBuf;

use dedupe_core::{DedupeError, Result};
use rusqlite::params;
use serde::Serialize;
use uuid::Uuid;

use crate::Database;

/// One processed file together with duplicate and mutation evidence, when present.
#[derive(Debug, Clone, Serialize)]
pub struct FileHistoryRecord {
    /// Immutable snapshot identifier.
    pub snapshot_id: Uuid,
    /// Scan session that observed the file.
    pub session_id: Uuid,
    /// Original path observed during the read-only scan.
    pub path: PathBuf,
    /// Exact observed size.
    pub size_bytes: u64,
    /// Processing state reached by the snapshot.
    pub state: String,
    /// Access result captured during metadata collection.
    pub access_status: String,
    /// First durable observation timestamp.
    pub observed_at: String,
    /// Processing completion timestamp, if reached.
    pub completed_at: Option<String>,
    /// Proven duplicate group, if this file was a duplicate candidate.
    pub group_id: Option<Uuid>,
    /// Current recommendation or sealed-plan action.
    pub action: Option<String>,
    /// Human-readable keeper-policy reason.
    pub reason: Option<String>,
    /// State of the newest plan containing this snapshot.
    pub plan_status: Option<String>,
    /// Latest quarantine transaction state for this plan item.
    pub transaction_status: Option<String>,
    /// Destination reserved or used by the quarantine transaction.
    pub quarantine_path: Option<PathBuf>,
    /// Current verified quarantine projection.
    pub quarantine_state: Option<String>,
    /// Independent permanent-deletion projection.
    pub permanent_delete_state: Option<String>,
    /// Other paths proven to contain the same duplicate content.
    pub duplicate_locations: Vec<PathBuf>,
}

/// Filtered history page plus project-wide totals.
#[derive(Debug, Clone, Serialize)]
pub struct FileHistoryPage {
    /// Total snapshots matching the current search and duplicate-only filter.
    pub total_matching: u64,
    /// All durable snapshots recorded for the project.
    pub total_processed: u64,
    /// Snapshots belonging to proven duplicate groups.
    pub duplicate_files: u64,
    /// Proven duplicate groups recorded for the project.
    pub duplicate_groups: u64,
    /// Snapshots that ended with an error or inaccessible state.
    pub problem_files: u64,
    /// Requested records ordered newest first.
    pub items: Vec<FileHistoryRecord>,
}

/// Read-only history repository.
#[derive(Debug, Clone)]
pub struct HistoryRepository {
    database: Database,
}

impl HistoryRepository {
    /// Bind history queries to a database.
    #[must_use]
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    /// List one bounded page without copying file content or duplicating audit data.
    pub fn list_files(
        &self,
        project_id: Uuid,
        search: &str,
        duplicate_only: bool,
        offset: u64,
        limit: u64,
    ) -> Result<FileHistoryPage> {
        let limit = limit.clamp(1, 200);
        let offset = i64::try_from(offset)
            .map_err(|_| DedupeError::InvalidInput("Vị trí phân trang quá lớn".into()))?;
        let limit = i64::try_from(limit)
            .map_err(|_| DedupeError::InvalidInput("Kích thước trang quá lớn".into()))?;
        let project = project_id.to_string();
        let connection = self.database.read_connection().map_err(store_error)?;
        let totals = project_totals(&connection, &project)?;
        let count_sql = if duplicate_only {
            DUPLICATE_COUNT_SQL
        } else {
            ALL_COUNT_SQL
        };
        let total_matching: i64 = connection
            .query_row(count_sql, params![project, search.trim()], |row| row.get(0))
            .map_err(store_error)?;
        let items = history_rows(
            &connection,
            &project,
            search.trim(),
            duplicate_only,
            offset,
            limit,
        )?;
        Ok(FileHistoryPage {
            total_matching: to_u64(total_matching, "tổng lịch sử phù hợp")?,
            total_processed: to_u64(totals.0, "tổng tệp đã xử lý")?,
            duplicate_files: to_u64(totals.1, "tổng tệp trùng")?,
            duplicate_groups: to_u64(totals.2, "tổng nhóm trùng")?,
            problem_files: to_u64(totals.3, "tổng tệp có vấn đề")?,
            items,
        })
    }
}

fn project_totals(
    connection: &rusqlite::Connection,
    project_id: &str,
) -> Result<(i64, i64, i64, i64)> {
    connection
        .query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(CASE WHEN dm.group_id IS NULL THEN 0 ELSE 1 END),0),
                    COUNT(DISTINCT dm.group_id),
                    COALESCE(SUM(CASE WHEN s.state='error'
                                           OR s.access_status IN ('denied','offline','missing','error')
                                      THEN 1 ELSE 0 END),0)
             FROM file_snapshots s
             JOIN file_entries e ON e.id=s.file_entry_id
             LEFT JOIN duplicate_members dm ON dm.snapshot_id=s.id
             WHERE e.project_id=?1",
            [project_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(store_error)
}

fn history_rows(
    connection: &rusqlite::Connection,
    project_id: &str,
    search: &str,
    duplicate_only: bool,
    offset: i64,
    limit: i64,
) -> Result<Vec<FileHistoryRecord>> {
    let sql = if duplicate_only {
        DUPLICATE_HISTORY_SQL
    } else {
        ALL_HISTORY_SQL
    };
    let mut statement = connection.prepare(sql).map_err(store_error)?;
    let rows = statement
        .query_map(params![project_id, search, limit, offset], map_raw_row)
        .map_err(store_error)?;
    rows.map(map_history_row).collect()
}

#[allow(clippy::type_complexity)]
fn map_raw_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<(
    String,
    String,
    String,
    i64,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
)> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
        row.get(13)?,
        row.get(14)?,
        row.get(15)?,
        row.get(16)?,
    ))
}

#[allow(clippy::type_complexity)]
fn map_history_row(
    row: rusqlite::Result<(
        String,
        String,
        String,
        i64,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )>,
) -> Result<FileHistoryRecord> {
    let value = row.map_err(store_error)?;
    Ok(FileHistoryRecord {
        snapshot_id: parse_uuid(&value.0, "snapshot")?,
        session_id: parse_uuid(&value.1, "session")?,
        path: PathBuf::from(value.2),
        size_bytes: to_u64(value.3, "kích thước lịch sử")?,
        state: value.4,
        access_status: value.5,
        observed_at: value.6,
        completed_at: value.7,
        group_id: value.8.map(|id| parse_uuid(&id, "group")).transpose()?,
        action: value.9,
        reason: value.10,
        plan_status: value.11,
        transaction_status: value.12,
        quarantine_path: value.13.map(PathBuf::from),
        quarantine_state: value.14,
        permanent_delete_state: value.15,
        duplicate_locations: value
            .16
            .map(|paths| paths.lines().map(PathBuf::from).collect())
            .unwrap_or_default(),
    })
}

const DUPLICATE_HISTORY_SQL: &str = "
    SELECT s.id,s.session_id,e.original_path,s.size_bytes,s.state,s.access_status,
           s.observed_at,s.completed_at,dm.group_id,COALESCE(pi.action,dm.recommendation),
           dm.reason,p.status,qt.status,qt.destination_path,q.state,q.permanent_delete_state,
           (SELECT group_concat(e2.original_path,char(10))
              FROM duplicate_members dm2
              JOIN file_snapshots s2 ON s2.id=dm2.snapshot_id
              JOIN file_entries e2 ON e2.id=s2.file_entry_id
             WHERE dm2.group_id=dm.group_id AND dm2.snapshot_id<>s.id)
      FROM duplicate_members dm
      CROSS JOIN file_snapshots s ON s.id=dm.snapshot_id
      JOIN file_entries e ON e.id=s.file_entry_id
      LEFT JOIN plan_items pi ON pi.snapshot_id=s.id AND pi.group_id=dm.group_id
       AND pi.plan_id=(SELECT newest.id FROM operation_plans newest
                        WHERE newest.session_id=s.session_id
                        ORDER BY COALESCE(newest.sealed_at,newest.created_at) DESC,newest.id DESC
                        LIMIT 1)
      LEFT JOIN operation_plans p ON p.id=pi.plan_id
      LEFT JOIN file_transactions qt ON qt.plan_item_id=pi.id AND qt.kind='quarantine'
      LEFT JOIN quarantine_entries q ON q.origin_transaction_id=qt.id
     WHERE e.project_id=?1
       AND (?2='' OR instr(lower(e.original_path),lower(?2))>0
                    OR instr(lower(dm.group_id),lower(?2))>0)
     ORDER BY s.observed_at DESC,e.original_path,s.id
     LIMIT ?3 OFFSET ?4";

const ALL_HISTORY_SQL: &str = "
    SELECT s.id,s.session_id,e.original_path,s.size_bytes,s.state,s.access_status,
           s.observed_at,s.completed_at,dm.group_id,COALESCE(pi.action,dm.recommendation),
           dm.reason,p.status,qt.status,qt.destination_path,q.state,q.permanent_delete_state,
           (SELECT group_concat(e2.original_path,char(10))
              FROM duplicate_members dm2
              JOIN file_snapshots s2 ON s2.id=dm2.snapshot_id
              JOIN file_entries e2 ON e2.id=s2.file_entry_id
             WHERE dm2.group_id=dm.group_id AND dm2.snapshot_id<>s.id)
      FROM file_snapshots s
      JOIN file_entries e ON e.id=s.file_entry_id
      LEFT JOIN duplicate_members dm ON dm.snapshot_id=s.id
      LEFT JOIN plan_items pi ON pi.snapshot_id=s.id AND pi.group_id=dm.group_id
       AND pi.plan_id=(SELECT newest.id FROM operation_plans newest
                        WHERE newest.session_id=s.session_id
                        ORDER BY COALESCE(newest.sealed_at,newest.created_at) DESC,newest.id DESC
                        LIMIT 1)
      LEFT JOIN operation_plans p ON p.id=pi.plan_id
      LEFT JOIN file_transactions qt ON qt.plan_item_id=pi.id AND qt.kind='quarantine'
      LEFT JOIN quarantine_entries q ON q.origin_transaction_id=qt.id
     WHERE e.project_id=?1
       AND (?2='' OR instr(lower(e.original_path),lower(?2))>0
                    OR instr(lower(COALESCE(dm.group_id,'')),lower(?2))>0)
     ORDER BY s.observed_at DESC,e.original_path,s.id
     LIMIT ?3 OFFSET ?4";

const DUPLICATE_COUNT_SQL: &str = "
    SELECT COUNT(*)
      FROM duplicate_members dm
      CROSS JOIN file_snapshots s ON s.id=dm.snapshot_id
      JOIN file_entries e ON e.id=s.file_entry_id
     WHERE e.project_id=?1
       AND (?2='' OR instr(lower(e.original_path),lower(?2))>0
                    OR instr(lower(dm.group_id),lower(?2))>0)";

const ALL_COUNT_SQL: &str = "
    SELECT COUNT(*)
      FROM file_snapshots s
      JOIN file_entries e ON e.id=s.file_entry_id
      LEFT JOIN duplicate_members dm ON dm.snapshot_id=s.id
     WHERE e.project_id=?1
       AND (?2='' OR instr(lower(e.original_path),lower(?2))>0
                    OR instr(lower(COALESCE(dm.group_id,'')),lower(?2))>0)";

fn parse_uuid(value: &str, label: &str) -> Result<Uuid> {
    Uuid::parse_str(value).map_err(|error| {
        DedupeError::Durability(format!("UUID {label} không hợp lệ trong kho: {error}"))
    })
}

fn to_u64(value: i64, label: &str) -> Result<u64> {
    u64::try_from(value)
        .map_err(|_| DedupeError::Durability(format!("{label} âm hoặc vượt phạm vi")))
}

fn store_error(error: impl std::fmt::Display) -> DedupeError {
    DedupeError::Durability(error.to_string())
}
