# Implementation Plan: Safe Duplicate File Management

**Branch**: `001-safe-duplicate-removal` | **Date**: 2026-08-09 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/001-safe-duplicate-removal/spec.md`

## Summary

Build a local-first Windows desktop product with a reusable Rust safety engine, a Rust CLI, a Tauri
TypeScript desktop adapter, and optional headless container. The engine inventories files once,
persists evidence in SQLite WAL, eliminates candidates through metadata and sampled reads, confirms
duplicates through separate full BLAKE3 and SHA-256 passes, and allows only dry-run or journaled,
verified quarantine/restore mutations. After those gates pass, a separate high-friction workflow can
delete only explicit quarantine entries using either retention-expired mode or a separately enabled
immediate mode that bypasses only the time gate. Both use a mode-bound short-lived token, exact phrase,
fsynced intent, and handle-bound deletion. A serialized database writer and bounded
per-volume I/O queues make crash state explainable and memory use predictable.

The Windows release is one compact Tauri NSIS setup containing a small native runtime helper that is
independent of WebView2. The helper preflights installed runtimes, downloads only missing artifacts
from pinned Microsoft HTTPS URLs, resumes verified partial files, validates the mandatory expected
size and SHA-256, and reports progress from actual received bytes. A bounded two-worker scheduler
permits independent missing artifacts to download concurrently while keeping memory below 32 MiB.
NSIS invokes the helper from a post-install hook before reporting success, then removes the temporary
helper resource and creates both application and uninstaller shortcuts. A real uninstall removes the installed program,
installer cache, database, logs, WebView profile, settings, and other app-local state; it never
silently deletes quarantined documents stored beside user source volumes.

## Technical Context

**Language/Version**: Rust 1.97.1, edition 2024; TypeScript 5.x on Node.js 24.x

**Primary Dependencies**: Tauri 2; rusqlite (bundled SQLite); blake3; sha2; crossbeam-channel;
windows-sys/windows; WinHTTP; Win32 common controls; serde; tracing; clap; globset;
unicode-normalization

**Storage**: SQLite in WAL mode with `synchronous=FULL`, foreign keys, versioned migrations; append-only
JSONL audit/transaction manifests; per-volume quarantine directories

**Testing**: `cargo test`, proptest, tempfile-based integration fixtures, deterministic fault injector,
Vitest for frontend state, Playwright/Tauri smoke tests where available

**Target Platform**: Windows 10/11 x86_64 MSVC primary; CLI/headless Linux container secondary

**Project Type**: Cargo workspace with reusable libraries, CLI, Tauri desktop application, web
frontend package, and a WebView-independent native Windows runtime helper embedded in NSIS

**Performance Goals**: scan near-1 TB; 100,000 small files; 10,000 documents; 1–20 GiB individual files;
responsive UI; bounded queues; no duplicate enumeration; installer progress refresh at least four
times per second while bytes arrive; at most two concurrent runtime downloads; no completed artifact
is downloaded again

**Constraints**: peak application memory below 2 GiB and runtime-helper memory below 32 MiB; 64-bit byte
accounting; streaming only; no default source deletion; no application telemetry or user-data network
traffic; the runtime helper may contact only pinned Microsoft runtime endpoints; fail closed on
identity/stability uncertainty; no administrator requirement for current-user installation

**Scale/Scope**: multiple roots and volumes, resumable sessions, millions of metadata rows, long paths,
Unicode, hard links, symlinks/junctions, locked and cloud-backed files

## Constitution Check

*GATE: Passed before Phase 0 research; re-checked after Phase 1 design.*

- **Data preservation — PASS**: scan-only is the first/default operation; dry-run precedes mutation;
  quarantine and restore were completed before the separately gated quarantine-only permanent-delete
  workflow.
- **Evidence — PASS**: strict grouping needs normalized name, exact `u64` size, distinct physical IDs,
  full BLAKE3 and full SHA-256, plus stable before/after observations. Quick hash only rejects.
- **Transactions — PASS**: mutation intent is committed with FULL synchronous durability before a
  handle-bound, non-overwriting same-volume rename; destination verification precedes completion.
- **Tests first — PASS**: task order will put invariant, property, transaction, recovery, and fault-
  injection tests before the corresponding implementation.
- **Bounded/local processing — PASS**: application queues have explicit capacities; a serialized DB
  writer batches state; normal application operation contacts no network service; logs omit document
  content. The separately built native helper has a two-download cap and may transmit only
  ordinary HTTPS requests for pinned Microsoft runtime binaries—never paths, filenames, hashes,
  projects, logs, or document data.
- **Safety case — PASS**: hazards and mitigations are recorded below and in `research.md`; transaction
  boundaries are captured in the failure matrix.

Post-design re-check: data model constrains sealed plans to retain a keeper; append-only event tables
reject update/delete; contracts reject mutation without explicit plan identity and confirmation. The
runtime manifest requires immutable size and SHA-256 evidence, and uninstall cleanup explicitly
excludes every per-volume quarantine root.

## Architecture Decision

The final architecture is a hexagonal Cargo workspace. `dedupe-core` owns policies, state machines,
hashing, scheduling, quarantine orchestration, and ports. `dedupe-store` is the sole SQLite adapter.
`dedupe-platform` owns filesystem identity and safe rename implementations. `dedupe-report` produces
exports. `dedupe-cli` and `apps/desktop/src-tauri` are thin adapters; neither may implement duplicate
or mutation rules. The frontend receives immutable snapshots/events and sends explicit commands.

A single database-writer actor serializes state transitions and batch commits. Enumerator and hash
workers communicate through bounded channels. Work is partitioned by volume so an HDD defaults to one
full-read worker, SATA SSD to two, NVMe to at most four, and unknown/network storage to one. The user
may lower limits; increasing them never bypasses queue bounds.

The Windows mutation adapter opens the exact source with DELETE/read-attributes access, validates
`FILE_ID_INFO` (volume serial + 128-bit file ID) on that handle, and requests a same-volume rename with
replacement disabled. The platform layer never enables copy-across-volume or replace-existing flags.
The core verifies the destination using a new handle before writing `verified`.

The release pipeline first builds the native helper with static CRT, then embeds that helper in the
single public Tauri NSIS setup. NSIS installs the application with `webviewInstallMode=skip` and calls
the helper before installation may complete. The helper uses synchronous WinHTTP reads on at
most two worker threads, 64 KiB streaming buffers, HTTP `Range` for `.part` files, atomic promotion
only after SHA-256 succeeds, and a Win32 UI fed by atomic byte counters. Content-Length determines the
real total; downloaded bytes, rolling throughput, ETA, current file state, per-item state, and the
weighted overall percentage are derived from those counters, never timers. If a server ignores a
range request, that item alone restarts from byte zero. A valid installed runtime or completed cached
artifact is reused after preflight/hash validation.

WebView2 Evergreen Standalone x64 is the only non-system runtime required by the current executable.
The MSVC dependency audit shows no `VCRUNTIME140.dll`/`MSVCP140.dll` import, so the installer MUST NOT
download the Visual C++ Redistributable. The manifest and worker pool remain multi-item capable for a
future proven dependency.

## Project Structure

### Documentation (this feature)

```text
specs/001-safe-duplicate-removal/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   ├── cli.md
│   ├── desktop-events.md
│   └── sqlite-schema.sql
├── checklists/
│   └── requirements.md
└── tasks.md
```

### Source Code (repository root)

```text
Cargo.toml
Cargo.lock
crates/
├── dedupe-core/
│   ├── src/
│   │   ├── scanner.rs
│   │   ├── metadata.rs
│   │   ├── path_normalization.rs
│   │   ├── duplicate_detector.rs
│   │   ├── quick_hash.rs
│   │   ├── full_hash.rs
│   │   ├── file_identity.rs
│   │   ├── quarantine.rs
│   │   ├── restore.rs
│   │   ├── transaction_journal.rs
│   │   ├── recovery.rs
│   │   ├── scheduler.rs
│   │   ├── project_manager.rs
│   │   ├── control.rs
│   │   ├── ports.rs
│   │   └── lib.rs
│   └── tests/
├── dedupe-store/
│   ├── migrations/
│   ├── src/
│   │   ├── database.rs
│   │   ├── writer.rs
│   │   ├── repositories.rs
│   │   └── lib.rs
│   └── tests/
├── dedupe-platform/
│   ├── src/
│   │   ├── windows.rs
│   │   ├── portable.rs
│   │   └── lib.rs
│   └── tests/
├── dedupe-report/
│   └── src/{csv.rs,json.rs,html.rs,lib.rs}
└── dedupe-testkit/
    └── src/{fixtures.rs,faults.rs,lib.rs}
apps/
├── cli/
│   └── src/main.rs
└── desktop/
    ├── src-tauri/src/{main.rs,state.rs,tauri_commands.rs,events.rs}
    ├── src-tauri/windows/{hooks.nsh,installer.nsi}
    └── src/{components,pages,stores,services,types}
├── runtime-installer/
│   ├── src/{main.rs,download.rs,manifest.rs,preflight.rs,progress.rs,ui.rs}
│   └── tests/{download_resume.rs,manifest.rs,progress.rs}
installer/
├── runtime-manifest.json
└── windows/{build-online-installer.ps1,verify-installer.ps1}
tests/
├── integration/
├── recovery/
├── properties/
└── conformance/
benchmarks/
├── src/main.rs
└── README.md
docs/
├── architecture.md
├── threat-model.md
├── user-guide.md
├── manual-recovery.md
├── build-windows.md
├── docker.md
└── diagrams/
Dockerfile
```

**Structure Decision**: Separate domain, storage, and platform crates prevent UI/CLI code from
creating alternate safety logic. `dedupe-testkit` exposes deterministic failures only to tests and
benchmark binaries, never to production commands.

## Implementation Phases

1. **Governance and design**: constitution, spec, threat model, schema, state machines, contracts.
2. **Read-only foundation**: project persistence, overlap-safe enumeration, metadata, control token,
   progress and errors.
3. **Evidence pipeline**: grouping, sampled rejector, streaming BLAKE3, streaming SHA-256, stability
   and hard-link classification.
4. **Review-only features**: keep rules, sealed operation plans, dry-run, reports, desktop/CLI review.
5. **Mutation safety**: transaction journal, preflight, same-volume handle-bound quarantine, destination
   verification, restore, startup reconciliation, fault injection.
6. **Adapters**: full CLI, responsive Tauri UI, native open-file/folder and optional Recycle Bin adapter.
7. **Packaging and proof**: Docker scan/headless image, benchmarks, Windows NSIS/MSI build, manuals,
   crash/power-loss test report.
8. **Deferred deletion**: only after all prior gates pass; separate feature approval and test evidence.
9. **Online Windows release**: native runtime preflight/downloader, real-byte progress, resume and
   mandatory SHA-256, compact NSIS payload, uninstaller shortcut, scoped full app-data cleanup,
   clean-machine/retry/resume/tamper/uninstall qualification, and release EXE checksum evidence.

## Runtime Download and Installer State Machines

```text
runtime item:
  pending -> prechecking -> installed_valid | cache_valid | downloading
  downloading -> paused_partial | verifying
  paused_partial -> downloading
  verifying -> cache_valid | integrity_failed
  cache_valid -> installing -> installed_valid | install_failed

release:
  initializing -> prechecking -> downloading -> verifying -> installing_runtime
  -> installing_application -> completed
  any active state -> cancelled | failed (resume keeps valid cache/partial evidence)
```

`received_bytes` is the sum of bytes physically present in valid completed cache files plus current
`.part` lengths; `network_bytes_this_run` is separate. Overall progress is
`received_bytes / required_download_bytes`, clamped monotonically only while the manifest is fixed.
Speed uses a rolling window over newly received network bytes. ETA is absent until speed is non-zero.
Install-only states retain 100% download progress and show their own textual status.

## Installer Resource Budget

| Resource | Budget | Enforcement |
|----------|--------|-------------|
| Concurrent downloads | 2 maximum | Fixed worker semaphore; one request handle per item |
| Read buffer | 64 KiB per worker | Streaming WinHTTP reads; no whole-file allocation |
| Runtime-helper memory | < 32 MiB peak | Clean-machine qualification and bounded collections |
| Retry | 3 attempts per item | Exponential backoff; valid `.part` retained |
| Cache | Content-addressed by SHA-256 | Rehash before reuse; invalid completed files quarantined/deleted |
| Release size | Online setup < 15 MiB target | LTO, one codegen unit, stripped symbols; runtimes not embedded |

## Data-Loss Safety Case

| Hazard | Prevention | Detection | Recovery / Evidence |
|--------|------------|-----------|---------------------|
| False duplicate | Two full digests, exact size/name policy, stable identity | Before/after snapshots | Skip as unstable; invariant tests |
| Path swapped after scan | Open handle and compare physical identity | Handle-bound `FILE_ID_INFO` | Abort transaction before move |
| Hard link counted twice | Volume + 128-bit file ID grouping | Alias classification | Report only; no reclaim action |
| Destination overwrite | Unique ID path; replacement disabled | Destination existence/open checks | Leave source unchanged; error event |
| Crash around move | Durable planned/moving events | Startup source/destination reconciliation | Preserve both if ambiguous; resume/restore |
| Last copy removed | Sealed-plan keeper constraint and preflight count | Group invariant query | Block plan/mutation |
| Cross-volume copy/delete | Per-volume quarantine and same-volume assertion | Volume identity comparison | Fail closed; no fallback copy/delete |
| DB/log cannot persist | FULL sync commit and mandatory audit flush before move | Commit/flush result | Abort before filesystem mutation |
| Destination bytes damaged | Full size and digest verification | Verification state | Never mark complete/reclaimed; alert user |
| Restore collision | Never overwrite original path | Destination existence and identity | Ask for alternate destination or cancel |
| Runtime response truncated | Expected length and SHA-256 | Stream byte count and final digest | Keep `.part`; resume only with HTTP 206 |
| Runtime server/content changed | Pinned HTTPS URL, size and SHA-256 | Final digest mismatch | Reject artifact; never execute |
| Installer interrupted | Content-addressed cache and `.part` | Startup preflight | Reuse verified work; resume missing suffix |
| Uninstall removes real documents | Cleanup allowlist excludes quarantine roots | Path-prefix tests and UI disclosure | Preserve quarantine and manual recovery manifest |

Residual risks: underlying storage or RAM may corrupt data while returning successful reads/writes;
cryptographic digest collision is theoretically non-zero; malicious kernel/filesystem drivers can
violate API guarantees. The two independent digests, destination reread, journaling, and retained copy
reduce but cannot mathematically eliminate those hardware/OS trust risks.

## Transaction Failure Matrix

| Injection boundary | Durable state | Expected filesystem state | Startup action |
|--------------------|---------------|---------------------------|----------------|
| Before `planned` commit | none | source only | No transaction; rescan safely |
| After `planned`, before preflight | planned | source only | Revalidate or cancel plan |
| After `moving`, before rename | moving | source only | Revalidate; retry only with consent |
| Rename returns error | moving + error event | source expected | Inspect both paths; unresolved on ambiguity |
| After rename, before DB update | moving | destination expected | Verify destination, then mark verified |
| After DB transition, before digest verify | moving | destination expected | Re-run destination verification |
| Destination and source both exist | moving | both | Preserve both; compare; require safe resolution |
| Commit/log failure after rename | moving or last durable state | destination expected | Reconcile from paths and evidence; never assume |
| Volume disconnect | last durable state | unknown | Mark blocked; retry after same volume returns |
| After verified commit | verified | destination only | No mutation; optionally audit re-verification |

## Installer Failure Matrix

| Boundary | Durable/cache state | Expected next run |
|----------|---------------------|-------------------|
| Before response headers | no new bytes | Retry request; progress remains zero |
| During download | `.part` with exact current length | Send `Range: bytes=<length>-`; accept only 206 for append |
| Server returns 200 to resume | old `.part` retained until decision | Truncate that item and restart; do not append full response |
| Size mismatch | `.part` retained as invalid evidence | Fail closed and offer retry; never execute |
| SHA-256 mismatch | invalid file is not promoted | Report integrity failure; fresh retry required |
| Crash after hash before rename | complete `.part` | Rehash and atomically promote on restart |
| Runtime installer fails | verified cache retained | Report exit code; retry installation without redownload |
| NSIS application install or runtime hook fails | verified runtime cache retained, NSIS error recorded | Retry setup; do not redownload verified runtimes |
| Uninstall during update | update flag set | Preserve app data; replace binaries only |
| Explicit uninstall | ordinary uninstaller run | Delete app-local data/cache/logs/settings and shortcuts; preserve quarantine |

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| Multiple Rust crates | Enforces one reusable safety engine and platform/storage isolation | A single crate makes UI/CLI shortcuts and unsafe dependency coupling easier |
| Serialized DB writer actor | Deterministic state ordering and fewer SQLite busy races | Arbitrary worker writes complicate recovery and WAL contention |
| Two full file passes | Required independent confirmation after BLAKE3 grouping | One digest or one combined pass cannot satisfy staged confirmation policy |
