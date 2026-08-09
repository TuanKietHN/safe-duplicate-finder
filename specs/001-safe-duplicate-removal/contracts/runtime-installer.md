# Runtime Installer Contract

The runtime helper is a native Windows executable embedded in the public NSIS setup and MUST start
without WebView2. It accepts no user document path and exposes no network API to the installed
application. NSIS invokes it before reporting installation success.

## Embedded Inputs

- A versioned JSON runtime manifest.
- Product identity `io.github.safeduplicate.finder`.

Every helper embeds each runtime artifact's pinned HTTPS URL, size, SHA-256, architecture, preflight
rule, and installer argument vector. The release records separate SHA-256 values for the public NSIS
setup, helper, manifest source, and installed application. Unknown manifest fields are rejected unless
the schema version explicitly permits them.

## Command Line

```text
safe-dedupe-setup.exe [--cache-dir <absolute-product-cache-path>] [--log <path>]
                      [--passive] [--no-launch]
```

Release mode refuses a cache directory outside the fixed product app-data root. Test builds may use
an injected manifest/server only behind a compile-time test feature and MUST NOT ship that feature.

Exit codes:

- `0`: all required runtimes are already valid or were installed and detected successfully.
- `2`: embedded manifest or command line invalid.
- `3`: runtime preflight/architecture unsupported.
- `4`: download cancelled by the user.
- `5`: network/retry exhausted.
- `6`: exact length or SHA-256 verification failed.
- `7`: runtime installer returned failure.
- `20`: unexpected local I/O/Win32 failure.

## Progress Contract

The UI reads immutable snapshots with:

```text
required_download_bytes
received_bytes
network_bytes_this_run
bytes_per_second
eta_seconds?
overall_basis_points
current_artifact_id?
items[] { id, display_name, state, received_bytes, size_bytes, message }
```

`received_bytes` is derived from successful write completions and verified cache lengths. Speed is a
rolling window of `network_bytes_this_run`, and ETA is computed only while speed is non-zero. The UI
MUST NOT increment progress through a timer, animation callback, or estimated phase duration.

## Download Contract

1. Preflight every item before opening its artifact URL.
2. Rehash a completed cache file; skip network only when length and SHA-256 match.
3. Resume `.part` with `Range: bytes=N-`; append only on matching `206 Content-Range`.
4. If resume receives 200, restart that item at zero without changing other item results.
5. Stream with 64 KiB buffers and no more than two workers.
6. Retry transient failures at most three times and retain valid partial bytes.
7. Require exact final length and SHA-256 before atomic promotion and execution.
8. Retain verified completed cache after a later install failure so retry does not redownload.

## Uninstall Contract

The public NSIS setup creates:

- `Trình tìm tệp trùng lặp an toàn.lnk` -> installed app executable.
- `Gỡ cài đặt Trình tìm tệp trùng lặp an toàn.lnk` -> installed `uninstall.exe`.

On explicit uninstall, cleanup removes the installation directory, `%APPDATA%` and `%LOCALAPPDATA%`
product roots, installer cache, product registry keys, and both shortcuts. On update-mode uninstall it
preserves app-local data. Cleanup never enumerates or recursively removes source roots, export
destinations, volume roots, or `.safe-duplicate-finder-quarantine` directories.
