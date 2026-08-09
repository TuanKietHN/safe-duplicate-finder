//! `SQLite` projection plus fsynced append-only manifest for irreversible quarantine deletion.

use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use dedupe_core::{
    DedupeError, Result,
    model::FileIdentity,
    permanent_delete::{
        DeletionEntry, PermanentDeleteBatch, PermanentDeleteBatchState, PermanentDeleteItem,
        PermanentDeleteItemState, PermanentDeleteMode,
    },
    ports::PermanentDeleteJournal,
};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use uuid::Uuid;

use crate::{Database, TransactionRepository};

/// Explicit-selection loader for the quarantine-only deletion domain.
#[derive(Debug, Clone)]
pub struct PermanentDeleteRepository {
    database: Database,
}

impl PermanentDeleteRepository {
    /// Bind the repository to a database.
    #[must_use]
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    /// Load exactly the individually selected verified quarantine UUIDs.
    pub fn selected_entries(&self, entry_ids: &[Uuid]) -> Result<Vec<DeletionEntry>> {
        TransactionRepository::new(self.database.clone()).deletion_entries(entry_ids)
    }
}

/// Durable permanent-delete journal. Every event is appended and fsynced before `SQLite` changes.
#[derive(Debug, Clone)]
pub struct SqlitePermanentDeleteJournal {
    database: Database,
    manifest_path: PathBuf,
}

impl SqlitePermanentDeleteJournal {
    /// Create a journal whose manifest lives outside source and quarantine roots.
    pub fn new(database: Database, manifest_path: impl Into<PathBuf>) -> Result<Self> {
        let manifest_path = manifest_path.into();
        if let Some(parent) = manifest_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                DedupeError::io("tạo thư mục manifest xóa vĩnh viễn", parent, error)
            })?;
        }
        Ok(Self {
            database,
            manifest_path,
        })
    }

    /// Manifest path for recovery inspection.
    #[must_use]
    pub fn manifest_path(&self) -> &Path {
        &self.manifest_path
    }

    fn append_manifest(&self, event: &DeleteManifestEvent<'_>) -> Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.manifest_path)
            .map_err(|error| {
                DedupeError::io("mở manifest xóa vĩnh viễn", &self.manifest_path, error)
            })?;
        serde_json::to_writer(&mut file, event)?;
        file.write_all(b"\n").map_err(|error| {
            DedupeError::io("ghi nối manifest xóa vĩnh viễn", &self.manifest_path, error)
        })?;
        file.flush().map_err(|error| {
            DedupeError::io(
                "đẩy dữ liệu manifest xóa vĩnh viễn",
                &self.manifest_path,
                error,
            )
        })?;
        file.sync_data().map_err(|error| {
            DedupeError::io("đồng bộ manifest xóa vĩnh viễn", &self.manifest_path, error)
        })?;
        Ok(())
    }
}

impl PermanentDeleteJournal for SqlitePermanentDeleteJournal {
    fn create_batch(&self, batch: &PermanentDeleteBatch) -> Result<()> {
        if batch.state != PermanentDeleteBatchState::Prepared
            || batch
                .items
                .iter()
                .any(|item| item.state != PermanentDeleteItemState::Planned)
        {
            return Err(DedupeError::State(
                "Tạo nhật ký xóa vĩnh viễn yêu cầu lô đã được lên kế hoạch đầy đủ".into(),
            ));
        }
        let occurred_at = batch.created_at.to_rfc3339();
        self.append_manifest(&DeleteManifestEvent::batch(
            batch,
            None,
            PermanentDeleteBatchState::Prepared,
            None,
            occurred_at.clone(),
        ))?;
        let mut connection = self.database.connection();
        let transaction = connection.transaction().map_err(store_error)?;
        transaction
            .execute(
                "INSERT INTO permanent_delete_batches (
                    id,project_id,status,deletion_mode,token_digest,selection_digest,
                    confirmation_phrase,entry_count,total_bytes,created_at,expires_at
                 ) VALUES (?1,?2,'prepared',?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    batch.id.to_string(),
                    batch.project_id.to_string(),
                    delete_mode_name(batch.mode),
                    batch.token_digest.as_slice(),
                    batch.selection_digest.as_slice(),
                    batch.confirmation_phrase,
                    to_i64(batch.entry_count, "số mục xóa vĩnh viễn")?,
                    to_i64(batch.total_bytes, "số byte xóa vĩnh viễn")?,
                    occurred_at,
                    batch.expires_at.to_rfc3339(),
                ],
            )
            .map_err(store_error)?;
        for (ordinal, item) in batch.items.iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO permanent_delete_items (
                        batch_id,entry_id,ordinal,status,quarantine_path,volume_id,file_id,
                        size_bytes,blake3_digest,sha256_digest,retain_until,updated_at
                     ) VALUES (?1,?2,?3,'planned',?4,?5,?6,?7,?8,?9,?10,?11)",
                    params![
                        batch.id.to_string(),
                        item.entry.id.to_string(),
                        i64::try_from(ordinal).map_err(|error| {
                            DedupeError::Safety(format!("Số thứ tự mục xóa bị tràn: {error}"))
                        })?,
                        item.entry.quarantine_path.to_string_lossy(),
                        item.entry.identity.volume_id,
                        item.entry.identity.file_id,
                        to_i64(item.entry.size_bytes, "số byte của mục xóa vĩnh viễn")?,
                        item.entry.blake3,
                        item.entry.sha256,
                        item.entry.retain_until.to_rfc3339(),
                        occurred_at,
                    ],
                )
                .map_err(store_error)?;
        }
        insert_event(
            &transaction,
            batch.id,
            None,
            None,
            "prepared",
            "batch",
            None,
            &occurred_at,
        )?;
        insert_audit(
            &transaction,
            batch,
            None,
            "permanent_delete.prepared",
            None,
            &occurred_at,
        )?;
        transaction.commit().map_err(store_error)
    }

    fn load_batch(&self, batch_id: Uuid) -> Result<PermanentDeleteBatch> {
        let connection = self.database.connection();
        let stored = connection
            .query_row(
                "SELECT id,project_id,status,token_digest,selection_digest,confirmation_phrase,
                        entry_count,total_bytes,created_at,expires_at,deletion_mode
                 FROM permanent_delete_batches WHERE id=?1",
                [batch_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                        row.get::<_, Vec<u8>>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, String>(10)?,
                    ))
                },
            )
            .optional()
            .map_err(store_error)?
            .ok_or_else(|| {
                DedupeError::InvalidInput(format!("Không nhận diện được lô xóa {batch_id}"))
            })?;
        let project_id = parse_uuid(&stored.1, "delete project")?;
        let mut statement = connection
            .prepare(
                "SELECT entry_id,status,quarantine_path,volume_id,file_id,size_bytes,
                        blake3_digest,sha256_digest,retain_until
                 FROM permanent_delete_items WHERE batch_id=?1 ORDER BY ordinal",
            )
            .map_err(store_error)?;
        let rows = statement
            .query_map([batch_id.to_string()], |row| {
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
            .map_err(store_error)?;
        let mut items = Vec::new();
        for row in rows {
            let row = row.map_err(store_error)?;
            items.push(PermanentDeleteItem {
                entry: DeletionEntry {
                    id: parse_uuid(&row.0, "delete entry")?,
                    project_id,
                    quarantine_path: PathBuf::from(row.2),
                    identity: FileIdentity {
                        volume_id: row.3,
                        file_id: row.4,
                    },
                    size_bytes: to_u64(row.5, "số byte của mục xóa")?,
                    blake3: row.6,
                    sha256: row.7,
                    retain_until: parse_time(&row.8, "hết hạn lưu giữ trước khi xóa")?,
                },
                state: parse_item_state(&row.1)?,
            });
        }
        Ok(PermanentDeleteBatch {
            id: parse_uuid(&stored.0, "delete batch")?,
            project_id,
            state: parse_batch_state(&stored.2)?,
            mode: parse_delete_mode(&stored.10)?,
            token_digest: fixed_digest(&stored.3, "token")?,
            selection_digest: fixed_digest(&stored.4, "selection")?,
            confirmation_phrase: stored.5,
            entry_count: to_u64(stored.6, "số mục xóa")?,
            total_bytes: to_u64(stored.7, "số byte xóa")?,
            created_at: parse_time(&stored.8, "tạo lô xóa")?,
            expires_at: parse_time(&stored.9, "hết hạn lô xóa")?,
            items,
        })
    }

    #[allow(clippy::too_many_lines)]
    fn transition_batch(
        &self,
        batch: &PermanentDeleteBatch,
        next: PermanentDeleteBatchState,
        error: Option<&str>,
    ) -> Result<()> {
        if !batch.state.can_transition_to(next) {
            return Err(DedupeError::State(format!(
                "Nhật ký từ chối chuyển trạng thái lô xóa vĩnh viễn {:?} -> {:?}",
                batch.state, next
            )));
        }
        let occurred_at = Utc::now().to_rfc3339();
        self.append_manifest(&DeleteManifestEvent::batch(
            batch,
            Some(batch.state),
            next,
            error,
            occurred_at.clone(),
        ))?;
        let mut connection = self.database.connection();
        let transaction = connection.transaction().map_err(store_error)?;
        let changed = transaction
            .execute(
                "UPDATE permanent_delete_batches SET status=?1,
                    started_at=CASE WHEN ?1='executing' THEN COALESCE(started_at,?2) ELSE started_at END,
                    completed_at=CASE WHEN ?1='completed' THEN ?2 ELSE completed_at END,
                    error_message=?3 WHERE id=?4 AND status=?5",
                params![
                    batch_state_name(next),
                    occurred_at,
                    error,
                    batch.id.to_string(),
                    batch_state_name(batch.state),
                ],
            )
            .map_err(store_error)?;
        if changed != 1 {
            return Err(DedupeError::State(format!(
                "Lô xóa {} đã thay đổi đồng thời",
                batch.id
            )));
        }
        if next == PermanentDeleteBatchState::Executing {
            for item in &batch.items {
                if item.state == PermanentDeleteItemState::Deleted {
                    continue;
                }
                let allowed = if batch.state == PermanentDeleteBatchState::Prepared {
                    "active"
                } else {
                    match item.state {
                        PermanentDeleteItemState::Failed => "failed",
                        _ => "deleting",
                    }
                };
                let reserved = transaction
                    .execute(
                        "UPDATE quarantine_entries SET permanent_delete_state='deleting',
                            permanent_delete_batch_id=?1
                         WHERE id=?2 AND project_id=?3 AND state='verified'
                           AND permanent_delete_state IN (?4,'deleting')
                           AND (permanent_delete_batch_id IS NULL OR permanent_delete_batch_id=?1)",
                        params![
                            batch.id.to_string(),
                            item.entry.id.to_string(),
                            batch.project_id.to_string(),
                            allowed,
                        ],
                    )
                    .map_err(store_error)?;
                if reserved != 1 {
                    return Err(DedupeError::Safety(format!(
                        "Mục cách ly {} không còn khả dụng cho lô xóa này",
                        item.entry.id
                    )));
                }
            }
        }
        if next == PermanentDeleteBatchState::Completed {
            let remaining: i64 = transaction
                .query_row(
                    "SELECT COUNT(*) FROM permanent_delete_items
                     WHERE batch_id=?1 AND status!='deleted'",
                    [batch.id.to_string()],
                    |row| row.get(0),
                )
                .map_err(store_error)?;
            if remaining != 0 {
                return Err(DedupeError::State(
                    "Không thể hoàn tất lô xóa khi còn mục chưa xóa".into(),
                ));
            }
        }
        insert_event(
            &transaction,
            batch.id,
            None,
            Some(batch_state_name(batch.state)),
            batch_state_name(next),
            "batch",
            error,
            &occurred_at,
        )?;
        insert_audit(
            &transaction,
            batch,
            None,
            &format!("permanent_delete.{}", batch_state_name(next)),
            error,
            &occurred_at,
        )?;
        transaction.commit().map_err(store_error)
    }

    fn transition_item(
        &self,
        batch: &PermanentDeleteBatch,
        item: &PermanentDeleteItem,
        next: PermanentDeleteItemState,
        error: Option<&str>,
    ) -> Result<()> {
        if !item.state.can_transition_to(next) {
            return Err(DedupeError::State(format!(
                "Nhật ký từ chối chuyển trạng thái mục xóa vĩnh viễn {:?} -> {:?}",
                item.state, next
            )));
        }
        let occurred_at = Utc::now().to_rfc3339();
        self.append_manifest(&DeleteManifestEvent::item(
            batch,
            item,
            next,
            error,
            occurred_at.clone(),
        ))?;
        let mut connection = self.database.connection();
        let transaction = connection.transaction().map_err(store_error)?;
        let changed = transaction
            .execute(
                "UPDATE permanent_delete_items SET status=?1,updated_at=?2,error_message=?3
                 WHERE batch_id=?4 AND entry_id=?5 AND status=?6",
                params![
                    item_state_name(next),
                    occurred_at,
                    error,
                    batch.id.to_string(),
                    item.entry.id.to_string(),
                    item_state_name(item.state),
                ],
            )
            .map_err(store_error)?;
        if changed != 1 {
            return Err(DedupeError::State(format!(
                "Mục xóa {} đã thay đổi đồng thời",
                item.entry.id
            )));
        }
        let projection_state = match next {
            PermanentDeleteItemState::Planned => "active",
            PermanentDeleteItemState::Deleting => "deleting",
            PermanentDeleteItemState::Deleted => "deleted",
            PermanentDeleteItemState::Failed => "failed",
        };
        let projected = transaction
            .execute(
                "UPDATE quarantine_entries SET permanent_delete_state=?1,
                    permanent_delete_batch_id=?2,
                    deleted_at=CASE WHEN ?1='deleted' THEN ?3 ELSE deleted_at END
                 WHERE id=?4 AND permanent_delete_batch_id=?2",
                params![
                    projection_state,
                    batch.id.to_string(),
                    occurred_at,
                    item.entry.id.to_string(),
                ],
            )
            .map_err(store_error)?;
        if projected != 1 {
            return Err(DedupeError::State(format!(
                "Bản chiếu cách ly của mục xóa {} đã thay đổi đồng thời",
                item.entry.id
            )));
        }
        insert_event(
            &transaction,
            batch.id,
            Some(item.entry.id),
            Some(item_state_name(item.state)),
            item_state_name(next),
            "item",
            error,
            &occurred_at,
        )?;
        insert_audit(
            &transaction,
            batch,
            Some(item.entry.id),
            &format!("permanent_delete.item_{}", item_state_name(next)),
            error,
            &occurred_at,
        )?;
        transaction.commit().map_err(store_error)
    }
}

#[derive(Serialize)]
struct DeleteManifestEvent<'a> {
    batch_id: Uuid,
    project_id: Uuid,
    entry_id: Option<Uuid>,
    event_type: &'static str,
    from: Option<String>,
    to: String,
    selection_digest: &'a [u8; 32],
    deletion_mode: &'static str,
    error: Option<&'a str>,
    occurred_at: String,
}

impl<'a> DeleteManifestEvent<'a> {
    fn batch(
        batch: &'a PermanentDeleteBatch,
        from: Option<PermanentDeleteBatchState>,
        to: PermanentDeleteBatchState,
        error: Option<&'a str>,
        occurred_at: String,
    ) -> Self {
        Self {
            batch_id: batch.id,
            project_id: batch.project_id,
            entry_id: None,
            event_type: "batch",
            from: from.map(|state| batch_state_name(state).to_owned()),
            to: batch_state_name(to).to_owned(),
            selection_digest: &batch.selection_digest,
            deletion_mode: delete_mode_name(batch.mode),
            error,
            occurred_at,
        }
    }

    fn item(
        batch: &'a PermanentDeleteBatch,
        item: &PermanentDeleteItem,
        to: PermanentDeleteItemState,
        error: Option<&'a str>,
        occurred_at: String,
    ) -> Self {
        Self {
            batch_id: batch.id,
            project_id: batch.project_id,
            entry_id: Some(item.entry.id),
            event_type: "item",
            from: Some(item_state_name(item.state).to_owned()),
            to: item_state_name(to).to_owned(),
            selection_digest: &batch.selection_digest,
            deletion_mode: delete_mode_name(batch.mode),
            error,
            occurred_at,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn insert_event(
    transaction: &rusqlite::Transaction<'_>,
    batch_id: Uuid,
    entry_id: Option<Uuid>,
    from: Option<&str>,
    to: &str,
    event_type: &str,
    error: Option<&str>,
    occurred_at: &str,
) -> Result<()> {
    transaction
        .execute(
            "INSERT INTO permanent_delete_events (
                batch_id,entry_id,from_status,to_status,event_type,error_message,occurred_at
             ) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                batch_id.to_string(),
                entry_id.map(|id| id.to_string()),
                from,
                to,
                event_type,
                error,
                occurred_at,
            ],
        )
        .map_err(store_error)?;
    Ok(())
}

fn insert_audit(
    transaction: &rusqlite::Transaction<'_>,
    batch: &PermanentDeleteBatch,
    entry_id: Option<Uuid>,
    event_type: &str,
    error: Option<&str>,
    occurred_at: &str,
) -> Result<()> {
    let payload = serde_json::to_string(&serde_json::json!({
        "batch_id": batch.id,
        "entry_id": entry_id,
        "entry_count": batch.entry_count,
        "total_bytes": batch.total_bytes,
        "selection_digest": hex::encode(batch.selection_digest),
        "deletion_mode": delete_mode_name(batch.mode),
        "error": error,
    }))?;
    transaction
        .execute(
            "INSERT INTO audit_events (
                event_id,project_id,actor,event_type,payload_json,occurred_at
             ) VALUES (?1,?2,'user',?3,?4,?5)",
            params![
                Uuid::new_v4().to_string(),
                batch.project_id.to_string(),
                event_type,
                payload,
                occurred_at,
            ],
        )
        .map_err(store_error)?;
    Ok(())
}

fn batch_state_name(state: PermanentDeleteBatchState) -> &'static str {
    match state {
        PermanentDeleteBatchState::Prepared => "prepared",
        PermanentDeleteBatchState::Executing => "executing",
        PermanentDeleteBatchState::Completed => "completed",
        PermanentDeleteBatchState::RecoveryRequired => "recovery_required",
        PermanentDeleteBatchState::Expired => "expired",
    }
}

fn item_state_name(state: PermanentDeleteItemState) -> &'static str {
    match state {
        PermanentDeleteItemState::Planned => "planned",
        PermanentDeleteItemState::Deleting => "deleting",
        PermanentDeleteItemState::Deleted => "deleted",
        PermanentDeleteItemState::Failed => "failed",
    }
}

fn delete_mode_name(mode: PermanentDeleteMode) -> &'static str {
    match mode {
        PermanentDeleteMode::RetentionExpired => "retention_expired",
        PermanentDeleteMode::Immediate => "immediate",
    }
}

fn parse_delete_mode(value: &str) -> Result<PermanentDeleteMode> {
    match value {
        "retention_expired" => Ok(PermanentDeleteMode::RetentionExpired),
        "immediate" => Ok(PermanentDeleteMode::Immediate),
        _ => Err(DedupeError::State(format!(
            "Không nhận diện được chế độ xóa vĩnh viễn {value}"
        ))),
    }
}

fn parse_batch_state(value: &str) -> Result<PermanentDeleteBatchState> {
    match value {
        "prepared" => Ok(PermanentDeleteBatchState::Prepared),
        "executing" => Ok(PermanentDeleteBatchState::Executing),
        "completed" => Ok(PermanentDeleteBatchState::Completed),
        "recovery_required" => Ok(PermanentDeleteBatchState::RecoveryRequired),
        "expired" => Ok(PermanentDeleteBatchState::Expired),
        _ => Err(DedupeError::State(format!(
            "Không nhận diện được trạng thái lô xóa vĩnh viễn {value}"
        ))),
    }
}

fn parse_item_state(value: &str) -> Result<PermanentDeleteItemState> {
    match value {
        "planned" => Ok(PermanentDeleteItemState::Planned),
        "deleting" => Ok(PermanentDeleteItemState::Deleting),
        "deleted" => Ok(PermanentDeleteItemState::Deleted),
        "failed" => Ok(PermanentDeleteItemState::Failed),
        _ => Err(DedupeError::State(format!(
            "Không nhận diện được trạng thái mục xóa vĩnh viễn {value}"
        ))),
    }
}

fn fixed_digest(value: &[u8], kind: &str) -> Result<[u8; 32]> {
    value
        .try_into()
        .map_err(|_| DedupeError::State(format!("Giá trị băm {kind} đã lưu có độ dài sai")))
}

fn parse_time(value: &str, kind: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|time| time.with_timezone(&Utc))
        .map_err(|error| {
            DedupeError::State(format!("Dấu thời gian {kind} đã lưu không hợp lệ: {error}"))
        })
}

fn parse_uuid(value: &str, kind: &str) -> Result<Uuid> {
    let label = match kind {
        "delete project" => "dự án xóa",
        "delete entry" => "mục xóa",
        "delete batch" => "lô xóa",
        _ => kind,
    };
    Uuid::parse_str(value)
        .map_err(|error| DedupeError::State(format!("UUID {label} đã lưu không hợp lệ: {error}")))
}

fn to_i64(value: u64, field: &str) -> Result<i64> {
    i64::try_from(value).map_err(|_| {
        DedupeError::Safety(format!("{field} vượt quá miền số nguyên có dấu của SQLite"))
    })
}

fn to_u64(value: i64, field: &str) -> Result<u64> {
    u64::try_from(value).map_err(|_| DedupeError::State(format!("Giá trị {field} đã lưu là số âm")))
}

fn store_error(error: rusqlite::Error) -> DedupeError {
    DedupeError::Durability(error.to_string())
}
