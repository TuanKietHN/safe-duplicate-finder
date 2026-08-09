BEGIN IMMEDIATE;

ALTER TABLE permanent_delete_batches
ADD COLUMN deletion_mode TEXT NOT NULL DEFAULT 'retention_expired'
    CHECK (deletion_mode IN ('retention_expired', 'immediate'));

INSERT INTO schema_migrations(version, name, applied_at)
VALUES (4, 'explicit_immediate_delete_mode', '2026-07-22T00:00:00Z');

COMMIT;
