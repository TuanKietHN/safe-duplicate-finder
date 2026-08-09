BEGIN IMMEDIATE;

ALTER TABLE scan_sessions
ADD COLUMN blocked_reason TEXT;

INSERT INTO schema_migrations(version, name, applied_at)
VALUES (5, 'scan_block_reason', '2026-07-22T00:00:00Z');

COMMIT;
