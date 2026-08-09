<!--
Sync Impact Report
- Version change: 1.0.0 -> 1.1.0
- Added principles:
  - I. Data Preservation Is Non-Negotiable
  - II. Evidence Before Mutation
  - III. Recoverable Transactions
  - IV. Test-First Safety Engineering
  - V. Bounded, Local, Observable Processing
- Added sections:
  - Mandatory Technical Constraints
  - Development Workflow and Quality Gates
- Clarified principle V and Windows constraints:
  - Normal application operation remains fully local.
  - A WebView-independent native helper embedded in the Windows setup may download only pinned,
    integrity-verified Microsoft runtime artifacts and may never transmit user/application data.
  - Explicit uninstall cleanup is fixed-root and must preserve per-volume quarantine documents.
- Removed sections: none; all template placeholders were resolved.
- Templates:
  - ✅ .specify/templates/plan-template.md
  - ✅ .specify/templates/spec-template.md
  - ✅ .specify/templates/tasks-template.md
  - ✅ .specify/templates/checklist-template.md (compatible; no change required)
- Agent skills reviewed: all installed speckit-* skill definitions are compatible.
- Deferred items: none.
-->
# Safe Duplicate Finder Constitution

## Core Principles

### I. Data Preservation Is Non-Negotiable
Every default workflow MUST preserve every source byte. First use MUST be scan-only; mutation
MUST require an explicit dry-run review followed by explicit user confirmation. Source files MUST
NOT be permanently deleted in the normal workflow. Quarantine MUST preserve at least one verified
copy in every duplicate group, and permanent deletion MUST be restricted to already-quarantined
files behind a separate, multi-step confirmation. When safety and speed conflict, the implementation
MUST choose safety. Rationale: a false positive or interrupted mutation can destroy irreplaceable
personal documents, while a false negative only leaves recoverable extra data.

### II. Evidence Before Mutation
A file MUST be classified as a strict duplicate only when normalized name, exact 64-bit byte size,
full BLAKE3, and full SHA-256 all match and both files remain stable. Content-only matching MUST be
opt-in and visibly warned. Quick hashes MAY reject candidates but MUST NOT confirm duplicates or
authorize mutations. Immediately before quarantine, the system MUST re-open the source and verify
file identity, volume identity, size, modification time, and stability; changed or uncertain files
MUST be marked unstable and skipped. Hard links MUST be identified by physical file identity and
MUST NOT be reported as independently reclaimable copies. Rationale: every mutation decision must
be supported by reproducible, durable evidence rather than a probabilistic shortcut.

### III. Recoverable Transactions
Every quarantine and restore operation MUST use a durable, stateful transaction journal with the
ordered states `planned`, `moving`, and `verified`. Database state MUST be flushed before filesystem
mutation, destination bytes MUST be verified before completion, and interrupted transactions MUST
be reconciled on next startup without assuming success. Quarantine MUST prefer a unique destination
on the same volume, preserve the original relative path, never overwrite, and support restoration to
the exact original location or an explicit conflict-safe alternative. Audit and transaction histories
MUST be append-only. Rationale: power loss, antivirus interference, disconnection, and process crashes
are ordinary operating conditions for a 1 TB scan, not exceptional excuses for data loss.

### IV. Test-First Safety Engineering
Tests for a safety-relevant behavior MUST be written and observed failing before its implementation.
Unit, integration, property-based, transaction, restore, crash-recovery, Unicode, long-path, hard-link,
symlink, cancellation, permission, and fault-injection suites are mandatory. The seven destructive
failure boundaries—before move, during move, after move before database update, after database update
before verification, storage disconnection, log failure, and commit failure—MUST have deterministic
tests. No quarantine, restore, recovery, or permanent-deletion phase may begin until its prerequisite
safety tests pass. Rationale: important safety properties must be executable invariants, not prose-only
intent.

### V. Bounded, Local, Observable Processing
Core file processing MUST live in an independently testable Rust library reused by the desktop app,
CLI, and optional headless build. Reads MUST be streaming; file sizes and byte counters MUST use
64-bit-safe types; concurrency MUST be bounded with backpressure; and metadata/progress MUST be
persisted in SQLite WAL transactions in batches. Default memory use MUST remain below 2 GiB under the
defined benchmark workloads. Normal product operation MUST remain local, send no telemetry or file
metadata, names, contents, or hashes externally, and require no administrator rights by default. The
separately built installer may use only the narrowly defined pinned-runtime exception below. Structured JSONL
logs and readable text logs MUST expose errors and state transitions without document contents.
Rationale: predictable resource use, privacy, and inspectable behavior are required for trustworthy
operation on large personal archives.

## Mandatory Technical Constraints

- Backend: stable Rust library workspace with separate adapters for Tauri and CLI.
- Desktop: Tauri with a TypeScript frontend; the UI MUST remain responsive during scanning.
- State: versioned SQLite migrations, WAL mode, foreign keys, transactions, busy handling, and startup
  integrity/recovery checks. The database MUST live outside all scan and quarantine roots.
- Hashing: streaming BLAKE3 followed by streaming SHA-256 only for surviving candidates.
- Windows: Windows 10/11, long paths, files larger than 4 GiB, Unicode, file/volume identity, locked
  files, cloud placeholders, sparse files, Recycle Bin option, and same-volume quarantine semantics.
- Traversal: symlinks and junctions MUST NOT be followed by default; overlap and cycle detection are
  mandatory; quarantine roots MUST always be excluded.
- GPU: MUST NOT be required. It may be introduced only after a reproducible benchmark proves a net
  benefit without weakening correctness, portability, or failure handling.
- Docker: optional CLI/headless packaging only, non-root, read-only source mounts for scanning, and no
  direct deletion of source files.
- The main scan, hash, duplicate, quarantine, restore, journal, recovery, database, logging, pause,
resume, and cancel flows MUST contain no stubs, mocks, empty functions, or TODO implementations.
- Installer network exception: a WebView-independent native helper embedded in Windows setup MAY contact only
  release-pinned HTTPS hosts to download runtime artifacts whose exact size and SHA-256 are embedded
  in the release manifest. It MUST preflight installed runtimes, reuse/resume verified cache, stream
  with bounded memory/concurrency, expose actual received-byte progress, and MUST NOT transmit paths,
  filenames, document contents/digests, projects, logs, telemetry, or any other application/user data.
- Explicit uninstall MUST remove only fixed product install/app-data/cache/registry/shortcut roots.
  Upgrade-mode uninstall MUST preserve app-local state, and every per-volume quarantine/source/export
  location MUST remain outside recursive cleanup.

## Development Workflow and Quality Gates

Work MUST follow this order: architecture and data-loss threat model; schema and state machines;
read-only scanner; duplicate detection; dry-run and reporting; quarantine; restore; crash recovery;
CLI; Tauri UI; Docker; tests and benchmarks; Windows packaging. Permanent deletion MUST remain out of
scope until all earlier safety gates pass.

Each specification MUST map requirements to acceptance scenarios and measurable outcomes. Each plan
MUST include a data-loss safety case, transaction failure matrix, recovery design, resource budget,
platform boundary decisions, and pre/post-design constitution checks. Each task list MUST include
tests before implementation and exact paths, with explicit traceability to requirements or user
stories. A phase is complete only when formatting, linting, unit tests, integration tests, safety
properties, recovery tests, and relevant benchmarks pass. Any failure in a sequential safety task
halts dependent work; warnings from an individual unreadable file may be isolated and reported.

## Governance

This constitution supersedes local convenience, performance optimizations, and conflicting project
documentation. Amendments require a written rationale, a migration/compatibility impact assessment,
updates to dependent templates and specifications, and semantic versioning: MAJOR for incompatible
principle removal or redefinition, MINOR for new principles or materially expanded obligations, and
PATCH for non-semantic clarification. Every plan, pull request, and release review MUST document
compliance with each principle. An exception requires an explicit, time-bounded waiver that names the
affected requirement, evidence, risk owner, safe fallback, and removal date; Principle I may not be
waived. Constitution compliance MUST be re-audited after design and before any filesystem mutation is
enabled in a release build.

**Version**: 1.1.0 | **Ratified**: 2026-07-21 | **Last Amended**: 2026-08-09
