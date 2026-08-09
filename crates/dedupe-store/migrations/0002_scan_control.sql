BEGIN IMMEDIATE;

ALTER TABLE scan_sessions ADD COLUMN control_request TEXT NOT NULL DEFAULT 'none'
    CHECK (control_request IN ('none', 'pause', 'resume', 'cancel'));
ALTER TABLE scan_sessions ADD COLUMN resume_state TEXT
    CHECK (resume_state IS NULL OR resume_state IN (
        'enumerating', 'quick_hashing', 'blake3_hashing', 'sha256_hashing', 'grouping', 'recovering'
    ));

INSERT INTO schema_migrations(version, name, applied_at)
VALUES (2, 'durable_scan_control', '2026-07-22T00:00:00Z');

COMMIT;
