---
description: "Test-first implementation tasks for Safe Duplicate File Management"
---

# Tasks: Safe Duplicate File Management

**Input**: `spec.md`, `plan.md`, `research.md`, `data-model.md`, `contracts/`, `quickstart.md`

**Tests**: Mandatory. Every safety-relevant behavior is specified as a failing test before production
implementation. Task completion requires the named test/build gate to pass.

**Format**: `[ID] [P?] [Story?] Description with exact file path`

## Phase 1: Setup and Reproducible Tooling

**Purpose**: Create a buildable workspace with locked dependencies and no generated artifacts tracked.

- [X] T001 Initialize Git repository and safe ignore rules in `.gitignore` and `.dockerignore`
- [X] T002 Create Cargo workspace, pinned MSRV, profiles, and shared dependencies in `Cargo.toml` and `rust-toolchain.toml`
- [X] T003 [P] Create crate manifests and module roots under `crates/dedupe-core`, `crates/dedupe-store`, `crates/dedupe-platform`, `crates/dedupe-report`, and `crates/dedupe-testkit`
- [X] T004 [P] Create CLI manifest and entry point in `apps/cli/Cargo.toml` and `apps/cli/src/main.rs`
- [X] T005 [P] Create Tauri/TypeScript application manifests in `apps/desktop/package.json`, `apps/desktop/src-tauri/Cargo.toml`, and `apps/desktop/src-tauri/tauri.conf.json`
- [X] T006 [P] Configure Rust formatting, linting, audit, and test commands in `.cargo/config.toml`, `clippy.toml`, and `deny.toml`
- [X] T007 [P] Configure TypeScript, Vite, ESLint, Prettier, and Vitest in `apps/desktop/tsconfig.json`, `vite.config.ts`, `eslint.config.js`, and `vitest.config.ts`
- [X] T008 Create CI build matrix and safety test gates in `.github/workflows/ci.yml`

**Checkpoint**: Empty workspace builds, formats, and lints on the installed toolchains.

---

## Phase 2: Foundational Safety Infrastructure

**Purpose**: Shared types, deterministic failures, persistence, logging, and control primitives that
block every user story.

- [X] T009 [P] Write data-loss threat model and invariant traceability in `docs/threat-model.md`
- [X] T010 [P] Implement deterministic fixture builder in `crates/dedupe-testkit/src/fixtures.rs`
- [X] T011 [P] Implement injectable filesystem/database/log failure boundaries in `crates/dedupe-testkit/src/faults.rs`
- [X] T012 [P] Write property tests for transaction/file state transition legality in `tests/properties/state_machines.rs`
- [X] T013 Implement domain IDs, byte-safe counters, modes, states, and snapshots in `crates/dedupe-core/src/model.rs`
- [X] T014 [P] Implement typed error taxonomy with retryability and path-safe context in `crates/dedupe-core/src/error.rs`
- [X] T015 Define storage, filesystem, clock, audit, and progress ports in `crates/dedupe-core/src/ports.rs`
- [X] T016 [P] Implement cooperative pause/resume/cancel control token in `crates/dedupe-core/src/control.rs`
- [X] T017 [P] Write control-token concurrency tests in `crates/dedupe-core/tests/control.rs`
- [X] T018 [P] Implement structured JSONL and readable text logging without document contents in `crates/dedupe-core/src/logging.rs`
- [X] T019 [P] Write append-only audit/log failure tests in `tests/integration/logging.rs`
- [X] T020 Copy and version the normative schema into `crates/dedupe-store/migrations/0001_initial.sql`
- [X] T021 Write schema, WAL, FULL sync, foreign-key, append-only-trigger, and migration tests in `crates/dedupe-store/tests/migrations.rs`
- [X] T022 Implement guarded database open, integrity check, migration, and backup in `crates/dedupe-store/src/database.rs`
- [X] T023 Implement bounded serialized database writer with batch transactions in `crates/dedupe-store/src/writer.rs`
- [X] T024 Implement typed repositories and idempotent writes in `crates/dedupe-store/src/repositories.rs`
- [X] T025 [P] Write DB-full, busy, interrupted-commit, and writer-shutdown tests in `crates/dedupe-store/tests/writer.rs`
- [X] T026 [P] Define portable and Windows physical-file identity contracts in `crates/dedupe-platform/src/lib.rs`
- [X] T027 [P] Implement monotonic progress snapshots and bounded event fan-out in `crates/dedupe-core/src/progress.rs`

**Checkpoint**: Foundational unit/property/integration tests pass; no filesystem mutation API is exposed.

---

## Phase 3: User Story 1 - Configure and Run a Read-Only Scan (Priority: P1) 🎯 MVP

**Goal**: Persist projects and roots, enumerate eligible physical files once, and collect safe metadata
without mutation.

**Independent Test**: Overlapping multi-volume roots plus locked/Unicode/long-path fixtures inventory
eligible physical files once, survive restart, and leave fixture metadata/content unchanged.

### Tests for User Story 1

- [X] T028 [P] [US1] Write project/root persistence and restart tests in `tests/integration/project_persistence.rs`
- [X] T029 [P] [US1] Write duplicate and parent/child root overlap tests in `crates/dedupe-core/tests/root_selection.rs`
- [X] T030 [P] [US1] Write extension, glob, size, hidden/system, and quarantine filter tests in `crates/dedupe-core/tests/filters.rs`
- [X] T031 [P] [US1] Write Unicode, long-path, missing, locked, permission, sparse, and cloud-placeholder scan tests in `tests/integration/scanner_edge_cases.rs`
- [X] T032 [P] [US1] Write symlink/junction no-follow and loop tests in `tests/integration/link_traversal.rs`
- [X] T033 [US1] Write bounded-queue and no-duplicate-enumeration tests in `tests/integration/scanner_backpressure.rs`

### Implementation for User Story 1

- [X] T034 [P] [US1] Implement project create/update/archive and configuration persistence in `crates/dedupe-core/src/project_manager.rs`
- [X] T035 [P] [US1] Implement Windows-aware path normalization and path keys in `crates/dedupe-core/src/path_normalization.rs`
- [X] T036 [P] [US1] Implement include/exclude filter compilation in `crates/dedupe-core/src/filters.rs`
- [X] T037 [P] [US1] Implement portable metadata snapshots in `crates/dedupe-core/src/metadata.rs`
- [X] T038 [P] [US1] Implement Windows `FILE_ID_INFO`, attributes, links, and long paths in `crates/dedupe-platform/src/windows.rs`
- [X] T039 [P] [US1] Implement conservative portable identity fallback in `crates/dedupe-platform/src/portable.rs`
- [X] T040 [US1] Implement overlap-safe, cycle-safe, read-only enumeration with bounded queues in `crates/dedupe-core/src/scanner.rs`
- [X] T041 [US1] Persist scan sessions, checkpoints, file entries, snapshots, and isolated errors in `crates/dedupe-store/src/scan_repository.rs`
- [X] T042 [US1] Expose project/root/scan commands in `apps/cli/src/commands/project.rs` and `apps/cli/src/commands/scan.rs`

**Checkpoint**: Scan-only MVP passes US1 tests and performs no source mutation.

---

## Phase 4: User Story 2 - Proven Duplicate Groups and Dry Run (Priority: P1)

**Goal**: Produce only stable, independently proven duplicate groups and immutable dry-run plans.

**Independent Test**: Same-name/same-size one-byte-different files, identical files, renamed identical
files, hard links, and files changing during reads are all classified exactly as specified.

### Tests for User Story 2

- [X] T043 [P] [US2] Write sampled-hash reject-only and boundary-size tests in `crates/dedupe-core/tests/quick_hash.rs`
- [X] T044 [P] [US2] Write streaming BLAKE3/SHA-256 tests for empty, small, 4+ GiB sparse, and cancellation fixtures in `crates/dedupe-core/tests/full_hash.rs`
- [X] T045 [P] [US2] Write same-name/same-size one-byte-difference and exact-content group tests in `tests/integration/duplicate_detection.rs`
- [X] T046 [P] [US2] Write strict versus acknowledged content-mode tests in `crates/dedupe-core/tests/comparison_modes.rs`
- [X] T047 [P] [US2] Write changed-during-hash and replaced-path stability tests in `tests/integration/unstable_files.rs`
- [X] T048 [P] [US2] Write hard-link alias and reclaimable-byte tests in `tests/integration/hard_links.rs`
- [X] T049 [P] [US2] Write keep-policy tie/conflict and at-least-one-keeper property tests in `tests/properties/keep_policy.rs`
- [X] T050 [US2] Write dry-run zero-mutation and sealed-plan staleness tests in `tests/integration/dry_run.rs`

### Implementation for User Story 2

- [X] T051 [P] [US2] Implement domain-separated head/middle/tail sampled hashing in `crates/dedupe-core/src/quick_hash.rs`
- [X] T052 [P] [US2] Implement cancellable streaming BLAKE3 and SHA-256 passes in `crates/dedupe-core/src/full_hash.rs`
- [X] T053 [P] [US2] Implement snapshot-token stability comparisons in `crates/dedupe-core/src/file_identity.rs`
- [X] T054 [US2] Implement metadata/quick/BLAKE3/SHA candidate pipeline and hard-link exclusion in `crates/dedupe-core/src/duplicate_detector.rs`
- [X] T055 [US2] Persist staged hashes and duplicate groups idempotently in `crates/dedupe-store/src/duplicate_repository.rs`
- [X] T056 [P] [US2] Implement default/manual keep rules in `crates/dedupe-core/src/keep_policy.rs`
- [X] T057 [US2] Implement sealed operation plans, keeper validation, and dry-run totals in `crates/dedupe-core/src/dry_run.rs`
- [X] T058 [US2] Persist operation plans/items with evidence versions in `crates/dedupe-store/src/plan_repository.rs`
- [X] T059 [US2] Expose results/plan/dry-run CLI commands in `apps/cli/src/commands/results.rs` and `apps/cli/src/commands/plan.rs`

**Checkpoint**: SI-002 through SI-005 and dry-run tests pass; still no production mutation API.

---

## Phase 5: User Story 3 - Pause, Resume, Cancel, and Recover Scan (Priority: P1)

**Goal**: Control and resume long scans from durable safe boundaries.

**Independent Test**: Pause, cancel, terminate, and resume at every evidence stage without database
corruption or unsafe reuse of stale evidence.

### Tests for User Story 3

- [X] T060 [P] [US3] Write pause/resume/cancel tests for enumeration and hashing in `tests/integration/scan_control.rs`
- [X] T061 [P] [US3] Write abrupt-process-stop checkpoint/restart tests in `tests/recovery/scan_resume.rs`
- [X] T062 [P] [US3] Write stale-cache invalidation tests after identity/size/time change in `tests/recovery/stale_evidence.rs`

### Implementation for User Story 3

- [X] T063 [US3] Implement bounded per-volume worker scheduling and safe work boundaries in `crates/dedupe-core/src/scheduler.rs`
- [X] T064 [US3] Integrate cooperative control and durable checkpoints into `crates/dedupe-core/src/scan_service.rs`
- [X] T065 [US3] Implement startup scan-session recovery and evidence invalidation in `crates/dedupe-core/src/scan_recovery.rs`
- [X] T066 [US3] Expose pause/resume/cancel/status commands in `apps/cli/src/commands/scan.rs`

**Checkpoint**: P1 read-only workflow is complete, resumable, bounded, and fully tested.

---

## Phase 6: User Story 4 - Verified Quarantine Transactions (Priority: P2)

**Goal**: Move approved duplicates only through durable, same-volume, no-overwrite transactions.

**Independent Test**: Inject failure before/during/after every journal, move, commit, log, and verify
boundary; every restart preserves all extant copies and returns an explainable state.

### Tests for User Story 4

- [X] T067 [P] [US4] Write transaction state and append-only event tests in `crates/dedupe-store/tests/transactions.rs`
- [X] T068 [P] [US4] Write preflight changed-file, last-keeper, and stale-plan tests in `tests/integration/quarantine_preflight.rs`
- [X] T069 [P] [US4] Write destination collision, wrong-volume, denied, full-volume, and disconnect tests in `tests/integration/quarantine_failures.rs`
- [X] T070 [US4] Write fault-injection matrix for crash before move through post-move verification in `tests/recovery/quarantine_matrix.rs`
- [X] T071 [P] [US4] Write log failure and database commit failure mutation-block tests in `tests/recovery/durability_failures.rs`
- [X] T072 [P] [US4] Write destination corruption and reclaimed-byte gating tests in `tests/integration/quarantine_verification.rs`
- [X] T073 [P] [US4] Write no-overwrite and source-identity race tests for the Windows adapter in `crates/dedupe-platform/tests/windows_safe_move.rs`

### Implementation for User Story 4

- [X] T074 [P] [US4] Implement append-only SQLite/JSONL transaction journal in `crates/dedupe-core/src/transaction_journal.rs`
- [X] T075 [P] [US4] Implement unique per-volume quarantine path allocation in `crates/dedupe-core/src/quarantine_layout.rs`
- [X] T076 [P] [US4] Implement handle-bound no-replace same-volume rename on Windows in `crates/dedupe-platform/src/windows_move.rs`
- [X] T077 [P] [US4] Implement no-replace same-filesystem portable rename in `crates/dedupe-platform/src/portable_move.rs`
- [X] T078 [US4] Implement preflight, planned/moving/verified ordering, destination verification, and reclaimed-byte gating in `crates/dedupe-core/src/quarantine.rs`
- [X] T079 [US4] Implement transaction/event/quarantine-entry repositories in `crates/dedupe-store/src/transaction_repository.rs`
- [X] T080 [US4] Implement startup source/destination reconciliation without history rewrite in `crates/dedupe-core/src/recovery.rs`
- [X] T081 [US4] Expose quarantine and recovery inspection/reconcile commands in `apps/cli/src/commands/quarantine.rs` and `apps/cli/src/commands/recover.rs`

**Checkpoint**: All quarantine fault-injection tests pass; only verified destinations count as reclaimed.

---

## Phase 7: User Story 5 - Restore Quarantined Files (Priority: P2)

**Goal**: Restore individual entries, groups, or sessions to original paths without overwrite.

**Independent Test**: Restore verified entries across free, occupied, missing-volume, permission, and
injected-crash destinations and verify original content evidence.

### Tests for User Story 5

- [X] T082 [P] [US5] Write conflict-free and destination-collision restore tests in `tests/integration/restore.rs`
- [X] T083 [P] [US5] Write restore interruption/reconciliation matrix in `tests/recovery/restore_matrix.rs`
- [X] T084 [P] [US5] Write group/session restore and idempotent rerun tests in `tests/integration/restore_batch.rs`

### Implementation for User Story 5

- [X] T085 [US5] Implement journaled, verified, no-overwrite restore service in `crates/dedupe-core/src/restore.rs`
- [X] T086 [US5] Extend transaction persistence for restore and batch correlation in `crates/dedupe-store/src/transaction_repository.rs`
- [X] T087 [US5] Extend startup reconciliation for interrupted restore in `crates/dedupe-core/src/recovery.rs`
- [X] T088 [US5] Expose restore commands in `apps/cli/src/commands/restore.rs`

**Checkpoint**: Quarantine is demonstrably reversible and SI-006 through SI-009 pass.

---

## Phase 8: User Story 6 - Logs, Reports, and Desktop UX (Priority: P2)

**Goal**: Deliver a responsive project/folder/scan/results/quarantine UI and consistent reports.

**Independent Test**: Live progress stays responsive under a reference scan and CSV/JSON/HTML agree on
counts, evidence, warnings, and savings without document content.

### Tests for User Story 6

- [X] T089 [P] [US6] Write cross-format report conformance and content-leak tests in `crates/dedupe-report/tests/conformance.rs`
- [X] T090 [P] [US6] Write Tauri command authorization/idempotency tests in `apps/desktop/src-tauri/tests/commands.rs`
- [X] T091 [P] [US6] Write frontend project/folder state and no-auto-scan tests in `apps/desktop/src/stores/project.test.ts`
- [X] T092 [P] [US6] Write progress, result selection, confirmation, and recovery UI tests in `apps/desktop/src/pages/workflows.test.tsx`

### Implementation for User Story 6

- [X] T093 [P] [US6] Implement streaming CSV export in `crates/dedupe-report/src/csv.rs`
- [X] T094 [P] [US6] Implement streaming JSON export in `crates/dedupe-report/src/json.rs`
- [X] T095 [P] [US6] Implement escaped self-contained HTML export in `crates/dedupe-report/src/html.rs`
- [X] T096 [US6] Implement Tauri service state, validated commands, and bounded events in `apps/desktop/src-tauri/src/state.rs`, `tauri_commands.rs`, and `events.rs`
- [X] T097 [P] [US6] Implement reusable frontend types/services/stores in `apps/desktop/src/types/index.ts`, `services/backend.ts`, and `stores/`
- [X] T098 [P] [US6] Implement Project and Folder screens in `apps/desktop/src/pages/ProjectsPage.tsx` and `FoldersPage.tsx`
- [X] T099 [P] [US6] Implement Scan and Results screens in `apps/desktop/src/pages/ScanPage.tsx` and `ResultsPage.tsx`
- [X] T100 [P] [US6] Implement Quarantine and Recovery screens in `apps/desktop/src/pages/QuarantinePage.tsx` and `RecoveryPage.tsx`
- [X] T101 [US6] Implement accessible application shell and non-destructive confirmations in `apps/desktop/src/App.tsx` and `apps/desktop/src/styles.css`

**Checkpoint**: Desktop UX remains responsive and all report/UI contract tests pass.

---

## Phase 9: User Story 7 - CLI and Headless Container (Priority: P3)

**Goal**: Complete the CLI and optional non-root, read-only-by-default container using the same engine.

**Independent Test**: Desktop/backend and CLI produce identical classifications; container scan cannot
write a read-only source and quarantine fails closed without an explicit writable mount.

### Tests for User Story 7

- [X] T102 [P] [US7] Write CLI JSON/human output and exit-code contract tests in `apps/cli/tests/cli_contract.rs`
- [X] T103 [P] [US7] Write CLI/core conformance fixtures in `tests/conformance/cli_core.rs`
- [X] T104 [P] [US7] Write container read-only source and non-root smoke test in `tests/container/smoke.ps1`

### Implementation for User Story 7

- [X] T105 [US7] Wire all CLI subcommands, confirmations, and exit codes in `apps/cli/src/main.rs` and `apps/cli/src/commands/mod.rs`
- [X] T106 [P] [US7] Create non-root multi-stage CLI image in `Dockerfile`
- [X] T107 [P] [US7] Create explicit scan/quarantine mount entrypoint in `docker/entrypoint.sh`
- [X] T108 [US7] Document Windows Docker limitations and mounts in `docs/docker.md`

**Checkpoint**: CLI/headless modes preserve the same safety invariants as desktop.

---

## Phase 10: User Story 8 - Permanent Delete from Quarantine (Priority: P4, Deferred Gate)

**Goal**: Add permanent deletion only for verified quarantine entries after every earlier phase passes.

**Independent Test**: Source paths, incomplete confirmations, and automatic cleanup default are rejected.
Only individually selected verified quarantine entries can be removed: after retention in normal mode,
or before retention after explicitly enabling immediate mode and completing its distinct challenge.

**Hard Gate**: T001–T108 and all safety/recovery tests MUST be complete before T109 begins.

- [X] T109 [US8] Record prerequisite safety-suite evidence and explicit deletion enablement decision in `docs/permanent-delete-gate.md`
- [X] T110 [P] [US8] Write source-path rejection, retention, exact typed phrase, count/bytes, and no-auto-delete tests in `crates/dedupe-store/tests/permanent_delete.rs`
- [X] T111 [P] [US8] Write interruption and audit durability tests for quarantine-only deletion in `crates/dedupe-store/tests/permanent_delete.rs`
- [X] T112 [US8] Implement quarantine-entry-only permanent deletion with multi-step token in `crates/dedupe-core/src/permanent_delete.rs`
- [X] T113 [US8] Expose deletion only in quarantine CLI/desktop adapters in `apps/cli/src/main.rs` and `apps/desktop/src-tauri/src/commands.rs`
- [X] T114 [US8] Implement high-friction quarantine delete dialog in `apps/desktop/src/components/PermanentDeleteDialog.tsx`

**Checkpoint**: Permanent deletion is unreachable from all source-file and default workflows.

---

## Phase 11: Benchmarks, Documentation, and Windows Packaging

- [X] T115 [P] Implement benchmark fixture generator for mandated file populations in `benchmarks/src/generate.rs`
- [X] T116 [P] Implement timed scan/hash/memory benchmark runner in `benchmarks/src/main.rs`
- [ ] T117 Run reference-device benchmark and record reproducible results in `docs/benchmark-report.md`
- [X] T118 [P] Write architecture, algorithms, and data-loss risk documentation in `docs/architecture.md` and `docs/algorithm.md`
- [X] T119 [P] Write build, development, and Windows packaging guides in `docs/build-windows.md`
- [X] T120 [P] Write user guide and project/filter/scan/results/quarantine workflows in `docs/user-guide.md`
- [X] T121 [P] Write manual recovery and database/manifest backup guide in `docs/manual-recovery.md`
- [X] T122 [P] Write complete test and fault-injection evidence guide in `docs/testing.md`
- [X] T123 Validate `specs/001-safe-duplicate-removal/quickstart.md` end to end on Windows
- [X] T124 Build and smoke-test NSIS installer; record artifact and checksum in `artifacts/windows/README.md`
- [X] T125 Run final offline/privacy check, cargo audit/deny, frontend audit, and full acceptance suite in `docs/release-evidence.md`

---

## Phase 12: Explicit Immediate Delete Follow-up

- [X] T126 [US8] Add a durable immediate deletion mode that bypasses only the retention-time gate in core, store, CLI, and desktop adapters
- [X] T127 [US8] Add explicit desktop opt-in, a distinct exact challenge, migration v4, and retained-entry isolation tests for immediate deletion
- [X] T128 [US8] Add explicit bulk selection for eligible filtered quarantine entries with clear-all and challenge-bound deletion
- [X] T129 [US8] Replace manual token/phrase transcription with one explicit final checkbox while preserving the bound backend challenge

---

## Phase 13: User Story 9 - Verified Online Windows Installer (Priority: P1 Release)

**Goal**: Ship one compact setup EXE with WebView-independent runtime preflight, real-byte aggregate
progress, resumable/content-verified downloads, and auditable full app-local uninstall cleanup.

**Independent Test**: Controlled HTTP fixtures and clean Windows VMs prove correct byte counters,
resume/retry/integrity/concurrency, successful install with/without WebView2, update preservation, and
explicit uninstall cleanup without touching quarantine.

### Tests and evidence for User Story 9

- [x] T130 [P] [US9] Write strict manifest parsing, HTTPS, size, SHA-256, architecture, and duplicate-ID tests in `apps/runtime-installer/tests/manifest.rs`
- [x] T131 [P] [US9] Write byte-weighted percent, rolling speed, ETA, verified-cache, and no-timer progress tests in `apps/runtime-installer/tests/progress.rs`
- [x] T132 [P] [US9] Write local HTTP tests for fresh transfer, 206 resume, ignored Range/200 restart, truncation, redirect, retry, cancellation, and cache reuse in `apps/runtime-installer/tests/download_resume.rs`
- [x] T133 [P] [US9] Write two-item overlap, concurrency-at-most-two, and bounded-buffer tests in `apps/runtime-installer/tests/concurrency.rs`
- [x] T134 [P] [US9] Write installed/cached/corrupt preflight and runtime-install-exit tests in `apps/runtime-installer/tests/preflight.rs`
- [x] T135 [P] [US9] Write NSIS hook static tests for app/uninstall shortcuts, explicit-uninstall cleanup, update preservation, and quarantine exclusion in `apps/runtime-installer/tests/nsis_contract.rs`
- [x] T136 [US9] Record release executable import audit proving WebView2-only non-system runtime in `docs/build-windows.md`

### Implementation for User Story 9

- [x] T137 [P] [US9] Add the native helper package and size-optimized static-CRT release build in `apps/runtime-installer/Cargo.toml`, workspace release profile, and `installer/windows/build-online-installer.ps1`
- [x] T138 [P] [US9] Implement versioned embedded manifest parsing and validation in `apps/runtime-installer/src/manifest.rs` and `installer/runtime-manifest.json`
- [x] T139 [P] [US9] Implement WebView2 registry preflight and completed-cache verification in `apps/runtime-installer/src/preflight.rs`
- [x] T140 [US9] Implement WinHTTP streaming, `.part`/Range resume, retry, exact length, SHA-256, atomic promotion, and cancellation in `apps/runtime-installer/src/download.rs`
- [x] T141 [US9] Implement bounded two-worker scheduling without whole-file allocation in `apps/runtime-installer/src/scheduler.rs`
- [x] T142 [P] [US9] Implement atomic counters, rolling throughput, ETA, per-item state, and aggregate snapshots in `apps/runtime-installer/src/progress.rs`
- [x] T143 [US9] Implement native Win32 setup UI for actual bytes, total size, speed, ETA, overall progress, current file, and all item states in `apps/runtime-installer/src/ui.rs`
- [x] T144 [US9] Implement verified runtime execution, exit handling, NSIS-hook integration, and retry without redownload in `apps/runtime-installer/src/install.rs` and `apps/runtime-installer/src/main.rs`
- [x] T145 [P] [US9] Configure the public Tauri NSIS setup to skip its opaque WebView2 downloader, embed the native helper, and use Vietnamese current-user settings in `apps/desktop/src-tauri/tauri.conf.json`
- [x] T146 [US9] Create app and uninstaller Start-menu shortcuts plus fixed-root explicit-uninstall cleanup/update preservation in `apps/desktop/src-tauri/windows/hooks.nsh`
- [x] T147 [P] [US9] Add reproducible two-stage helper-plus-NSIS build, resource embedding, checksum, and verification scripts in `installer/windows/build-online-installer.ps1` and `installer/windows/verify-installer.ps1`
- [x] T148 [P] [US9] Add installer cache/log paths and uninstall boundary documentation in `docs/build-windows.md`, `docs/user-guide.md`, and `README.md`

### Qualification for User Story 9

- [x] T149 [US9] Run format, clippy, workspace tests, frontend tests/build, and runtime-installer fixture suite
- [x] T150 [US9] Build the final online setup EXE and prove it is below 15 MiB with SHA-256 in `artifacts/windows/README.md`
- [ ] T151 [US9] Smoke-test no-WebView2 and already-valid-WebView2 clean-machine installation and launch in `docs/release-evidence.md`
- [ ] T152 [US9] Smoke-test interruption/resume, retry, corrupt cache, wrong SHA-256, and no-redownload behavior in `docs/release-evidence.md`
- [ ] T153 [US9] Smoke-test upgrade data preservation and explicit uninstall fixed-root deletion while quarantine hashes remain unchanged in `docs/release-evidence.md`

---

## Dependencies and Execution Order

```text
Setup -> Foundational
Foundational -> US1
US1 -> US2
US1 + Foundational -> US3
US2 + US3 -> US4
US4 -> US5
US1..US5 -> US6
US1..US6 -> US7
US1..US7 + all safety gates -> US8
US1..US8 -> US9 online installer and uninstall qualification
All selected stories -> Benchmarks / Packaging / Release Evidence
```

- Within every story, tests precede implementation and must be observed failing first.
- Tasks marked `[P]` touch different files and may run in parallel after their phase dependencies pass.
- Database schema/repository changes and mutation state-machine changes run sequentially.
- A failed sequential safety task halts dependent work; isolated test fixtures may continue for diagnosis.

## Parallel Examples

- **US1**: T028–T032 can be authored in parallel; T034–T039 can then implement separate modules.
- **US2**: T043–T049 are independent test files; T051–T053 and T056 are independent implementations.
- **US4**: T067–T073 cover separate fault classes; T074–T077 touch separate journal/layout/platform files.
- **US6**: report formats, Tauri tests, frontend stores, and page components can proceed independently.
- **Docs**: T118–T122 can proceed in parallel after behavior stabilizes.

## Implementation Strategy

1. Ship a demonstrable read-only MVP at the end of US1.
2. Add duplicate proof and dry-run; stop and validate every safety fixture.
3. Complete resumability before enabling any mutation.
4. Enable quarantine only after the full crash matrix passes; immediately add restore and recovery.
5. Add desktop/CLI adapters without duplicating engine logic.
6. Do not expose permanent deletion until its hard gate is signed with evidence; after enablement,
   expose it only through explicit quarantine workflows. Retained entries require a separate immediate
   mode opt-in and mode-specific exact challenge.
7. Ship the online installer only after runtime dependency audit, real-byte fixture tests, mandatory
   SHA-256, clean-machine installation, retry/resume, update, and quarantine-preserving uninstall pass.

## Format Validation

All 153 tasks use markdown checkboxes, sequential IDs, story labels in story phases, and exact file
paths. `[P]` appears only where tasks can change separate files after dependencies are satisfied.
