//! Idempotent persistence and reconstruction of proven duplicate groups.

use std::path::PathBuf;

use chrono::Utc;
use dedupe_core::{
    DedupeError, Result,
    model::{
        AccessStatus, ComparisonMode, DuplicateGroup, DuplicateMember, FileIdentity,
        FileMetadataSnapshot, HashAlgorithm, HashResult, LinkKind, MemberAction, ProvenFile,
    },
    path_normalization::{path_identity_key, path_key},
};
use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use crate::Database;

/// Repository for immutable hash evidence and duplicate groups.
#[derive(Debug, Clone)]
pub struct DuplicateRepository {
    database: Database,
}

impl DuplicateRepository {
    /// Bind the repository to a database.
    #[must_use]
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    /// Replace the proven-group projection for one session in a single transaction.
    pub fn replace_session_groups(
        &self,
        session_id: Uuid,
        groups: &[DuplicateGroup],
    ) -> Result<()> {
        let mut connection = self.database.connection();
        let transaction = connection.transaction().map_err(store_error)?;
        transaction
            .execute(
                "DELETE FROM duplicate_groups WHERE session_id=?1",
                [session_id.to_string()],
            )
            .map_err(store_error)?;
        let now = Utc::now().to_rfc3339();
        let mut reclaimable = 0_u64;
        for group in groups {
            group.validate_keeper()?;
            let member_count = i64::try_from(group.members.len())
                .map_err(|_| DedupeError::Safety("Số thành viên vượt quá phạm vi SQLite".into()))?;
            transaction
                .execute(
                    "INSERT INTO duplicate_groups (
                        id,session_id,mode,size_bytes,normalized_name,blake3_digest,
                        sha256_digest,member_count,reclaimable_bytes,verified_at
                     ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                    params![
                        group.id.to_string(),
                        session_id.to_string(),
                        mode_name(group.mode),
                        as_i64(group.size_bytes)?,
                        group.normalized_name,
                        group.blake3,
                        group.sha256,
                        member_count,
                        as_i64(group.maximum_reclaimable_bytes())?,
                        now,
                    ],
                )
                .map_err(store_error)?;
            reclaimable = reclaimable.saturating_add(group.maximum_reclaimable_bytes());
            for member in &group.members {
                let snapshot_id =
                    snapshot_id_for_path(&transaction, session_id, &member.file.metadata.path)?;
                persist_hash(
                    &transaction,
                    &snapshot_id,
                    "blake3",
                    "blake3",
                    &member.file.blake3,
                    &now,
                )?;
                persist_hash(
                    &transaction,
                    &snapshot_id,
                    "sha256",
                    "sha256",
                    &member.file.sha256,
                    &now,
                )?;
                transaction
                    .execute(
                        "UPDATE file_snapshots SET state='duplicate_confirmed', completed_at=?1
                         WHERE id=?2",
                        params![now, snapshot_id],
                    )
                    .map_err(store_error)?;
                transaction
                    .execute(
                        "INSERT INTO duplicate_members (group_id,snapshot_id,recommendation,reason)
                         VALUES (?1,?2,?3,?4)",
                        params![
                            group.id.to_string(),
                            snapshot_id,
                            action_name(member.action),
                            member.reason,
                        ],
                    )
                    .map_err(store_error)?;
            }
        }
        transaction
            .execute(
                "UPDATE scan_sessions SET duplicate_groups=?1,reclaimable_bytes=?2,updated_at=?3
                 WHERE id=?4",
                params![
                    as_i64(groups.len() as u64)?,
                    as_i64(reclaimable)?,
                    now,
                    session_id.to_string(),
                ],
            )
            .map_err(store_error)?;
        transaction.commit().map_err(store_error)?;
        Ok(())
    }

    /// Load all proven groups for review, reporting, or sealed plan construction.
    pub fn load_session_groups(&self, session_id: Uuid) -> Result<Vec<DuplicateGroup>> {
        let connection = self
            .database
            .read_connection()
            .map_err(|error| DedupeError::Durability(error.to_string()))?;
        let mut group_statement = connection
            .prepare(
                "SELECT id,mode,size_bytes,normalized_name,blake3_digest,sha256_digest
                 FROM duplicate_groups WHERE session_id=?1 ORDER BY size_bytes DESC,id",
            )
            .map_err(store_error)?;
        let group_rows = group_statement
            .query_map([session_id.to_string()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                    row.get::<_, Vec<u8>>(5)?,
                ))
            })
            .map_err(store_error)?;
        let mut headers = Vec::new();
        for row in group_rows {
            headers.push(row.map_err(store_error)?);
        }
        drop(group_statement);

        let mut groups = Vec::with_capacity(headers.len());
        for (id, mode, size, normalized_name, blake3, sha256) in headers {
            let group_id = Uuid::parse_str(&id)
                .map_err(|error| DedupeError::State(format!("UUID nhóm không hợp lệ: {error}")))?;
            let mut member_statement = connection
                .prepare(
                    "SELECT m.snapshot_id,m.recommendation,m.reason,
                            e.original_path,e.normalized_path,e.normalized_name,e.extension,
                            s.volume_id,s.file_id,s.size_bytes,s.created_time_ns,s.modified_time_ns,
                            s.link_kind,s.hardlink_count,s.access_status,s.snapshot_token
                     FROM duplicate_members m
                     JOIN file_snapshots s ON s.id=m.snapshot_id
                     JOIN file_entries e ON e.id=s.file_entry_id
                     WHERE m.group_id=?1 ORDER BY e.normalized_path",
                )
                .map_err(store_error)?;
            let rows = member_statement
                .query_map([id], |row| {
                    Ok(MemberRow {
                        snapshot_id: row.get(0)?,
                        recommendation: row.get(1)?,
                        reason: row.get(2)?,
                        path: row.get(3)?,
                        normalized_path: row.get(4)?,
                        normalized_name: row.get(5)?,
                        extension: row.get(6)?,
                        volume_id: row.get(7)?,
                        file_id: row.get(8)?,
                        size_bytes: row.get(9)?,
                        created_ns: row.get(10)?,
                        modified_ns: row.get(11)?,
                        link_kind: row.get(12)?,
                        hardlink_count: row.get(13)?,
                        access_status: row.get(14)?,
                        snapshot_token: row.get(15)?,
                    })
                })
                .map_err(store_error)?;
            let mut members = Vec::new();
            for row in rows {
                let row = row.map_err(store_error)?;
                let metadata = row.metadata()?;
                let blake3_result = load_hash(&connection, &row.snapshot_id, "blake3")?;
                let sha256_result = load_hash(&connection, &row.snapshot_id, "sha256")?;
                members.push(DuplicateMember {
                    file: ProvenFile {
                        metadata,
                        blake3: blake3_result,
                        sha256: sha256_result,
                    },
                    action: parse_action(&row.recommendation)?,
                    reason: row.reason,
                });
            }
            groups.push(DuplicateGroup {
                id: group_id,
                mode: parse_mode(&mode)?,
                size_bytes: to_u64(size, "kích thước nhóm")?,
                normalized_name,
                blake3,
                sha256,
                members,
            });
        }
        Ok(groups)
    }
}

struct MemberRow {
    snapshot_id: String,
    recommendation: String,
    reason: String,
    path: String,
    normalized_path: String,
    normalized_name: String,
    extension: Option<String>,
    volume_id: Option<String>,
    file_id: Option<String>,
    size_bytes: i64,
    created_ns: Option<i64>,
    modified_ns: i64,
    link_kind: String,
    hardlink_count: Option<i64>,
    access_status: String,
    snapshot_token: Vec<u8>,
}

impl MemberRow {
    fn metadata(&self) -> Result<FileMetadataSnapshot> {
        Ok(FileMetadataSnapshot {
            path: PathBuf::from(&self.path),
            normalized_path: self.normalized_path.clone(),
            normalized_name: self.normalized_name.clone(),
            extension: self.extension.clone(),
            size_bytes: to_u64(self.size_bytes, "kích thước tệp")?,
            created_ns: self.created_ns.map(i128::from),
            modified_ns: i128::from(self.modified_ns),
            identity: self
                .volume_id
                .clone()
                .zip(self.file_id.clone())
                .map(|(volume_id, file_id)| FileIdentity { volume_id, file_id }),
            link_kind: parse_link(&self.link_kind)?,
            hardlink_count: self
                .hardlink_count
                .map(|value| to_u64(value, "số liên kết cứng"))
                .transpose()?,
            access_status: parse_access(&self.access_status)?,
            snapshot_token: fixed_token(&self.snapshot_token)?,
        })
    }
}

fn persist_hash(
    transaction: &rusqlite::Transaction<'_>,
    snapshot_id: &str,
    stage: &str,
    algorithm: &str,
    result: &HashResult,
    now: &str,
) -> Result<()> {
    transaction
        .execute(
            "INSERT INTO hash_results (
                id,snapshot_id,stage,algorithm,digest,bytes_read,snapshot_token_before,
                snapshot_token_after,stable,started_at,completed_at
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10)
             ON CONFLICT(snapshot_id,stage) DO UPDATE SET
                algorithm=excluded.algorithm,digest=excluded.digest,bytes_read=excluded.bytes_read,
                snapshot_token_before=excluded.snapshot_token_before,
                snapshot_token_after=excluded.snapshot_token_after,stable=excluded.stable,
                completed_at=excluded.completed_at",
            params![
                Uuid::new_v4().to_string(),
                snapshot_id,
                stage,
                algorithm,
                result.digest,
                as_i64(result.bytes_read)?,
                result.snapshot_before.as_slice(),
                result.snapshot_after.as_slice(),
                i64::from(result.stable),
                now,
            ],
        )
        .map_err(store_error)?;
    Ok(())
}

fn load_hash(
    connection: &rusqlite::Connection,
    snapshot_id: &str,
    stage: &str,
) -> Result<HashResult> {
    connection
        .query_row(
            "SELECT algorithm,digest,bytes_read,snapshot_token_before,snapshot_token_after,stable
             FROM hash_results WHERE snapshot_id=?1 AND stage=?2",
            params![snapshot_id, stage],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            },
        )
        .optional()
        .map_err(store_error)?
        .ok_or_else(|| DedupeError::State(format!("Thiếu bằng chứng {stage} cho {snapshot_id}")))
        .and_then(|(algorithm, digest, bytes_read, before, after, stable)| {
            Ok(HashResult {
                algorithm: parse_algorithm(&algorithm)?,
                digest,
                bytes_read: to_u64(bytes_read, "số byte đã băm")?,
                snapshot_before: fixed_token(&before)?,
                snapshot_after: fixed_token(&after)?,
                stable: stable != 0,
            })
        })
}

pub(crate) fn snapshot_id_for_path(
    connection: &rusqlite::Connection,
    session_id: Uuid,
    path: &std::path::Path,
) -> Result<String> {
    let normalized_match: Option<(String, String)> = connection
        .query_row(
            "SELECT s.id,e.original_path FROM scan_sessions ss
             JOIN file_entries e ON e.project_id=ss.project_id AND e.path_key=?2
             JOIN file_snapshots s ON s.session_id=ss.id AND s.file_entry_id=e.id
             WHERE ss.id=?1",
            params![session_id.to_string(), path_key(path)?.as_slice()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    let identity_key = path_identity_key(path)?;
    if let Some((snapshot_id, original_path)) = normalized_match
        && path_identity_key(std::path::Path::new(&original_path))? == identity_key
    {
        return Ok(snapshot_id);
    }
    let collision_match: Option<(String, String)> = connection
        .query_row(
            "SELECT s.id,e.original_path FROM scan_sessions ss
             JOIN file_entries e ON e.project_id=ss.project_id AND e.path_key=?2
             JOIN file_snapshots s ON s.session_id=ss.id AND s.file_entry_id=e.id
             WHERE ss.id=?1",
            params![session_id.to_string(), identity_key.as_slice()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    if let Some((snapshot_id, original_path)) = collision_match
        && path_identity_key(std::path::Path::new(&original_path))? == identity_key
    {
        return Ok(snapshot_id);
    }
    Err(DedupeError::State(format!(
        "Không có ảnh chụp quét chính xác cho đường dẫn đã chứng minh {}",
        path.display()
    )))
}

fn fixed_token(value: &[u8]) -> Result<[u8; 32]> {
    value
        .try_into()
        .map_err(|_| DedupeError::State("Token đã lưu có độ dài sai".into()))
}

fn as_i64(value: u64) -> Result<i64> {
    i64::try_from(value)
        .map_err(|_| DedupeError::Safety("Giá trị vượt quá phạm vi có dấu của SQLite".into()))
}

fn to_u64(value: i64, field: &str) -> Result<u64> {
    u64::try_from(value).map_err(|_| DedupeError::State(format!("Giá trị {field} đã lưu là số âm")))
}

fn mode_name(mode: ComparisonMode) -> &'static str {
    match mode {
        ComparisonMode::Strict => "strict",
        ComparisonMode::Content => "content",
    }
}

fn parse_mode(value: &str) -> Result<ComparisonMode> {
    match value {
        "strict" => Ok(ComparisonMode::Strict),
        "content" => Ok(ComparisonMode::Content),
        _ => Err(DedupeError::State(format!(
            "Không nhận diện được chế độ so sánh: {value}"
        ))),
    }
}

fn action_name(action: MemberAction) -> &'static str {
    match action {
        MemberAction::Keep => "keep",
        MemberAction::Quarantine => "quarantine",
        MemberAction::Manual => "manual",
    }
}

fn parse_action(value: &str) -> Result<MemberAction> {
    match value {
        "keep" => Ok(MemberAction::Keep),
        "quarantine" => Ok(MemberAction::Quarantine),
        "manual" => Ok(MemberAction::Manual),
        _ => Err(DedupeError::State(format!(
            "Không nhận diện được thao tác thành viên: {value}"
        ))),
    }
}

fn parse_algorithm(value: &str) -> Result<HashAlgorithm> {
    match value {
        "blake3" => Ok(HashAlgorithm::Blake3),
        "sha256" => Ok(HashAlgorithm::Sha256),
        "blake3-sampled-v1" => Ok(HashAlgorithm::QuickBlake3V1),
        _ => Err(DedupeError::State(format!(
            "Không nhận diện được thuật toán băm: {value}"
        ))),
    }
}

fn parse_link(value: &str) -> Result<LinkKind> {
    match value {
        "regular" => Ok(LinkKind::Regular),
        "hardlink" => Ok(LinkKind::HardLink),
        "symlink" => Ok(LinkKind::Symlink),
        "junction" => Ok(LinkKind::Junction),
        "other" => Ok(LinkKind::Other),
        _ => Err(DedupeError::State(format!(
            "Không nhận diện được loại liên kết: {value}"
        ))),
    }
}

fn parse_access(value: &str) -> Result<AccessStatus> {
    match value {
        "readable" => Ok(AccessStatus::Readable),
        "locked" => Ok(AccessStatus::Locked),
        "denied" => Ok(AccessStatus::Denied),
        "offline" => Ok(AccessStatus::Offline),
        "missing" => Ok(AccessStatus::Missing),
        "error" => Ok(AccessStatus::Error),
        _ => Err(DedupeError::State(format!(
            "Không nhận diện được trạng thái truy cập: {value}"
        ))),
    }
}

fn store_error(error: rusqlite::Error) -> DedupeError {
    DedupeError::Durability(error.to_string())
}
