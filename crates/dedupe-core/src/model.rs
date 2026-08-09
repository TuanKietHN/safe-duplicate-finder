//! Domain entities and explicit state machines.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Duplicate comparison policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonMode {
    /// Normalized filename, exact size, both full digests, stability, and distinct identity.
    Strict,
    /// Exact size, both full digests, stability, and distinct identity; filename may differ.
    Content,
}

/// Link classification used to prevent reclaiming hard-link aliases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    /// Ordinary independently stored file.
    Regular,
    /// A path with multiple links to one physical file.
    HardLink,
    /// Symbolic link, not followed by default.
    Symlink,
    /// Windows junction/reparse directory.
    Junction,
    /// Other platform-specific link-like object.
    Other,
}

/// Accessibility observed during metadata collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessStatus {
    /// File can be opened for reading.
    Readable,
    /// File is locked by another process.
    Locked,
    /// Access was denied.
    Denied,
    /// Cloud/network content is offline.
    Offline,
    /// File disappeared.
    Missing,
    /// Other error.
    Error,
}

/// Stable physical identity. The volume and file identifier must be compared together.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FileIdentity {
    /// Platform volume identifier encoded without truncation.
    pub volume_id: String,
    /// Platform file identifier encoded without truncation.
    pub file_id: String,
}

/// Metadata snapshot that is rechecked around every content read and mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMetadataSnapshot {
    /// Original user-visible path.
    pub path: PathBuf,
    /// Normalized path used for overlap detection.
    pub normalized_path: String,
    /// Normalized leaf name used by strict mode.
    pub normalized_name: String,
    /// Lowercase extension without leading dot.
    pub extension: Option<String>,
    /// Exact byte size using a 64-bit-safe type.
    pub size_bytes: u64,
    /// Creation timestamp in nanoseconds since Unix epoch, if available.
    pub created_ns: Option<i128>,
    /// Modification timestamp in nanoseconds since Unix epoch.
    pub modified_ns: i128,
    /// Stable physical identity when the platform can provide it.
    pub identity: Option<FileIdentity>,
    /// Link classification.
    pub link_kind: LinkKind,
    /// Link count if available.
    pub hardlink_count: Option<u64>,
    /// Accessibility observed.
    pub access_status: AccessStatus,
    /// Digest of identity-relevant metadata, never of document content.
    pub snapshot_token: [u8; 32],
}

/// Result of one sampled or full streaming hash pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HashResult {
    /// Algorithm/stage identifier.
    pub algorithm: HashAlgorithm,
    /// Raw digest bytes.
    pub digest: Vec<u8>,
    /// Bytes read in this pass.
    pub bytes_read: u64,
    /// Snapshot before reading.
    pub snapshot_before: [u8; 32],
    /// Snapshot after reading.
    pub snapshot_after: [u8; 32],
    /// True only when both metadata snapshots match.
    pub stable: bool,
}

/// Supported evidence stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HashAlgorithm {
    /// Domain-separated sampled BLAKE3 rejector.
    QuickBlake3V1,
    /// Full streaming BLAKE3.
    Blake3,
    /// Full streaming SHA-256.
    Sha256,
}

/// File candidate and its confirmed evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenFile {
    /// Immutable metadata snapshot.
    pub metadata: FileMetadataSnapshot,
    /// Full BLAKE3 evidence.
    pub blake3: HashResult,
    /// Full SHA-256 evidence.
    pub sha256: HashResult,
}

/// Why a member is retained or proposed for quarantine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberAction {
    /// This member is retained.
    Keep,
    /// This member may be quarantined after explicit confirmation.
    Quarantine,
    /// A user decision is required.
    Manual,
}

/// One member of a proven duplicate group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateMember {
    /// Proven file evidence.
    pub file: ProvenFile,
    /// Proposed action.
    pub action: MemberAction,
    /// Human-readable deterministic reason.
    pub reason: String,
}

/// A group is created only after full BLAKE3 and full SHA-256 agree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateGroup {
    /// Stable group identifier for the scan session.
    pub id: Uuid,
    /// Comparison mode used.
    pub mode: ComparisonMode,
    /// Exact member size.
    pub size_bytes: u64,
    /// Normalized name in strict mode.
    pub normalized_name: Option<String>,
    /// Full BLAKE3 digest.
    pub blake3: Vec<u8>,
    /// Full SHA-256 digest.
    pub sha256: Vec<u8>,
    /// Independently stored members only.
    pub members: Vec<DuplicateMember>,
}

impl DuplicateGroup {
    /// Bytes reclaimable while retaining exactly one member.
    #[must_use]
    pub fn maximum_reclaimable_bytes(&self) -> u64 {
        self.size_bytes
            .saturating_mul(self.members.len().saturating_sub(1) as u64)
    }

    /// Enforce the invariant that at least one member remains.
    pub fn validate_keeper(&self) -> crate::Result<()> {
        if self.members.iter().any(|m| m.action == MemberAction::Keep) {
            Ok(())
        } else {
            Err(crate::DedupeError::Safety(format!(
                "Nhóm trùng lặp {} không có tệp giữ lại",
                self.id
            )))
        }
    }
}

/// Keep selection policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeepPolicy {
    /// Primary roots, then oldest, then shortest path.
    Default {
        /// Roots explicitly marked as authoritative by the user.
        primary_roots: Vec<PathBuf>,
    },
    /// Oldest modification time.
    Oldest,
    /// Newest modification time.
    Newest,
    /// Shortest normalized path.
    ShortestPath,
    /// Explicit path chosen by the user.
    Manual(PathBuf),
}

/// A sealed, reviewable plan tied to immutable evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationPlan {
    /// Plan identifier used for idempotency.
    pub id: Uuid,
    /// Groups and selected actions.
    pub groups: Vec<DuplicateGroup>,
    /// Evidence generation/version.
    pub evidence_version: u64,
    /// Creation time.
    pub created_at: DateTime<Utc>,
}

/// Quarantine/restore transaction kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionKind {
    /// Move from source to quarantine.
    Quarantine,
    /// Move from quarantine back to source.
    Restore,
}

/// Durable transaction state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionState {
    /// Intent persisted before mutation.
    Planned,
    /// Source evidence revalidated.
    PreflightValidated,
    /// Mutation is about to execute.
    Moving,
    /// Destination exists but is not yet verified.
    MovedUnverified,
    /// Destination evidence is verified and durable.
    Verified,
    /// Preflight failed with source left untouched.
    PreflightFailed,
    /// Move returned an error.
    MoveFailed,
    /// Destination verification failed.
    VerifyFailed,
    /// Startup reconciliation is required.
    RecoveryRequired,
    /// User cancelled before mutation.
    Cancelled,
    /// Recovery proved that only the original source exists.
    ReconciledSourceOnly,
    /// Recovery found both paths and preserved both for explicit review.
    ReconciledBoth,
    /// Recovery found neither path and recorded the data-loss condition.
    ReconciledMissing,
}

impl TransactionState {
    /// Validate one forward transition; history is append-only.
    #[must_use]
    pub fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (
                Self::Planned,
                Self::PreflightValidated
                    | Self::PreflightFailed
                    | Self::Cancelled
                    | Self::ReconciledSourceOnly
                    | Self::ReconciledBoth
                    | Self::ReconciledMissing
            ) | (
                Self::PreflightValidated,
                Self::Moving
                    | Self::Cancelled
                    | Self::ReconciledSourceOnly
                    | Self::ReconciledBoth
                    | Self::ReconciledMissing
            ) | (
                Self::Moving,
                Self::MovedUnverified
                    | Self::MoveFailed
                    | Self::RecoveryRequired
                    | Self::ReconciledSourceOnly
                    | Self::ReconciledBoth
                    | Self::ReconciledMissing
            ) | (
                Self::MovedUnverified,
                Self::Verified
                    | Self::VerifyFailed
                    | Self::RecoveryRequired
                    | Self::ReconciledSourceOnly
                    | Self::ReconciledBoth
                    | Self::ReconciledMissing
            ) | (
                Self::MoveFailed | Self::VerifyFailed,
                Self::RecoveryRequired
                    | Self::ReconciledSourceOnly
                    | Self::ReconciledBoth
                    | Self::ReconciledMissing
            ) | (
                Self::RecoveryRequired,
                Self::Verified
                    | Self::ReconciledSourceOnly
                    | Self::ReconciledBoth
                    | Self::ReconciledMissing
            )
        )
    }
}

/// Durable mutation record carrying all evidence required for recovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTransaction {
    /// Transaction identifier.
    pub id: Uuid,
    /// Owning project.
    pub project_id: Uuid,
    /// Scan session whose evidence authorized the action.
    pub session_id: Option<Uuid>,
    /// Immutable sealed plan item, when applicable.
    pub plan_item_id: Option<Uuid>,
    /// Quarantine or restore.
    pub kind: TransactionKind,
    /// Current state (history lives in append-only events).
    pub state: TransactionState,
    /// Source path for this transaction.
    pub source: PathBuf,
    /// Destination path for this transaction.
    pub destination: PathBuf,
    /// Expected physical identity.
    pub identity: FileIdentity,
    /// Expected size.
    pub size_bytes: u64,
    /// Expected full BLAKE3.
    pub blake3: Vec<u8>,
    /// Expected full SHA-256.
    pub sha256: Vec<u8>,
    /// Original snapshot token.
    pub snapshot_token: [u8; 32],
    /// Start time.
    pub started_at: DateTime<Utc>,
}

/// Persistent project configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    /// Project identifier.
    pub id: Uuid,
    /// Display name.
    pub name: String,
    /// Source roots.
    pub roots: Vec<ProjectRoot>,
    /// Comparison mode.
    pub mode: ComparisonMode,
    /// Worker configuration.
    pub workers: WorkerConfig,
}

/// Source root configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectRoot {
    /// Root identifier.
    pub id: Uuid,
    /// Original root path.
    pub path: PathBuf,
    /// Whether keep policy prefers this root.
    pub primary: bool,
}

/// Bounded scheduler settings.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct WorkerConfig {
    /// Metadata worker count.
    pub metadata_workers: usize,
    /// Maximum full-read workers per volume.
    pub full_hash_workers_per_volume: usize,
    /// Maximum queued metadata records.
    pub queue_capacity: usize,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            metadata_workers: 8,
            full_hash_workers_per_volume: 2,
            queue_capacity: 1024,
        }
    }
}
