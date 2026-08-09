# Desktop Command and Event Contract

Tauri commands are request/response operations with UUID idempotency keys. Commands never accept raw
SQL or an unchecked destination path.

## Commands

- `project_create`, `project_update`, `project_delete_record`, `project_list`
- `root_add`, `root_remove`, `root_validate`
- `scan_start`, `scan_pause`, `scan_resume`, `scan_cancel`, `scan_snapshot`
- `results_page`, `group_update_selection`, `plan_seal`, `dry_run`
- `quarantine_execute`, `quarantine_verify`, `restore_execute`
- `prepare_permanent_delete(entry_ids[], delete_now)`,
  `execute_permanent_delete(batch_id, token, confirmation)`
- `recovery_list`, `recovery_reconcile`
- `report_export`, `open_containing_folder`, `open_file_readonly`

Mutation commands require their operation-specific immutable IDs and confirmation. The backend
validates that plan evidence is current, a keeper exists where applicable, and confirmations match
exactly. Frontend selection is advisory and never bypasses backend validation.

Permanent-delete preparation accepts UUIDs only and returns exact count, bytes, expiry, token, mode,
and phrase. `delete_now=false` requires expired retention. `delete_now=true` explicitly selects
immediate mode, bypasses only the retention-time gate, and returns a different exact challenge phrase.
Execution has no raw path parameter. It revalidates the immutable, mode-bound batch and all selected
quarantine evidence before the first deletion; SQLite plus the fsynced manifest are authoritative.

## Events

Events use `{ schema_version, sequence, project_id, session_id, emitted_at, kind, payload }`.

The adapter keeps a fixed 32-event newest-only buffer and at most one pending event per scan session.
The frontend consumes it through `next_scan_event`; every event is advisory and a sequence gap or
empty poll falls back to the durable `scan_status` command. A slow or disconnected UI therefore
cannot backpressure scan/hash workers or grow memory without bound.

- `scan://snapshot`: monotonic counters and current stage/file display path
- `scan://state`: session transition with reason
- `scan://error`: isolated error classification and retryability
- `results://updated`: group count and potential savings, no full result payload
- `transaction://state`: planned/moving/verified/recovery-required transition
- `recovery://required`: unresolved transaction summary

The UI obtains pageable result details by command, not an unbounded event. Sequence gaps cause the UI
to call `scan_snapshot`; it never reconstructs safety state from events alone.
