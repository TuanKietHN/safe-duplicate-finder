//! Quarantine inventory and interrupted-transaction queries.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use dedupe_core::{
    DedupeError, Result,
    model::{FileIdentity, FileTransaction, TransactionKind, TransactionState},
    permanent_delete::DeletionEntry,
};
use rusqlite::OptionalExtension;
use serde::Serialize;
use uuid::Uuid;

use crate::Database;

/// User-visible verified quarantine inventory row.
#[derive(Debug, Clone, Serialize)]
pub struct QuarantineEntryRecord {
    /// Entry identifier used for restore.
    pub id: Uuid,
    /// Owning project.
    pub project_id: Uuid,
    /// Original source path.
    pub original_path: PathBuf,
    /// Current quarantine path.
    pub quarantine_path: PathBuf,
    /// Exact bytes.
    pub size_bytes: u64,
    /// Verified/restoring/restored/recovery-required state.
    pub state: String,
    /// Independent irreversible-deletion projection.
    pub permanent_delete_state: String,
    /// Earliest configured retention expiry.
    pub retain_until: String,
    /// Time at which destination verification completed.
    pub quarantined_at: String,
}

/// Transaction and quarantine query repository.
#[derive(Debug, Clone)]
pub struct TransactionRepository {
    database: Database,
}

impl TransactionRepository {
    /// Bind the repository to a database.
    #[must_use]
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    /// List quarantine entries for a project.
    pub fn list_quarantine(&self, project_id: Uuid) -> Result<Vec<QuarantineEntryRecord>> {
        let connection = self.database.connection();
        let mut statement = connection
            .prepare(
                "SELECT id,project_id,original_path,quarantine_path,size_bytes,
                        CASE WHEN permanent_delete_state='active' THEN state
                             ELSE permanent_delete_state END,
                        permanent_delete_state,retain_until,quarantined_at
                 FROM quarantine_entries WHERE project_id=?1 ORDER BY quarantined_at DESC,id",
            )
            .map_err(store_error)?;
        let rows = statement
            .query_map([project_id.to_string()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                ))
            })
            .map_err(store_error)?;
        rows.map(|row| {
            let (
                id,
                project,
                original,
                quarantine,
                size,
                state,
                permanent_delete_state,
                retain_until,
                quarantined_at,
            ) = row.map_err(store_error)?;
            Ok(QuarantineEntryRecord {
                id: parse_uuid(&id, "quarantine entry")?,
                project_id: parse_uuid(&project, "project")?,
                original_path: PathBuf::from(original),
                quarantine_path: PathBuf::from(quarantine),
                size_bytes: to_u64(size, "kích thước cách ly")?,
                state,
                permanent_delete_state,
                retain_until,
                quarantined_at,
            })
        })
        .collect()
    }

    /// Select verified entries belonging to one scan session for an explicit batch restore.
    pub fn verified_entries_for_session(&self, session_id: Uuid) -> Result<Vec<Uuid>> {
        self.verified_entry_ids(
            "SELECT q.id FROM quarantine_entries q
             JOIN file_transactions t ON t.id=q.origin_transaction_id
             WHERE q.state='verified' AND q.permanent_delete_state='active'
               AND t.session_id=?1 ORDER BY q.quarantined_at,q.id",
            session_id,
        )
    }

    /// Select verified entries belonging to one duplicate group for an explicit batch restore.
    pub fn verified_entries_for_group(&self, group_id: Uuid) -> Result<Vec<Uuid>> {
        self.verified_entry_ids(
            "SELECT q.id FROM quarantine_entries q
             JOIN file_transactions t ON t.id=q.origin_transaction_id
             JOIN plan_items p ON p.id=t.plan_item_id
             WHERE q.state='verified' AND q.permanent_delete_state='active'
               AND p.group_id=?1 ORDER BY q.quarantined_at,q.id",
            group_id,
        )
    }

    /// Select every currently verified entry for one project for an explicit batch restore.
    pub fn verified_entries_for_project(&self, project_id: Uuid) -> Result<Vec<Uuid>> {
        self.verified_entry_ids(
            "SELECT q.id FROM quarantine_entries q
             WHERE q.state='verified' AND q.permanent_delete_state='active'
               AND q.project_id=?1 ORDER BY q.quarantined_at,q.id",
            project_id,
        )
    }

    fn verified_entry_ids(&self, sql: &str, scope_id: Uuid) -> Result<Vec<Uuid>> {
        let connection = self.database.connection();
        let mut statement = connection.prepare(sql).map_err(store_error)?;
        let rows = statement
            .query_map([scope_id.to_string()], |row| row.get::<_, String>(0))
            .map_err(store_error)?;
        rows.map(|row| {
            let id = row.map_err(store_error)?;
            parse_uuid(&id, "quarantine entry")
        })
        .collect()
    }

    /// Load the verified originating quarantine transaction used to construct a restore.
    pub fn verified_quarantine_transaction(&self, entry_id: Uuid) -> Result<FileTransaction> {
        let transaction = self
            .database
            .connection()
            .query_row(
                "SELECT t.id,t.project_id,t.session_id,t.plan_item_id,t.source_path,
                        t.destination_path,t.volume_id,t.file_id,t.size_bytes,t.blake3_digest,
                        t.sha256_digest,t.source_snapshot_token,t.started_at
                 FROM quarantine_entries q JOIN file_transactions t
                   ON t.id=q.origin_transaction_id
                 WHERE q.id=?1 AND q.state='verified'
                   AND q.permanent_delete_state='active' AND t.status='verified'",
                [entry_id.to_string()],
                map_transaction_row,
            )
            .optional()
            .map_err(store_error)?
            .ok_or_else(|| {
                DedupeError::Safety(format!(
                    "Mục cách ly {entry_id} chưa được xác minh hoặc không tồn tại"
                ))
            })?;
        transaction.try_into_model(TransactionKind::Quarantine, TransactionState::Verified)
    }

    /// Load only explicit UUID-selected, currently verified quarantine entries for deletion.
    /// Original/source paths are intentionally not returned in the deletion domain type.
    pub fn deletion_entries(&self, entry_ids: &[Uuid]) -> Result<Vec<DeletionEntry>> {
        if entry_ids.is_empty() {
            return Err(DedupeError::InvalidInput(
                "Phải chọn ít nhất một UUID mục cách ly".into(),
            ));
        }
        let mut unique = std::collections::HashSet::with_capacity(entry_ids.len());
        if entry_ids.iter().any(|entry| !unique.insert(*entry)) {
            return Err(DedupeError::InvalidInput(
                "UUID mục cách ly bị lặp trong lựa chọn rõ ràng".into(),
            ));
        }
        let connection = self.database.connection();
        let mut statement = connection
            .prepare(
                "SELECT q.id,q.project_id,q.quarantine_path,q.volume_id,q.file_id,
                        q.size_bytes,q.blake3_digest,q.sha256_digest,q.retain_until
                 FROM quarantine_entries q JOIN file_transactions t
                   ON t.id=q.origin_transaction_id
                 WHERE q.id=?1 AND q.state='verified'
                   AND q.permanent_delete_state='active' AND t.status='verified'",
            )
            .map_err(store_error)?;
        let mut entries = Vec::with_capacity(entry_ids.len());
        for entry_id in entry_ids {
            let stored = statement
                .query_row([entry_id.to_string()], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, Vec<u8>>(6)?,
                        row.get::<_, Vec<u8>>(7)?,
                        row.get::<_, String>(8)?,
                    ))
                })
                .optional()
                .map_err(store_error)?
                .ok_or_else(|| {
                    DedupeError::Safety(format!(
                        "Mục cách ly {entry_id} chưa được xác minh, không hoạt động hoặc không tồn tại"
                    ))
                })?;
            entries.push(DeletionEntry {
                id: parse_uuid(&stored.0, "quarantine entry")?,
                project_id: parse_uuid(&stored.1, "project")?,
                quarantine_path: PathBuf::from(stored.2),
                identity: FileIdentity {
                    volume_id: stored.3,
                    file_id: stored.4,
                },
                size_bytes: to_u64(stored.5, "kích thước cách ly")?,
                blake3: stored.6,
                sha256: stored.7,
                retain_until: DateTime::parse_from_rfc3339(&stored.8)
                    .map_err(|error| {
                        DedupeError::State(format!("Dấu thời gian lưu giữ không hợp lệ: {error}"))
                    })?
                    .with_timezone(&Utc),
            });
        }
        Ok(entries)
    }

    /// List transactions that startup recovery must inspect.
    pub fn pending_recovery(&self, project_id: Uuid) -> Result<Vec<FileTransaction>> {
        let connection = self.database.connection();
        let mut statement = connection
            .prepare(
                "SELECT id,project_id,session_id,plan_item_id,source_path,destination_path,
                        volume_id,file_id,size_bytes,blake3_digest,sha256_digest,
                        source_snapshot_token,started_at,kind,status
                 FROM file_transactions
                 WHERE project_id=?1 AND status IN (
                    'planned','preflight_validated','moving','moved_unverified','move_failed',
                    'verify_failed','recovery_required'
                 ) ORDER BY updated_at,id",
            )
            .map_err(store_error)?;
        let rows = statement
            .query_map([project_id.to_string()], |row| {
                let stored = StoredTransaction {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    session_id: row.get(2)?,
                    plan_item_id: row.get(3)?,
                    source: row.get(4)?,
                    destination: row.get(5)?,
                    volume_id: row.get(6)?,
                    file_id: row.get(7)?,
                    size_bytes: row.get(8)?,
                    blake3: row.get(9)?,
                    sha256: row.get(10)?,
                    snapshot_token: row.get(11)?,
                    started_at: row.get(12)?,
                };
                Ok((stored, row.get::<_, String>(13)?, row.get::<_, String>(14)?))
            })
            .map_err(store_error)?;
        rows.map(|row| {
            let (stored, kind, state) = row.map_err(store_error)?;
            stored.try_into_model(parse_kind(&kind)?, parse_state(&state)?)
        })
        .collect()
    }

    /// Load a single interrupted transaction for explicit reconciliation.
    pub fn transaction(&self, transaction_id: Uuid) -> Result<FileTransaction> {
        let connection = self.database.connection();
        let pair = connection
            .query_row(
                "SELECT id,project_id,session_id,plan_item_id,source_path,destination_path,
                        volume_id,file_id,size_bytes,blake3_digest,sha256_digest,
                        source_snapshot_token,started_at,kind,status
                 FROM file_transactions WHERE id=?1",
                [transaction_id.to_string()],
                |row| {
                    Ok((
                        StoredTransaction {
                            id: row.get(0)?,
                            project_id: row.get(1)?,
                            session_id: row.get(2)?,
                            plan_item_id: row.get(3)?,
                            source: row.get(4)?,
                            destination: row.get(5)?,
                            volume_id: row.get(6)?,
                            file_id: row.get(7)?,
                            size_bytes: row.get(8)?,
                            blake3: row.get(9)?,
                            sha256: row.get(10)?,
                            snapshot_token: row.get(11)?,
                            started_at: row.get(12)?,
                        },
                        row.get::<_, String>(13)?,
                        row.get::<_, String>(14)?,
                    ))
                },
            )
            .optional()
            .map_err(store_error)?
            .ok_or_else(|| {
                DedupeError::InvalidInput(format!(
                    "Không nhận diện được giao dịch {transaction_id}"
                ))
            })?;
        pair.0
            .try_into_model(parse_kind(&pair.1)?, parse_state(&pair.2)?)
    }
}

struct StoredTransaction {
    id: String,
    project_id: String,
    session_id: Option<String>,
    plan_item_id: Option<String>,
    source: String,
    destination: String,
    volume_id: String,
    file_id: String,
    size_bytes: i64,
    blake3: Vec<u8>,
    sha256: Vec<u8>,
    snapshot_token: Vec<u8>,
    started_at: String,
}

impl StoredTransaction {
    fn try_into_model(
        self,
        kind: TransactionKind,
        state: TransactionState,
    ) -> Result<FileTransaction> {
        Ok(FileTransaction {
            id: parse_uuid(&self.id, "transaction")?,
            project_id: parse_uuid(&self.project_id, "project")?,
            session_id: self
                .session_id
                .as_deref()
                .map(|value| parse_uuid(value, "session"))
                .transpose()?,
            plan_item_id: self
                .plan_item_id
                .as_deref()
                .map(|value| parse_uuid(value, "plan item"))
                .transpose()?,
            kind,
            state,
            source: PathBuf::from(self.source),
            destination: PathBuf::from(self.destination),
            identity: FileIdentity {
                volume_id: self.volume_id,
                file_id: self.file_id,
            },
            size_bytes: to_u64(self.size_bytes, "kích thước giao dịch")?,
            blake3: self.blake3,
            sha256: self.sha256,
            snapshot_token: fixed_token(&self.snapshot_token)?,
            started_at: DateTime::parse_from_rfc3339(&self.started_at)
                .map_err(|error| {
                    DedupeError::State(format!("Dấu thời gian giao dịch không hợp lệ: {error}"))
                })?
                .with_timezone(&Utc),
        })
    }
}

fn map_transaction_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredTransaction> {
    Ok(StoredTransaction {
        id: row.get(0)?,
        project_id: row.get(1)?,
        session_id: row.get(2)?,
        plan_item_id: row.get(3)?,
        source: row.get(4)?,
        destination: row.get(5)?,
        volume_id: row.get(6)?,
        file_id: row.get(7)?,
        size_bytes: row.get(8)?,
        blake3: row.get(9)?,
        sha256: row.get(10)?,
        snapshot_token: row.get(11)?,
        started_at: row.get(12)?,
    })
}

fn parse_kind(value: &str) -> Result<TransactionKind> {
    match value {
        "quarantine" => Ok(TransactionKind::Quarantine),
        "restore" => Ok(TransactionKind::Restore),
        _ => Err(DedupeError::State(format!(
            "Không nhận diện được loại giao dịch: {value}"
        ))),
    }
}

fn parse_state(value: &str) -> Result<TransactionState> {
    match value {
        "planned" => Ok(TransactionState::Planned),
        "preflight_validated" => Ok(TransactionState::PreflightValidated),
        "moving" => Ok(TransactionState::Moving),
        "moved_unverified" => Ok(TransactionState::MovedUnverified),
        "verified" => Ok(TransactionState::Verified),
        "preflight_failed" => Ok(TransactionState::PreflightFailed),
        "move_failed" => Ok(TransactionState::MoveFailed),
        "verify_failed" => Ok(TransactionState::VerifyFailed),
        "recovery_required" => Ok(TransactionState::RecoveryRequired),
        "cancelled" => Ok(TransactionState::Cancelled),
        "reconciled_source_only" => Ok(TransactionState::ReconciledSourceOnly),
        "reconciled_both" => Ok(TransactionState::ReconciledBoth),
        "reconciled_missing" => Ok(TransactionState::ReconciledMissing),
        _ => Err(DedupeError::State(format!(
            "Không hỗ trợ trạng thái giao dịch: {value}"
        ))),
    }
}

fn fixed_token(value: &[u8]) -> Result<[u8; 32]> {
    value
        .try_into()
        .map_err(|_| DedupeError::State("Token ảnh chụp đã lưu có độ dài sai".into()))
}

fn parse_uuid(value: &str, kind: &str) -> Result<Uuid> {
    let label = match kind {
        "quarantine entry" => "mục cách ly",
        "project" => "dự án",
        "transaction" => "giao dịch",
        "session" => "phiên quét",
        "plan item" => "mục kế hoạch",
        _ => kind,
    };
    Uuid::parse_str(value)
        .map_err(|error| DedupeError::State(format!("UUID {label} đã lưu không hợp lệ: {error}")))
}

fn to_u64(value: i64, field: &str) -> Result<u64> {
    u64::try_from(value).map_err(|_| DedupeError::State(format!("Giá trị {field} đã lưu là số âm")))
}

fn store_error(error: rusqlite::Error) -> DedupeError {
    DedupeError::Durability(error.to_string())
}
