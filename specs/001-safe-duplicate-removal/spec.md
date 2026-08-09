# Feature Specification: Safe Duplicate File Management

**Feature Branch**: `001-safe-duplicate-removal`

**Created**: 2026-07-21

**Status**: Implemented development baseline (0.1.9); Windows online installer optimization in progress  

**Input**: A local Windows desktop application that safely discovers exact duplicate files across
multiple folders and volumes, supports evidence-based review, and uses recoverable quarantine and
restore workflows without deleting source data by default.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Configure and Run a Read-Only Scan (Priority: P1)

A Windows user creates a named project, adds all desired source folders before scanning, defines
include/exclude rules, and starts a read-only scan only when ready. Closing and reopening the app
preserves the project and source list.

**Why this priority**: Safe, deliberate discovery is the minimum useful product and makes no source
changes.

**Independent Test**: Create a project with overlapping folders on multiple volumes, restart the
application, run a scan, and verify every eligible physical file is inventoried once with no source
metadata or content changed.

**Acceptance Scenarios**:

1. **Given** a new project, **When** the user adds several folders, **Then** no scan begins until the
   user selects `Bắt đầu quét`.
2. **Given** duplicate, parent/child, missing, or quarantine folder selections, **When** the source
   list is validated, **Then** overlaps and invalid selections are reported and no file is scanned
   twice.
3. **Given** a saved project, **When** the application is restarted, **Then** folders, priority roots,
   filters, and the last scan summary are restored.
4. **Given** an unreadable, locked, disappearing, changing, cloud-placeholder, sparse, long-path, or
   Unicode file, **When** it is encountered, **Then** it is skipped or classified safely, the error is
   recorded, and the remaining scan continues.

---

### User Story 2 - Review Proven Duplicate Groups and Dry Run (Priority: P1)

The user reviews groups that are confirmed using full-content evidence, sees why one file is kept,
and runs a dry run that predicts quarantine actions and reclaimable bytes without changing data.

**Why this priority**: No mutation is acceptable until the user can inspect deterministic evidence
and the exact proposed actions.

**Independent Test**: Scan fixtures containing same-name/same-size files that differ by one byte,
identical files with different names, hard links, and files that change during hashing; verify only
stable, fully proven duplicates enter the appropriate result groups and dry run changes nothing.

**Acceptance Scenarios**:

1. **Given** strict mode, **When** files share normalized names and exact sizes, **Then** they are not
   duplicates until two independent full-content digests match and stability is proven.
2. **Given** content-only mode is disabled, **When** byte-identical files have different normalized
   names, **Then** they are not placed in a strict duplicate group.
3. **Given** the user enables content-only mode after acknowledging its warning, **When** byte-identical
   files have different names, **Then** they may be grouped while remaining subject to all other proof.
4. **Given** multiple copies, **When** keep rules are evaluated, **Then** at least one file remains kept;
   conflicts require manual resolution.
5. **Given** a proposed result set, **When** dry run executes, **Then** keep/quarantine choices, reasons,
   warnings, and estimated reclaimable bytes are shown with zero filesystem mutations.

---

### User Story 3 - Pause, Resume, Cancel, and Recover a Scan (Priority: P1)

The user can pause, continue, or cancel a large scan without corrupting state and can resume an
interrupted session after reopening the application.

**Why this priority**: A near-1 TB scan may take hours and must coexist with normal desktop use and
unplanned shutdowns.

**Independent Test**: Pause, cancel, terminate, and restart scans at metadata, sample-read, and
full-read stages; verify persisted progress is consistent, completed work is not repeated
unnecessarily, and no mutation occurs.

**Acceptance Scenarios**:

1. **Given** an active scan, **When** pause or cancel is requested, **Then** workers stop at safe work
   boundaries and committed state remains valid.
2. **Given** a paused or interrupted scan, **When** it is resumed, **Then** verified completed stages
   are reused only while the file identity and stability evidence remains valid.
3. **Given** one file fails, **When** other work remains, **Then** the overall scan continues and live
   error counters update without freezing the interface.

---

### User Story 4 - Quarantine with Verified Transactions (Priority: P2)

After reviewing and confirming a dry run, the user moves selected duplicate copies into recoverable,
per-volume quarantine while the product journals and verifies every action.

**Why this priority**: Quarantine provides reclaimable organization without making deletion the first
mutation.

**Independent Test**: Quarantine a duplicate group while injecting interruption at every transaction
boundary; after restart, verify the system identifies the actual source/destination state, never
overwrites, never loses the last copy, and reports only verified moves as completed.

**Acceptance Scenarios**:

1. **Given** a confirmed duplicate and approved action, **When** pre-move identity, size, time, or
   digest evidence differs, **Then** the file is marked unstable and not moved.
2. **Given** a stable approved file, **When** quarantine succeeds, **Then** original and quarantine
   paths, identity, size, evidence, timestamps, and state transitions are durably recorded.
3. **Given** the destination exists or cannot be safely created, **When** quarantine is attempted,
   **Then** no file is overwritten and the transaction remains recoverable with a clear error.
4. **Given** an interrupted transaction, **When** the app reopens, **Then** it reconciles whether the
   file is at source, destination, or both and never assumes the move completed.
5. **Given** a moved file, **When** destination verification has not passed, **Then** the transaction
   is not complete and its bytes are not counted as reclaimed.

---

### User Story 5 - Restore Quarantined Files (Priority: P2)

The user restores a file, duplicate group, or entire quarantine session to its recorded original
location with conflict-safe handling and full verification.

**Why this priority**: Quarantine is only safe when reversal is reliable and independently auditable.

**Independent Test**: Restore verified quarantine entries with free, occupied, missing-volume, and
permission-denied original destinations; verify correct content and journal state without overwrite.

**Acceptance Scenarios**:

1. **Given** an available original path, **When** restore completes, **Then** the restored file matches
   recorded identity-compatible content evidence and the history records verification.
2. **Given** an occupied original path, **When** restore is requested, **Then** the existing file is not
   overwritten and the user chooses a safe alternative or cancels.
3. **Given** interruption during restore, **When** the app restarts, **Then** recovery reconciles both
   locations and preserves all extant copies until the user resolves ambiguity.

---

### User Story 6 - Inspect Logs, Reports, and Safety Evidence (Priority: P2)

The user inspects live progress, errors, duplicate evidence, quarantine/restore history, and exports
CSV, JSON, or HTML reports without exposing document contents.

**Why this priority**: Trust requires visible evidence, actionable errors, and a durable audit trail.

**Independent Test**: Run a scan containing duplicates, links, changing files, and errors; compare live
statistics, readable logs, append-only audit records, and all export formats for consistent counts.

**Acceptance Scenarios**:

1. **Given** an active scan, **When** progress changes, **Then** the interface shows discovered and
   processed files, bytes read, current file, read rate, groups, potential savings, errors, skips,
   unstable files, elapsed time, and clearly labeled estimated remaining time.
2. **Given** a completed scan, **When** a report is exported, **Then** it includes group membership,
   keep/quarantine choices and reasons, size, content evidence, verification state, warnings, links,
   unreadable files, and potential savings.
3. **Given** log or report generation, **When** records are written, **Then** document contents are never
   included and audit history cannot be silently edited.

---

### User Story 7 - Use the Same Safety Engine from CLI and Headless Mode (Priority: P3)

An advanced user or operator runs scan, dry-run, report, quarantine, restore, and recovery inspection
through a command line using the same rules as the desktop application; an optional containerized
scan supports explicitly mounted sources.

**Why this priority**: Reuse makes the safety rules testable and supports automation without creating
a second, inconsistent file engine.

**Independent Test**: Execute equivalent desktop and command-line operations against identical
fixtures and verify matching classifications and safety decisions; run a read-only container scan as
a non-privileged user.

**Acceptance Scenarios**:

1. **Given** identical project inputs, **When** desktop and CLI scans run, **Then** they produce the same
   duplicate groups and safety statuses.
2. **Given** a container with read-only source mounts, **When** scan or dry run executes, **Then** no
   source mutation is possible.
3. **Given** quarantine is requested in headless mode, **When** required writable mounts and explicit
   confirmation are absent, **Then** the request fails closed.

---

### User Story 8 - Permanently Delete from Quarantine (Priority: P4, Deferred Safety Gate)

Only after scan, dry-run, quarantine, restore, and crash-recovery gates pass, the user may permanently
delete selected files that are already in quarantine through a separate, high-friction workflow.

**Why this priority**: Permanent deletion is intentionally last and never part of the default path.

**Independent Test**: Attempt permanent deletion from source paths, without confirmation, before the
retention date in normal mode, and before the retention date after explicitly enabling immediate
mode; verify that only a fully confirmed quarantine-only request whose UI checkbox submits the
mode-specific backend challenge is accepted.

**Acceptance Scenarios**:

1. **Given** a source file outside quarantine, **When** permanent deletion is requested, **Then** the
   request is rejected.
2. **Given** quarantine files, **When** the final explicit confirmation checkbox is not selected,
   **Then** nothing is deleted.
3. **Given** automatic retention cleanup is disabled, **When** retention time passes, **Then** nothing
   is deleted automatically.
4. **Given** a verified retained quarantine entry, **When** the user explicitly enables immediate
   mode, explicitly selects it individually or through bulk selection, and completes its separate exact challenge, **Then** only that
   selected quarantine object is deleted and the 30-day wait is bypassed.

---

### User Story 9 - Install, Resume, and Completely Uninstall on Windows (Priority: P1 Release)

A Windows 10/11 user downloads one compact setup EXE. The setup detects valid installed runtimes,
downloads only missing runtime artifacts with resumable verified transfers, displays real aggregate
progress, and installs the application. The Start menu contains explicit application and uninstall
shortcuts. A normal uninstall removes the installed program and all app-local user state while
preserving real quarantined documents outside those app-local directories.

**Why this priority**: A safe application is not releasable if setup appears frozen, repeats a
200-MiB download, executes unverified bytes, or leaves hidden local databases/log/cache after the
user explicitly uninstalls it.

**Independent Test**: On clean Windows 10/11 VMs, exercise no-runtime, runtime-already-valid,
interrupted-download, HTTP-range-ignored, wrong-length, wrong-SHA, runtime-install-failure,
application-install-failure, retry, upgrade, and uninstall cases. Compare the UI counters with bytes
observed by a controlled HTTP fixture and verify the post-uninstall path allowlist.

**Acceptance Scenarios**:

1. **Given** WebView2 is already valid, **When** setup starts, **Then** it skips the runtime download
   and proceeds without contacting the runtime artifact URL.
2. **Given** a partial runtime file and a server that supports byte ranges, **When** setup restarts,
   **Then** only the missing suffix is received, the displayed byte totals match the actual transfer,
   and the complete file is promoted only after size and SHA-256 match.
3. **Given** two or more independent missing artifacts in a future manifest, **When** setup downloads
   them, **Then** at most two run concurrently; one failure does not discard another verified result.
4. **Given** active download traffic, **When** bytes arrive, **Then** the UI shows total downloaded,
   total required size, rolling speed, ETA, overall percentage, current file, and each item state from
   actual received-byte counters rather than a timer.
5. **Given** a cached completed file, **When** its length or SHA-256 is invalid, **Then** it is not
   executed or counted complete and the user receives an integrity error.
6. **Given** installation succeeds, **When** the Start menu is inspected, **Then** it contains one
   application shortcut and one clearly named uninstall shortcut.
7. **Given** the user runs uninstall (not update), **When** it completes, **Then** binaries,
   application database, WAL/SHM, logs, settings, WebView profile, reports/cache stored in the
   application data directories, installer cache, registry entries, and shortcuts are removed.
8. **Given** quarantined documents exist on selected source volumes, **When** uninstall completes,
   **Then** those documents and their sidecar recovery manifest remain untouched and the uninstaller
   states that boundary before removal.

### Edge Cases

- Source file is locked, unreadable, renamed, removed, replaced, truncated, extended, or written while
  being inventoried or hashed.
- A volume disconnects, becomes read-only, changes mount letter, lacks quarantine/database space, or
  reports an I/O error during scan, move, verification, restore, or commit.
- Antivirus, indexing, or cloud synchronization temporarily locks or hydrates a file.
- Sparse files, cloud placeholders, network files, files larger than 4 GiB, long paths, reserved names,
  invalid destination names, Unicode normalization variants, and case-only name differences occur.
- Hard links, symlinks, junctions, directory cycles, repeated roots, and parent/child roots occur.
- Database is busy, full, corrupted, or left with an incomplete transaction after process termination
  or power loss.
- Logging or report output fails while the primary operation is otherwise able to continue.
- Source and destination both exist after an interrupted move or restore and differ in evidence.
- The last kept copy is deselected, becomes unstable, or disappears before a group action starts.
- The same project is run repeatedly with unchanged and partially changed files.
- Runtime download is interrupted, resumed through a proxy, redirected, returns chunked data, ignores
  `Range`, changes `ETag`, reports the wrong length, is tampered, or has no usable network connection.
- Setup is rerun with a valid cached Runtime, a corrupted completed cache file, a complete `.part`
  file awaiting verification, or a runtime that another process installs during preflight.
- Uninstall is invoked during upgrade versus as an explicit user removal, and app-local data contains
  locked WebView/log/database files.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST create, rename, open, persist, and remove project records without deleting
  or modifying source files.
- **FR-002**: A project MUST persist multiple source folders, priority roots, filters, last-run state,
  and summaries across application restarts.
- **FR-003**: Users MUST be able to add and remove all source folders before explicitly starting a scan;
  adding a folder MUST NOT start scanning.
- **FR-004**: The system MUST detect duplicate roots, parent/child overlap, nonexistent roots, and roots
  that contain project state or quarantine data.
- **FR-005**: Each eligible physical file MUST be enumerated at most once per scan despite overlapping
  selections.
- **FR-006**: Filters MUST support document-only defaults, all types, include/exclude extensions,
  excluded folders, file globs, minimum size, hidden files, system files, and quarantine exclusion.
- **FR-007**: Symlinks and junctions MUST not be followed by default; opt-in traversal MUST warn about
  cycles and MUST still detect them.
- **FR-008**: Metadata inventory MUST record full and normalized paths, normalized name, extension,
  exact byte size, creation/modification times, physical file and volume identities, link type,
  accessibility, lock state, and stability state when the platform exposes them.
- **FR-009**: The system MUST distinguish hard-link aliases from independently stored copies and MUST
  report them separately without counting their bytes as independently reclaimable.
- **FR-010**: Strict mode MUST be the default and MUST require normalized-name equality, exact byte-size
  equality, two independent full-content digest matches, stability, and distinct physical identities.
- **FR-011**: Content-only mode MUST be disabled by default and MUST require an acknowledged warning
  before files with different normalized names can be grouped.
- **FR-012**: Candidate reduction MAY use metadata and sampled content, but sampled evidence MUST only
  reject candidates and MUST never confirm a duplicate or authorize mutation.
- **FR-013**: Unique-size or otherwise unique candidate files MUST not require full-content reading.
- **FR-014**: Full-content evidence MUST be computed using streaming reads without loading whole files
  into memory.
- **FR-015**: The system MUST compare identity, size, and modification evidence before and after a
  content read; changed or uncertain files MUST be classified unstable.
- **FR-016**: Unstable files MUST NOT be selected for quarantine or permanent deletion.
- **FR-017**: Duplicate groups MUST expose their proof and the reason each member is kept, proposed for
  quarantine, excluded, unstable, or a hard-link alias.
- **FR-018**: Keep selection MUST support preferred roots, oldest, newest, shortest path, preferred
  volume, and manual choice.
- **FR-019**: Default keep selection MUST prefer a marked primary root, then oldest modification time,
  then shortest path, and MUST request manual choice when the result remains uncertain.
- **FR-020**: Every duplicate group MUST retain at least one stable, verified file before any mutation.
- **FR-021**: First use and the default project action MUST be read-only scan.
- **FR-022**: Dry run MUST show exact proposed keep and quarantine actions, reasons, warnings, and
  potential reclaimed bytes without filesystem mutation.
- **FR-023**: The results view MUST support expanding groups, full paths, file/folder opening, file
  preview, evidence display, per-file and per-group selection, and MUST NOT offer select-all permanent
  deletion.
- **FR-024**: Immediately before quarantine, the system MUST re-open and revalidate source identity,
  volume, size, modification time, stability, and evidence; uncertainty MUST fail closed.
- **FR-025**: Quarantine MUST use a separate location on each source volume when available, preserve
  original relative structure, allocate a unique entry identifier, and never overwrite.
- **FR-026**: Every move MUST journal transaction ID, paths, identities, size, both full-content evidence
  values, timestamps, pre/post states, verification result, and error.
- **FR-027**: A move MUST durably record planned intent before source revalidation and filesystem
  mutation.
- **FR-028**: A moved file MUST be verified at its destination before the transaction is complete or
  bytes are counted as reclaimed.
- **FR-029**: Startup recovery MUST reconcile incomplete moves from actual source and destination state
  and MUST NOT assume an incomplete journal entry succeeded.
- **FR-030**: Quarantine MUST support verifying one or more entries again after the move.
- **FR-031**: The system MUST restore one file, one duplicate group, or an entire session to recorded
  original paths without overwriting an existing file.
- **FR-032**: Restore MUST be journaled, interruption-recoverable, and verified before completion.
- **FR-033**: The quarantine view MUST support search and filtering by date and project.
- **FR-034**: Recycle Bin integration MAY be offered as an explicit operation separate from quarantine.
- **FR-035**: Permanent deletion MUST reject files outside quarantine, require explicit selection
  and a final confirmation checkbox, and show count and bytes. The UI MUST submit the short-lived
  mode-specific token and exact phrase to the backend without requiring manual transcription. Normal
  mode MUST honor the retention policy. A separate explicit immediate mode MAY bypass only the
  retention-time gate; it MUST retain every other identity, digest, token, audit, and handle-bound
  deletion check. Automatic permanent deletion MUST remain disabled.
- **FR-036**: Pause, resume, and cancel MUST take effect at safe work boundaries and leave committed
  project state valid.
- **FR-037**: Interrupted scan state MUST be resumable after restart, while stale evidence MUST be
  invalidated when file identity or stability changes.
- **FR-038**: A single-file failure MUST be recorded and exposed without aborting unrelated work.
- **FR-039**: Live progress MUST expose folder/file counts, processed work, bytes read, current file,
  read rate, duplicate groups, potential savings, errors, skips, unstable files, elapsed time, and an
  explicitly approximate remaining-time estimate.
- **FR-040**: The user MUST be able to configure worker limits, read-rate limits, reduced CPU mode, and
  background operation.
- **FR-041**: Work scheduling MUST bound parallel work and pending queues so that large scans do not
  create unbounded memory or one worker per file.
- **FR-042**: The system MUST support long Windows paths, Unicode paths, files larger than 4 GiB,
  locked files, network locations, sparse files, and cloud placeholders without crashing.
- **FR-043**: Project and operation state MUST be durable, versioned, transactional, recoverable after
  abrupt shutdown, and stored outside all source/quarantine roots.
- **FR-044**: Incomplete transactions MUST be discoverable and repairable without treating ambiguity
  as success.
- **FR-045**: Application, scan, error, audit, transaction, and restore logs MUST be available in both
  structured and human-readable forms appropriate to their audience.
- **FR-046**: Audit and transaction history MUST be append-only and MUST not contain document content.
- **FR-047**: Reports MUST export CSV, JSON, and HTML with duplicate groups, keep/quarantine decisions,
  reasons, size, evidence, verification, potential savings, errors, warnings, unstable files, links,
  and unreadable files.
- **FR-048**: Desktop, command-line, and optional headless interfaces MUST share the same safety and
  classification behavior.
- **FR-049**: Headless read-only scans MUST support non-privileged execution and read-only source
  access; mutation MUST fail when explicit writable quarantine access is absent.
- **FR-050**: The interface MUST remain responsive during long operations and MUST not start a scan on
  application launch.
- **FR-051**: Operation must remain fully local: no document, path, filename, digest, project data, or
  telemetry may be transmitted externally by default.
- **FR-052**: The application MUST not require elevated administrator privilege for normal operation
  and MUST limit requested access to user-selected folders.
- **FR-053**: A manual recovery guide and exportable transaction manifest MUST allow a user to locate
  and restore files even if the application cannot start.
- **FR-054**: Re-running an unchanged project MUST be idempotent and MUST NOT repeat verified mutations
  or lose data.
- **FR-055**: The product MUST provide reproducible benchmarks for 100,000 small files, 10,000 document
  files, 1–20 GiB files, near-1 TB simulated data, same-volume roots, and multi-volume roots.
- **FR-056**: Windows release MUST provide one compact current-user NSIS setup EXE containing a native
  runtime helper that does not depend on WebView2 and runs before setup reports success.
- **FR-057**: Before any runtime request, setup MUST detect whether the required runtime is already
  installed and valid; a valid result MUST skip both download and installation.
- **FR-058**: Each runtime manifest item MUST contain a stable identifier, display name, pinned HTTPS
  URL, exact unsigned 64-bit length, lowercase/uppercase-insensitive SHA-256, installer arguments,
  detection rule, and architecture.
- **FR-059**: Setup MUST stream downloads with bounded buffers, retain `.part` files, resume with HTTP
  `Range`, accept append only on a valid `206 Partial Content`, and restart only the affected item when
  the server returns a full response.
- **FR-060**: Setup MUST hash the complete downloaded file using streaming SHA-256 and MUST NOT rename
  it complete, execute it, or count it installable until both exact length and SHA-256 match.
- **FR-061**: A valid completed cache artifact MUST be reused after revalidation, including after setup
  cancellation, process termination, runtime installer failure, or application installer failure.
- **FR-062**: The runtime download scheduler MUST support multiple manifest items, preflight them
  independently, and download at most two missing independent items concurrently; it MUST NOT force
  sequential download when safe parallelism is available.
- **FR-063**: Runtime download UI MUST show actual total received bytes, total required download size,
  rolling bytes-per-second, estimated remaining time, overall byte-weighted percentage, current file,
  and every item state. No displayed download progress value MAY be driven by a synthetic timer.
- **FR-064**: Cancellation MUST stop at a bounded read boundary, preserve valid partial/completed cache,
  and leave no runtime executable marked verified unless the mandatory digest passed.
- **FR-065**: The current x86_64 release manifest MUST include only WebView2 Evergreen Standalone x64;
  Visual C++ Redistributable MUST remain excluded while the release executable imports no
  `VCRUNTIME140.dll` or `MSVCP140.dll`.
- **FR-066**: Setup MUST record a local installer log containing artifact ID, non-sensitive URL host,
  expected/received byte counts, resume offset, retries, digest result, installer exit code, and stage;
  it MUST NOT record or transmit user document metadata.
- **FR-067**: Successful installation MUST create a Start-menu application shortcut and a separately
  labeled Start-menu uninstaller shortcut.
- **FR-068**: Explicit uninstall MUST remove installed binaries, installer cache, the application
  database and journals, logs, settings, WebView profile/cache, app-generated reports/cache under the
  application data roots, product registry entries, and application/uninstaller shortcuts.
- **FR-069**: Upgrade/uninstall invoked internally by an installer update MUST preserve app-local data.
  Explicit uninstall cleanup MUST use fixed known app-data roots and MUST NOT recursively delete a
  user-selected source root, export destination, or per-volume quarantine root.
- **FR-070**: The online setup binary, embedded runtime helper, runtime manifest, and installed release
  executable MUST have release-recorded SHA-256 checksums and clean-machine smoke evidence.

### Safety Invariants *(mandatory for filesystem mutation)*

- **SI-001**: No default workflow permanently deletes a source file.
- **SI-002**: No file is classified as a strict duplicate without stable identity, exact size,
  normalized-name match, distinct physical identity, and two matching full-content digests.
- **SI-003**: Quick or sampled evidence can only eliminate candidates, never confirm a duplicate.
- **SI-004**: Every duplicate group retains at least one stable verified copy before and after every
  mutation.
- **SI-005**: A file changed after scan or during evidence collection is never moved.
- **SI-006**: A quarantine or restore transaction is complete only after destination verification is
  durable.
- **SI-007**: Recovery treats incomplete or ambiguous state as unresolved, never as successful.
- **SI-008**: No mutation overwrites an existing destination.
- **SI-009**: Running the same project repeatedly is idempotent with respect to verified actions.

### Key Entities *(include if feature involves data)*

- **Project**: Named collection of source roots, filters, keep policy, resource limits, and scan history.
- **Source Root**: Selected folder with volume identity, priority, validation, and overlap status.
- **Scan Session**: Durable execution record with lifecycle, checkpoints, progress, and summary.
- **File Record**: Observed path, physical identity, metadata snapshots, evidence stages, and stability.
- **Duplicate Group**: Set of proven independent copies with mode, proof, keep decision, and savings.
- **Operation Plan**: Immutable dry-run selection and reasons tied to the evidence snapshot.
- **Transaction**: Journaled quarantine or restore intent and state transitions.
- **Quarantine Entry**: Unique destination, original path, retention, verification, and restore history.
- **Audit Event**: Append-only record of user intent, safety decisions, state changes, and errors.
- **Error Record**: Isolated failure with operation, path-safe context, classification, and retry status.
- **Runtime Manifest**: Versioned immutable list of required runtime artifacts, detection rules,
  expected sizes/digests, installation commands, and architecture.
- **Runtime Transfer**: Per-item cache path, partial length, network bytes for this run, retry count,
  transfer state, digest result, and installer exit status.
- **Installer Session**: Aggregate immutable manifest identity plus total/received bytes, speed window,
  ETA, current stage, result, and local diagnostic log path.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: In the mandated safety fixture, two same-name, same-size files differing by one byte are
  classified as non-duplicates in 100% of runs.
- **SC-002**: A file modified after scan or during a content read is prevented from quarantine in 100%
  of deterministic race tests.
- **SC-003**: Every tested duplicate group retains at least one stable verified copy across dry-run,
  quarantine, restore, cancel, and injected-crash scenarios.
- **SC-004**: Zero source permanent-deletion operations are reachable through first use, scan, dry-run,
  quarantine, restore, or recovery workflows.
- **SC-005**: All defined transaction failure boundaries recover to an explainable state with no lost
  extant copy in 100% of deterministic fault-injection runs.
- **SC-006**: A verified quarantine entry restores to its original path with matching full-content
  evidence in 100% of conflict-free restore tests.
- **SC-007**: A scan of 100,000 fixture records and the near-1 TB benchmark completes with peak process
  memory below 2 GiB.
- **SC-008**: Adding sources, starting, pausing, resuming, canceling, reviewing results, quarantining,
  and restoring remain interactively usable without an unresponsive desktop window lasting more than
  two seconds under the reference workload.
- **SC-009**: After abrupt termination at each defined scan or mutation boundary, the next launch either
  resumes safely or presents an explicit unresolved action without database corruption.
- **SC-010**: CSV, JSON, and HTML reports agree on group count, file count, evidence status, and potential
  savings for the same completed session.
- **SC-011**: Desktop and CLI classification outputs are identical for 100% of shared conformance
  fixtures.
- **SC-012**: No network transmission occurs during normal scanning, review, quarantine, restore,
  reporting, or recovery conformance tests.
- **SC-013**: All mandatory unit, integration, property, transaction, recovery, restore, Unicode,
  long-path, link, cancellation, permission, and fault-injection tests pass before release packaging.
- **SC-014**: For controlled 0-byte, partial, resumed, concurrent, and completed-cache fixtures, every
  displayed received-byte value equals the fixture server's accepted payload bytes plus verified cache
  bytes, and overall percentage differs from exact byte-weighted progress by at most 0.1 percentage
  point.
- **SC-015**: A 50%-complete partial WebView2 artifact resumes without retransmitting the verified
  prefix when the server returns 206; a server returning 200 causes a clean item-only restart with no
  duplicate append.
- **SC-016**: 100% of wrong-length, wrong-SHA-256, truncated, and altered-cache test artifacts are
  rejected before process execution.
- **SC-017**: With two independent fixture artifacts and sufficient bandwidth, both transfers overlap
  in time while peak runtime-helper memory stays below 32 MiB and concurrency never exceeds two.
- **SC-018**: On supported clean Windows 10/11 VMs, setup installs and launches successfully whether
  WebView2 is absent or already valid, and retrying after interruption does not redownload a verified
  completed artifact.
- **SC-019**: After explicit uninstall, the fixed application install/data/cache/registry/shortcut
  allowlist is absent in 100% of qualification runs, while all seeded per-volume quarantine documents
  and recovery manifests remain byte-identical.
- **SC-020**: The final online setup EXE is below 15 MiB, excluding runtime bytes downloaded on demand,
  and its SHA-256 plus helper/runtime-manifest evidence is published in `artifacts/windows/README.md`.

## Assumptions

- The primary user is the owner of local personal archives on Windows 10 or 11 and can grant ordinary
  user access to selected folders.
- Strict mode is used unless the user explicitly enables content-only comparison.
- Scan-only and dry-run are always safe to run with read-only source access.
- Same-volume quarantine is preferred; if unavailable, the operation fails closed until a separately
  designed, copy-verify-source-preserve workflow is approved.
- Estimated remaining time is advisory and may change with file size, device, caching, and contention.
- Network shares and cloud placeholders are supported conservatively and may be skipped when stable
  identity or complete content cannot be proven.
- Permanent deletion remains deferred until all prerequisite safety suites and recovery evidence pass;
  once passed, it is enabled only through the isolated User Story 8 quarantine workflow.
- Runtime network access occurs only in the native helper embedded in Windows setup. Normal application
  scanning, review, quarantine, restore, history, reporting, and recovery remain fully local.
- "All user data" in uninstall means all state stored under the product's fixed `%APPDATA%` and
  `%LOCALAPPDATA%` roots. Quarantined user documents are real recoverable files, not disposable app
  cache, and therefore remain outside automatic uninstall deletion.

## Out of Scope

- Similarity, fuzzy matching, semantic document comparison, and edition detection.
- Duplicate decisions based only on name, size, timestamp, sampled content, or a single digest.
- Mandatory GPU acceleration or upload of any file data to an online service.
- Automatic scanning at startup, hidden background service behavior, or default telemetry.
- Direct source deletion from containerized/headless operation.
- Cross-volume quarantine that removes the source before a separately verified destination and explicit
  policy exist.
- Bundling a fixed WebView2 runtime or the 200-MiB standalone runtime inside the online setup EXE.
- Downloading Visual C++ Redistributable without a release-binary dependency audit proving it is
  required.
- Deleting user-selected source folders, arbitrary export destinations, or per-volume quarantine
  documents as part of ordinary application uninstall.
