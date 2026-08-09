BEGIN IMMEDIATE;

CREATE INDEX IF NOT EXISTS idx_duplicate_members_snapshot
    ON duplicate_members(snapshot_id, group_id);
CREATE INDEX IF NOT EXISTS idx_operation_plans_session_created
    ON operation_plans(session_id, created_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_plan_items_snapshot
    ON plan_items(snapshot_id, group_id, plan_id);
CREATE INDEX IF NOT EXISTS idx_file_transactions_plan_item
    ON file_transactions(plan_item_id, kind, updated_at DESC);

INSERT INTO schema_migrations(version, name, applied_at)
VALUES (6, 'history_query_indexes', '2026-07-23T00:00:00Z');

COMMIT;
