BEGIN IMMEDIATE;

ALTER TABLE quarantine_entries ADD COLUMN permanent_delete_state TEXT NOT NULL DEFAULT 'active'
    CHECK (permanent_delete_state IN ('active', 'deleting', 'deleted', 'failed'));
ALTER TABLE quarantine_entries ADD COLUMN permanent_delete_batch_id TEXT;
ALTER TABLE quarantine_entries ADD COLUMN deleted_at TEXT;

CREATE TABLE permanent_delete_batches (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE RESTRICT,
    status TEXT NOT NULL CHECK (status IN (
        'prepared', 'executing', 'completed', 'recovery_required', 'expired'
    )),
    token_digest BLOB NOT NULL CHECK (length(token_digest) = 32),
    selection_digest BLOB NOT NULL CHECK (length(selection_digest) = 32),
    confirmation_phrase TEXT NOT NULL,
    entry_count INTEGER NOT NULL CHECK (entry_count > 0),
    total_bytes INTEGER NOT NULL CHECK (total_bytes >= 0),
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    started_at TEXT,
    completed_at TEXT,
    error_message TEXT
) STRICT;

CREATE TABLE permanent_delete_items (
    batch_id TEXT NOT NULL REFERENCES permanent_delete_batches(id) ON DELETE RESTRICT,
    entry_id TEXT NOT NULL REFERENCES quarantine_entries(id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    status TEXT NOT NULL CHECK (status IN ('planned', 'deleting', 'deleted', 'failed')),
    quarantine_path TEXT NOT NULL,
    volume_id TEXT NOT NULL,
    file_id TEXT NOT NULL,
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    blake3_digest BLOB NOT NULL CHECK (length(blake3_digest) = 32),
    sha256_digest BLOB NOT NULL CHECK (length(sha256_digest) = 32),
    retain_until TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    error_message TEXT,
    PRIMARY KEY (batch_id, entry_id),
    UNIQUE (batch_id, ordinal)
) STRICT;

CREATE INDEX idx_permanent_delete_batches_project
    ON permanent_delete_batches(project_id, created_at DESC);
CREATE INDEX idx_permanent_delete_items_entry
    ON permanent_delete_items(entry_id, updated_at DESC);

CREATE TABLE permanent_delete_events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    batch_id TEXT NOT NULL REFERENCES permanent_delete_batches(id) ON DELETE RESTRICT,
    entry_id TEXT REFERENCES quarantine_entries(id) ON DELETE RESTRICT,
    from_status TEXT,
    to_status TEXT NOT NULL,
    event_type TEXT NOT NULL CHECK (event_type IN ('batch', 'item')),
    error_message TEXT,
    occurred_at TEXT NOT NULL
) STRICT;

CREATE TRIGGER permanent_delete_events_no_update
BEFORE UPDATE ON permanent_delete_events BEGIN
    SELECT RAISE(ABORT, 'permanent delete events are append-only');
END;

CREATE TRIGGER permanent_delete_events_no_delete
BEFORE DELETE ON permanent_delete_events BEGIN
    SELECT RAISE(ABORT, 'permanent delete events are append-only');
END;

INSERT INTO schema_migrations(version, name, applied_at)
VALUES (3, 'quarantine_only_permanent_delete', '2026-07-22T00:00:00Z');

COMMIT;
