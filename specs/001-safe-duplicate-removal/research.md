# Research: Safe Duplicate File Management

## Decision 1: Rust and desktop baseline

- **Decision**: Rust 1.97.1, edition 2024, Tauri 2, TypeScript frontend, Node.js 24.x.
- **Rationale**: Rust provides explicit error handling and memory safety for the reusable engine. Tauri
  keeps the Windows desktop shell thin while using the same Rust process. The installed Node version
  is 24.11.0; Rust 1.97.1 is the current stable patch as of 2026-07-21.
- **Alternatives considered**: Electron (larger runtime and duplicates backend boundary); C# only
  (conflicts with requested reusable Rust core); GPU pipeline (no demonstrated I/O benefit).

## Decision 2: SQLite access model

- **Decision**: `rusqlite` with bundled SQLite; WAL, `synchronous=FULL`, `foreign_keys=ON`, busy timeout,
  a single writer actor, short read connections, batched metadata writes, explicit migrations.
- **Rationale**: SQLite WAL preserves the main database and appends committed records while readers can
  continue. A single writer creates deterministic event ordering and avoids worker-level lock storms.
- **Alternatives considered**: SQLx async pool (unneeded concurrency and larger surface); one connection
  per worker (write contention); custom append database (less mature crash semantics).

## Decision 3: Candidate and evidence pipeline

- **Decision**: Persist metadata, group by strict key `(normalized_name, size)` or content key `size`,
  compute a domain-separated sampled BLAKE3 rejector, then full streaming BLAKE3, then a separate full
  streaming SHA-256 pass only for equal BLAKE3 groups.
- **Rationale**: Unique sizes and sampled differences avoid unnecessary whole-file I/O while sampled
  evidence can never produce a positive decision. Separate full passes satisfy the staged policy.
- **Alternatives considered**: Compute both full digests in one pass (violates staged SHA trigger and
  reads SHA for candidates BLAKE3 would reject); memory-map files (address-space and mutation behavior
  harder to bound); GPU hashing (transfer and storage I/O dominate).

## Decision 4: Windows file identity and safe move

- **Decision**: Open handles and use volume serial plus 128-bit file ID. Revalidate the same handle
  immediately before a replacement-disabled same-volume rename. Never set cross-volume copy or
  replace-existing behavior.
- **Rationale**: The pair uniquely identifies a file on one computer and handle-bound operations reduce
  time-of-check/time-of-use path replacement. Same-volume rename avoids a hidden copy/delete sequence.
- **Alternatives considered**: Canonical path only (aliases/races); timestamps only (coarse and mutable);
  generic cross-volume move (may delete source after copy and loses simple crash semantics).

## Decision 5: Scheduler and backpressure

- **Decision**: Bounded crossbeam channels, cooperative pause/cancel token, one enumerator coordinator,
  metadata workers, serialized DB writer, and per-volume full-read semaphores. Defaults: unknown/network
  and HDD 1 full-read worker, SATA SSD 2, NVMe up to 4; global CPU work capped below logical CPUs.
- **Rationale**: Storage queue depth, not CPU count alone, controls large sequential read performance.
  Bounded queues cap memory and permit safe cancellation at file/chunk boundaries.
- **Alternatives considered**: thread per file (unbounded); global Rayon pool without per-volume limits
  (easy disk saturation); unbounded async tasks (unbounded memory/pending handles).

## Decision 6: Journaling and reconciliation

- **Decision**: SQLite transaction row plus append-only event rows and flushed JSONL manifest. Commit
  `planned`, revalidate, commit `moving`, perform move, verify destination, commit `verified`. Startup
  reconciliation checks source and destination and emits a new event; it never rewrites history.
- **Rationale**: Neither filesystem nor SQLite provides a shared atomic transaction. An explicit saga
  records enough evidence to determine the real state after every interruption.
- **Alternatives considered**: two-phase commit (filesystem has no participant protocol); assume rename
  success after return (does not cover crash before DB update); delete-then-record (irrecoverable).

## Decision 7: Quarantine layout

- **Decision**: `.safe-duplicate-finder-quarantine/<project>/<session>/<entry-id>/<relative-path>` on the
  same volume, excluded from all scans. Entry ID prevents collisions; original path remains metadata.
- **Rationale**: Same-volume mutation and unique directories prevent overwrite and keep sessions easy
  to inspect manually.
- **Alternatives considered**: one central quarantine (cross-volume moves); flat filenames (collisions
  and poor recovery); Recycle Bin default (less explicit restore/audit control).

## Decision 8: Logging, privacy, and reports

- **Decision**: `tracing` structured events to JSONL plus concise text logs; immutable audit events;
  local CSV/JSON/HTML exports. No network client dependency in production crates.
- **Rationale**: Two log forms serve diagnosis and ordinary users while the dependency boundary makes
  accidental telemetry harder.
- **Alternatives considered**: cloud crash reporting (privacy conflict); text-only logs (weak parsing);
  database-only audit (harder manual recovery if app fails).

## Decision 9: Packaging

- **Decision**: One compact Tauri NSIS executable is the public online setup. It embeds a native,
  WebView-independent runtime helper, uses `webviewInstallMode=skip`, and invokes that helper before
  NSIS may report installation success. Optional MSI remains secondary where VBSCRIPT/WiX prerequisites exist;
  a multi-stage non-root Docker image remains available for CLI/headless scan.
- **Rationale**: Windows packaging must be built on Windows; NSIS has fewer optional Windows feature
  requirements. The default Tauri `downloadBootstrapper` shows only the small WebView2 bootstrapper
  transfer and delegates the large runtime transfer to another process, so it cannot provide exact
  aggregate runtime byte progress or application-controlled resume. Keeping the downloader as an
  embedded native helper avoids a circular dependency on WebView2 while preserving one small setup
  file. Docker is documented as
  secondary because native Windows volume identity, Recycle Bin, and ACL behavior are not faithfully
  available through Linux containers.
- **Alternatives considered**: Docker desktop-only product (unsafe platform mismatch); portable binary
  only (no complete desktop install/update experience); Tauri's default downloaded bootstrapper
  (insufficient byte-level visibility/resume); embedded offline WebView2 (~127–200 MiB added to every
  setup download); fixed WebView2 (~180 MiB and app-owned patch cadence); Tauri/WebView bootstrapper
  (cannot run when the missing runtime is exactly what setup must install).

## Decision 10: Runtime inventory and artifact pinning

- **Decision**: The x86_64 Windows manifest contains only Microsoft WebView2 Evergreen Standalone x64.
  The 2026-08-06 signed artifact is pinned at 209,605,840 bytes with SHA-256
  `4AC55375E52435B5AAF2E2E76D81F539A2602CEA38D4B647F5FAADE467C6E078` and an immutable Microsoft
  delivery URL. Every release regenerates and verifies this evidence before publishing.
- **Rationale**: `dumpbin /dependents` on the release executable shows Windows system DLLs and UCRT API
  sets but no `VCRUNTIME140.dll` or `MSVCP140.dll`; Windows 10/11 provides UCRT. Installing the Visual
  C++ Redistributable would add an unnecessary transfer, elevation/update surface, and failure mode.
  WebView2 is the only non-system runtime required by the Tauri desktop shell.
- **Alternatives considered**: Always download VC++ redist (not dependency-driven); download the small
  WebView2 Evergreen bootstrapper (its subsequent large transfer is opaque to our byte counters);
  use mutable `aka.ms`/fwlink as the executed artifact without a pinned digest (cannot enforce a
  stable mandatory SHA-256).

## Decision 11: Downloader transport, concurrency, and resume

- **Decision**: Use native WinHTTP over HTTPS with synchronous streaming reads on a bounded two-worker
  pool. Each artifact uses a 64 KiB buffer, content-addressed completed cache, `.part` resume file,
  HTTP `Range`, exact Content-Range validation, three retries, and streaming SHA-256 before atomic
  promotion.
- **Rationale**: WinHTTP is present on supported Windows versions and avoids shipping another TLS/HTTP
  runtime. Two workers overlap independent latency without unbounded memory or handle growth. Final
  digest validation protects against changed or corrupted response bytes; treating status 200 during
  resume as a clean restart prevents duplicate append.
- **Alternatives considered**: Sequential-only download (needlessly slow with multiple independent
  artifacts); one task per chunk (more server load and complex reassembly); unbounded async client
  (larger binary and memory surface); BITS (good resilience but harder single-window deterministic
  aggregate UI/portable tests); trust ETag alone (not a content-integrity proof).

## Decision 12: Progress and uninstall boundaries

- **Decision**: UI snapshots are derived solely from bytes successfully written plus revalidated cache
  length. Rolling speed and ETA are computed from new network-byte samples. NSIS creates a dedicated
  uninstall shortcut. Explicit uninstall removes fixed app-local install/data/cache/registry/shortcut
  roots, while update-mode uninstall preserves state and all per-volume quarantine paths are excluded.
- **Rationale**: Timer-driven progress lies under slow/fast networks and is especially misleading for
  a 200-MiB runtime. Fixed cleanup roots make the destructive scope auditable. Quarantine contains the
  user's only remaining real file in some workflows, so treating it as disposable app cache would
  violate the data-preservation constitution.
- **Alternatives considered**: Synthetic percent-by-phase; delete only binaries and leave hidden
  database/log/cache; recursively search drives for product-named folders (unsafe broad deletion);
  delete quarantine on uninstall (irreversible loss unrelated to installer state).

## Sources

- Rust stable release notes: https://doc.rust-lang.org/releases.html
- Tauri Windows prerequisites: https://v2.tauri.app/start/prerequisites/
- Tauri Windows installers: https://v2.tauri.app/distribute/windows-installer/
- Microsoft WebView2 distribution: https://learn.microsoft.com/microsoft-edge/webview2/concepts/distribution
- Microsoft WebView2 Evergreen versus Fixed: https://learn.microsoft.com/microsoft-edge/webview2/concepts/evergreen-vs-fixed-version
- Microsoft WinHTTP read API: https://learn.microsoft.com/windows/win32/api/winhttp/nf-winhttp-winhttpreaddata
- Microsoft WinHTTP query flags/Content-Range: https://learn.microsoft.com/windows/win32/winhttp/query-info-flags
- Microsoft VC++ redistributable dependency guidance: https://learn.microsoft.com/cpp/windows/redistributing-visual-cpp-files
- SQLite WAL: https://sqlite.org/wal.html
- Windows `FILE_ID_INFO`: https://learn.microsoft.com/windows/win32/api/winbase/ns-winbase-file_id_info
- Windows `MoveFileExW`: https://learn.microsoft.com/windows/win32/api/winbase/nf-winbase-movefileexw
