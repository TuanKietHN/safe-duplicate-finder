PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;
PRAGMA synchronous = FULL;
PRAGMA busy_timeout = 5000;

BEGIN IMMEDIATE;

CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY CHECK (version > 0),
    name TEXT NOT NULL UNIQUE,
    applied_at TEXT NOT NULL
) STRICT;

CREATE TABLE projects (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    mode TEXT NOT NULL DEFAULT 'strict' CHECK (mode IN ('strict', 'content')),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'archived')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    last_scan_at TEXT,
    preferred_volume_id TEXT,
    worker_limit INTEGER NOT NULL DEFAULT 4 CHECK (worker_limit BETWEEN 1 AND 64),
    read_limit_bytes_per_sec INTEGER CHECK (read_limit_bytes_per_sec IS NULL OR read_limit_bytes_per_sec > 0),
    retention_days INTEGER NOT NULL DEFAULT 30 CHECK (retention_days BETWEEN 1 AND 3650),
    automatic_permanent_delete INTEGER NOT NULL DEFAULT 0 CHECK (automatic_permanent_delete IN (0, 1))
) STRICT;

CREATE TABLE project_roots (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    original_path TEXT NOT NULL,
    normalized_path TEXT NOT NULL,
    path_key BLOB NOT NULL,
    volume_id TEXT,
    is_primary INTEGER NOT NULL DEFAULT 0 CHECK (is_primary IN (0, 1)),
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    validation_status TEXT NOT NULL DEFAULT 'pending'
        CHECK (validation_status IN ('pending', 'valid', 'missing', 'duplicate', 'overlap', 'forbidden', 'unreadable')),
    created_at TEXT NOT NULL,
    UNIQUE (project_id, path_key)
) STRICT;

CREATE INDEX idx_project_roots_project ON project_roots(project_id, enabled);

CREATE TABLE filter_rules (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN (
        'include_extension', 'exclude_extension', 'exclude_folder', 'exclude_glob',
        'minimum_size', 'skip_hidden', 'skip_system', 'skip_quarantine'
    )),
    value TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    created_at TEXT NOT NULL
) STRICT;

CREATE TABLE scan_sessions (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE RESTRICT,
    mode TEXT NOT NULL CHECK (mode IN ('strict', 'content')),
    state TEXT NOT NULL CHECK (state IN (
        'draft', 'enumerating', 'quick_hashing', 'blake3_hashing', 'sha256_hashing',
        'grouping', 'pausing', 'paused', 'cancelling', 'cancelled', 'interrupted',
        'recovering', 'completed', 'blocked'
    )),
    started_at TEXT,
    finished_at TEXT,
    discovered_files INTEGER NOT NULL DEFAULT 0 CHECK (discovered_files >= 0),
    processed_files INTEGER NOT NULL DEFAULT 0 CHECK (processed_files >= 0),
    bytes_read INTEGER NOT NULL DEFAULT 0 CHECK (bytes_read >= 0),
    duplicate_groups INTEGER NOT NULL DEFAULT 0 CHECK (duplicate_groups >= 0),
    reclaimable_bytes INTEGER NOT NULL DEFAULT 0 CHECK (reclaimable_bytes >= 0),
    error_count INTEGER NOT NULL DEFAULT 0 CHECK (error_count >= 0),
    skipped_count INTEGER NOT NULL DEFAULT 0 CHECK (skipped_count >= 0),
    unstable_count INTEGER NOT NULL DEFAULT 0 CHECK (unstable_count >= 0),
    config_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

CREATE INDEX idx_scan_sessions_project ON scan_sessions(project_id, created_at DESC);

CREATE TABLE file_entries (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE RESTRICT,
    root_id TEXT NOT NULL REFERENCES project_roots(id) ON DELETE RESTRICT,
    original_path TEXT NOT NULL,
    normalized_path TEXT NOT NULL,
    path_key BLOB NOT NULL,
    normalized_name TEXT NOT NULL,
    extension TEXT,
    created_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    UNIQUE (project_id, path_key)
) STRICT;

CREATE INDEX idx_file_entries_project_name ON file_entries(project_id, normalized_name);

CREATE TABLE file_snapshots (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES scan_sessions(id) ON DELETE CASCADE,
    file_entry_id TEXT NOT NULL REFERENCES file_entries(id) ON DELETE RESTRICT,
    volume_id TEXT,
    file_id TEXT,
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    created_time_ns INTEGER,
    modified_time_ns INTEGER NOT NULL,
    attributes INTEGER NOT NULL DEFAULT 0,
    link_kind TEXT NOT NULL CHECK (link_kind IN ('regular', 'hardlink', 'symlink', 'junction', 'other')),
    hardlink_count INTEGER CHECK (hardlink_count IS NULL OR hardlink_count > 0),
    access_status TEXT NOT NULL CHECK (access_status IN ('readable', 'locked', 'denied', 'offline', 'missing', 'error')),
    state TEXT NOT NULL CHECK (state IN (
        'discovered', 'metadata_ready', 'candidate', 'unique', 'hardlink_alias', 'skipped',
        'quick_rejected', 'quick_hashed', 'blake3_rejected', 'blake3_hashed',
        'sha256_rejected', 'duplicate_confirmed', 'planned_keep', 'planned_quarantine',
        'preflight_failed', 'moving', 'quarantined_unverified', 'quarantined_verified',
        'restoring', 'restored_unverified', 'restored_verified', 'recovery_required',
        'unstable', 'error'
    )),
    snapshot_token BLOB NOT NULL,
    observed_at TEXT NOT NULL,
    completed_at TEXT,
    UNIQUE (session_id, file_entry_id)
) STRICT;

CREATE INDEX idx_file_snapshots_candidates
    ON file_snapshots(session_id, size_bytes, state, file_entry_id);
CREATE INDEX idx_file_snapshots_identity
    ON file_snapshots(session_id, volume_id, file_id) WHERE file_id IS NOT NULL;

CREATE TABLE hash_results (
    id TEXT PRIMARY KEY,
    snapshot_id TEXT NOT NULL REFERENCES file_snapshots(id) ON DELETE CASCADE,
    stage TEXT NOT NULL CHECK (stage IN ('quick', 'blake3', 'sha256')),
    algorithm TEXT NOT NULL CHECK (algorithm IN ('blake3-sampled-v1', 'blake3', 'sha256')),
    digest BLOB NOT NULL,
    bytes_read INTEGER NOT NULL CHECK (bytes_read >= 0),
    snapshot_token_before BLOB NOT NULL,
    snapshot_token_after BLOB NOT NULL,
    stable INTEGER NOT NULL CHECK (stable IN (0, 1)),
    started_at TEXT NOT NULL,
    completed_at TEXT NOT NULL,
    UNIQUE (snapshot_id, stage)
) STRICT;

CREATE INDEX idx_hash_results_digest ON hash_results(stage, digest, snapshot_id);

CREATE TABLE duplicate_groups (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES scan_sessions(id) ON DELETE CASCADE,
    mode TEXT NOT NULL CHECK (mode IN ('strict', 'content')),
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    normalized_name TEXT,
    blake3_digest BLOB NOT NULL,
    sha256_digest BLOB NOT NULL,
    member_count INTEGER NOT NULL CHECK (member_count >= 2),
    reclaimable_bytes INTEGER NOT NULL CHECK (reclaimable_bytes >= 0),
    verified_at TEXT NOT NULL,
    UNIQUE (session_id, mode, size_bytes, normalized_name, blake3_digest, sha256_digest)
) STRICT;

CREATE TABLE duplicate_members (
    group_id TEXT NOT NULL REFERENCES duplicate_groups(id) ON DELETE CASCADE,
    snapshot_id TEXT NOT NULL REFERENCES file_snapshots(id) ON DELETE RESTRICT,
    recommendation TEXT NOT NULL CHECK (recommendation IN ('keep', 'quarantine', 'manual')),
    reason TEXT NOT NULL,
    PRIMARY KEY (group_id, snapshot_id)
) STRICT;

CREATE TABLE operation_plans (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES scan_sessions(id) ON DELETE RESTRICT,
    status TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft', 'sealed', 'executing', 'completed', 'cancelled', 'stale')),
    policy TEXT NOT NULL CHECK (policy IN ('default', 'oldest', 'newest', 'shortest', 'preferred_volume', 'manual')),
    evidence_version INTEGER NOT NULL CHECK (evidence_version > 0),
    created_at TEXT NOT NULL,
    sealed_at TEXT,
    completed_at TEXT
) STRICT;

CREATE TABLE plan_items (
    id TEXT PRIMARY KEY,
    plan_id TEXT NOT NULL REFERENCES operation_plans(id) ON DELETE CASCADE,
    group_id TEXT NOT NULL,
    snapshot_id TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('keep', 'quarantine')),
    reason TEXT NOT NULL,
    selected_by TEXT NOT NULL CHECK (selected_by IN ('policy', 'user')),
    FOREIGN KEY (group_id, snapshot_id)
        REFERENCES duplicate_members(group_id, snapshot_id) ON DELETE RESTRICT,
    UNIQUE (plan_id, snapshot_id)
) STRICT;

CREATE INDEX idx_plan_items_group ON plan_items(plan_id, group_id, action);

CREATE TRIGGER operation_plan_requires_keeper_before_seal
BEFORE UPDATE OF status ON operation_plans
WHEN NEW.status = 'sealed' AND OLD.status = 'draft'
BEGIN
    SELECT CASE WHEN EXISTS (
        SELECT 1
        FROM (SELECT DISTINCT group_id FROM plan_items WHERE plan_id = NEW.id) AS groups_in_plan
        WHERE NOT EXISTS (
            SELECT 1 FROM plan_items keep_item
            WHERE keep_item.plan_id = NEW.id
              AND keep_item.group_id = groups_in_plan.group_id
              AND keep_item.action = 'keep'
        )
    ) THEN RAISE(ABORT, 'sealed plan must retain at least one keeper per group') END;
END;

CREATE TABLE file_transactions (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE RESTRICT,
    session_id TEXT REFERENCES scan_sessions(id) ON DELETE RESTRICT,
    plan_item_id TEXT REFERENCES plan_items(id) ON DELETE RESTRICT,
    kind TEXT NOT NULL CHECK (kind IN ('quarantine', 'restore')),
    status TEXT NOT NULL CHECK (status IN (
        'planned', 'preflight_validated', 'moving', 'moved_unverified', 'verified',
        'cancelled', 'preflight_failed', 'move_failed', 'verify_failed',
        'recovery_required', 'reconciled_source_only', 'reconciled_destination_only',
        'reconciled_both', 'reconciled_missing'
    )),
    source_path TEXT NOT NULL,
    destination_path TEXT NOT NULL,
    source_path_key BLOB NOT NULL,
    destination_path_key BLOB NOT NULL,
    volume_id TEXT NOT NULL,
    file_id TEXT NOT NULL,
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    blake3_digest BLOB NOT NULL,
    sha256_digest BLOB NOT NULL,
    source_snapshot_token BLOB NOT NULL,
    started_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    verified_at TEXT,
    error_code TEXT,
    error_message TEXT,
    UNIQUE (kind, plan_item_id)
) STRICT;

CREATE INDEX idx_file_transactions_recovery ON file_transactions(project_id, status, updated_at);

CREATE TABLE transaction_events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    transaction_id TEXT NOT NULL REFERENCES file_transactions(id) ON DELETE RESTRICT,
    from_status TEXT,
    to_status TEXT NOT NULL,
    source_exists INTEGER CHECK (source_exists IS NULL OR source_exists IN (0, 1)),
    destination_exists INTEGER CHECK (destination_exists IS NULL OR destination_exists IN (0, 1)),
    verification_result TEXT,
    error_code TEXT,
    error_message TEXT,
    occurred_at TEXT NOT NULL
) STRICT;

CREATE TRIGGER transaction_events_no_update
BEFORE UPDATE ON transaction_events BEGIN
    SELECT RAISE(ABORT, 'transaction events are append-only');
END;

CREATE TRIGGER transaction_events_no_delete
BEFORE DELETE ON transaction_events BEGIN
    SELECT RAISE(ABORT, 'transaction events are append-only');
END;

CREATE TABLE quarantine_entries (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE RESTRICT,
    origin_transaction_id TEXT NOT NULL UNIQUE REFERENCES file_transactions(id) ON DELETE RESTRICT,
    original_path TEXT NOT NULL,
    quarantine_path TEXT NOT NULL,
    volume_id TEXT NOT NULL,
    file_id TEXT NOT NULL,
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    blake3_digest BLOB NOT NULL,
    sha256_digest BLOB NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('verified', 'restoring', 'restored', 'missing', 'recovery_required')),
    quarantined_at TEXT NOT NULL,
    retain_until TEXT NOT NULL,
    last_verified_at TEXT NOT NULL,
    restored_at TEXT,
    UNIQUE (quarantine_path)
) STRICT;

CREATE TABLE scan_checkpoints (
    session_id TEXT PRIMARY KEY REFERENCES scan_sessions(id) ON DELETE CASCADE,
    stage TEXT NOT NULL,
    cursor_json TEXT NOT NULL,
    committed_items INTEGER NOT NULL CHECK (committed_items >= 0),
    updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE error_records (
    id TEXT PRIMARY KEY,
    project_id TEXT REFERENCES projects(id) ON DELETE RESTRICT,
    session_id TEXT REFERENCES scan_sessions(id) ON DELETE CASCADE,
    transaction_id TEXT REFERENCES file_transactions(id) ON DELETE RESTRICT,
    file_entry_id TEXT REFERENCES file_entries(id) ON DELETE RESTRICT,
    operation TEXT NOT NULL,
    category TEXT NOT NULL,
    os_code INTEGER,
    message TEXT NOT NULL,
    retryable INTEGER NOT NULL CHECK (retryable IN (0, 1)),
    occurred_at TEXT NOT NULL
) STRICT;

CREATE INDEX idx_error_records_session ON error_records(session_id, occurred_at);

CREATE TABLE audit_events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT NOT NULL UNIQUE,
    project_id TEXT REFERENCES projects(id) ON DELETE RESTRICT,
    session_id TEXT REFERENCES scan_sessions(id) ON DELETE RESTRICT,
    transaction_id TEXT REFERENCES file_transactions(id) ON DELETE RESTRICT,
    actor TEXT NOT NULL CHECK (actor IN ('user', 'system', 'recovery')),
    event_type TEXT NOT NULL,
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    occurred_at TEXT NOT NULL
) STRICT;

CREATE TRIGGER audit_events_no_update
BEFORE UPDATE ON audit_events BEGIN
    SELECT RAISE(ABORT, 'audit events are append-only');
END;

CREATE TRIGGER audit_events_no_delete
BEFORE DELETE ON audit_events BEGIN
    SELECT RAISE(ABORT, 'audit events are append-only');
END;

INSERT INTO schema_migrations(version, name, applied_at)
VALUES (1, 'initial_safe_duplicate_schema', '2026-07-21T00:00:00Z');

COMMIT;

