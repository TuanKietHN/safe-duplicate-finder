# Data Model and State Machines

The normative SQL contract is [contracts/sqlite-schema.sql](contracts/sqlite-schema.sql). All IDs are
UUID text; byte counts are non-negative signed 64-bit SQLite integers constrained below `i64::MAX`.

## Entities and Relationships

- A **Project** has many **Project Roots**, **Filter Rules**, and **Scan Sessions**.
- A **Scan Session** has many **File Entries**, **Duplicate Groups**, **Operation Plans**, errors, and
  progress checkpoints.
- A **File Entry** is a logical normalized path; each scan creates one immutable **File Snapshot**.
- A **File Snapshot** has zero or more staged **Hash Results** and may share a physical identity with
  hard-link aliases.
- A **Duplicate Group** has **Duplicate Members**. Distinct members must not share the same physical
  identity. Exactly one or more members are keepers in every sealed plan.
- An **Operation Plan** freezes selected evidence; its **Plan Items** choose keep or quarantine.
- A **File Transaction** journals quarantine or restore. It has append-only **Transaction Events** and
  may create/update a **Quarantine Entry** only after verification.
- **Audit Events** and **Error Records** reference the owning project/session/transaction when known.

## File Processing State Machine

```text
discovered
  -> metadata_ready
  -> candidate | unique | hardlink_alias | skipped | error
candidate
  -> quick_rejected | quick_hashed
quick_hashed
  -> blake3_rejected | blake3_hashed
blake3_hashed
  -> sha256_rejected | duplicate_confirmed
duplicate_confirmed
  -> planned_keep | planned_quarantine
planned_quarantine
  -> preflight_failed | moving
moving
  -> quarantined_unverified | recovery_required
quarantined_unverified
  -> quarantined_verified | recovery_required
quarantined_verified
  -> restoring
restoring
  -> restored_unverified | recovery_required
restored_unverified
  -> restored_verified | recovery_required

Any evidence stage -> unstable when identity, size, or modification evidence changes.
unstable, unique, hardlink_alias, skipped, and error cannot transition to mutation states.
```

## Scan Session State Machine

```text
draft -> enumerating -> quick_hashing -> blake3_hashing -> sha256_hashing -> grouping -> completed
                  \            \              \               \
                   -> pausing -> paused -> prior active stage
Any active stage -> cancelling -> cancelled
Any active stage -> interrupted -> recovering -> paused or prior active stage
Fatal database integrity failure -> blocked (read-only diagnostics only)
```

## Quarantine / Restore Transaction State Machine

```text
planned -> preflight_validated -> moving -> moved_unverified -> verified
   |              |                 |             |
cancelled    preflight_failed   move_failed   verify_failed
                                      \          /
                                      recovery_required
                                             |
                                   reconciled_source_only
                                   reconciled_destination_only
                                   reconciled_both
                                   reconciled_missing
```

Only `verified` contributes reclaimed bytes. A new append-only event records each transition. Recovery
may transition an incomplete transaction forward only after reproducing the same verification; it may
never erase or rewrite an older event.

## Permanent Delete State Machines

```text
batch: prepared -> executing -> completed
          |            |
        expired     recovery_required -> executing

item: planned -> deleting -> deleted
          ^          |
          |        failed -> deleted (missing-after-intent recovery)
          +-----------+  (explicit same-item retry)
```

`DeletionEntry` contains only the registry UUID, project, quarantine path, identity, size, full
digests, and retention timestamp. It cannot represent an original/source path. Batch authorization
stores the selected deletion mode, only a digest of a short-lived token, and a digest of the exact
sorted selection. Append-only permanent-delete events and audit rows are fsynced/committed around
every irreversible boundary.

## Validation Rules

1. Normalized paths are unique per project, compared case-insensitively for Windows semantics while
   preserving the original path for display.
2. Physical identity is `(volume_serial, file_id_128)`; equal identity means hard-link alias, not an
   independently reclaimable duplicate.
3. Hash results include algorithm, stage, bytes read, source snapshot, and completion time.
4. A confirmed strict group has equal normalized names, sizes, BLAKE3, and SHA-256 across members.
5. A sealed plan cannot contain a group without a `keep` item.
6. A transaction references an immutable sealed plan item and copies its evidence snapshot.
7. Append-only audit and transaction event rows reject update and delete.
8. A quarantine entry becomes `verified` only when its transaction is verified.
9. Permanent deletion requires `state=verified`, `permanent_delete_state=active`, and an explicit UUID
   selection. Normal mode additionally requires expired retention; immediate mode explicitly bypasses
   only that time gate and binds a different exact phrase to the durable batch.
10. `automatic_permanent_delete` remains zero in both deletion modes.
11. A batch can complete only when every item is durably `deleted`; a missing path counts only after
    a durable `deleting` intent proves an interrupted authorized system call.

## Windows Installer Models (file-backed, outside the application database)

The native runtime helper MUST remain usable before SQLite and WebView2 exist. Installer state is therefore
file-backed under `%LOCALAPPDATA%\io.github.safeduplicate.finder\installer-cache` and MUST NOT reuse
the application database.

### RuntimeManifest

```text
schema_version: u32
release_version: string
architecture: x86_64-windows
artifacts[]: RuntimeArtifact
```

### RuntimeArtifact

```text
id: stable ASCII identifier
display_name: localized label
architecture: x64
url: pinned https URL
size_bytes: u64
sha256: 64 hexadecimal characters
cache_file_name: content-addressed executable name
install_args[]: exact argument vector
detection: versioned registry/file rule
max_retries: 3
```

### RuntimeTransfer

```text
artifact_id
state: pending | prechecking | installed_valid | cache_valid | downloading |
       verifying | installing | completed | cancelled | failed
part_path
complete_path
resume_offset: u64
received_bytes: u64
network_bytes_this_run: u64
retry_count: u8
sha256_result: optional digest
installer_exit_code: optional i32
last_error: optional non-sensitive message
```

The `.part` length is the only durable resume offset. A sidecar may retain URL/ETag diagnostics, but
never overrides the embedded manifest. A completed cache file is authoritative only after its exact
length and full SHA-256 are revalidated in the current session.

### InstallerProgressSnapshot

```text
manifest_id
stage
required_download_bytes: u64
received_bytes: u64
network_bytes_this_run: u64
bytes_per_second: u64
eta_seconds: optional u64
overall_basis_points: 0..10000
current_artifact_id: optional string
items[]: { artifact_id, state, received_bytes, size_bytes, message }
```

Snapshots are derived from atomic counters and emitted at most ten times per second. The UI never
increments counters itself. `overall_basis_points = floor(received_bytes * 10000 /
required_download_bytes)` with a checked wide intermediate; the completed/no-download case is 10000.

### Installer State Machines

```text
artifact:
pending -> prechecking -> installed_valid
                       -> cache_valid
                       -> downloading -> verifying -> cache_valid -> installing -> completed
                             |               |             |              |
                          cancelled    integrity_failed  invalid       install_failed

session:
initializing -> prechecking -> downloading -> verifying -> installing_runtime
             -> installing_application -> completed
any active state -> cancelled | failed
```

### Installer Validation Rules

1. Only HTTPS manifest URLs are accepted; redirects remain HTTPS.
2. Download append is legal only for status 206 whose Content-Range begins at the `.part` length.
3. Status 200 during resume truncates/restarts that artifact; it is never appended.
4. Exact size and SHA-256 are both mandatory before atomic `.part` promotion or execution.
5. At most two artifact transfers are active; each uses one 64 KiB buffer.
6. A preinstalled valid runtime is excluded from required download bytes.
7. Cached verified artifacts contribute their full size to received bytes without adding to
   `network_bytes_this_run`.
8. Explicit uninstall recursively removes only fixed product install/app-data/cache roots. Upgrade
   mode preserves data. Per-volume `.safe-duplicate-finder-quarantine` paths are never cleanup roots.
