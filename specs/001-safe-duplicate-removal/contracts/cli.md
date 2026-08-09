# CLI Contract

Binary name: `safe-dedupe`. Global `--database` and optional `--log-directory` precede the command.
Review/list/status commands accept `--json` where shown; diagnostics and the newly allocated scan ID
go to stderr. Mutating commands require an explicit identifier and exact confirmation token.

```text
safe-dedupe --database <path> project create --name <name>
safe-dedupe --database <path> project add-root --project <uuid> --path <path> [--primary]
safe-dedupe --database <path> project set-workers --project <uuid> --workers <1..64>
safe-dedupe --database <path> project list [--json]
safe-dedupe --database <path> scan start --project <uuid> [--mode strict|content]
    [--acknowledge-content-mode] [--all-files] [--minimum-size <bytes>]
safe-dedupe scan status --session <uuid> [--json]
safe-dedupe scan pause|resume|cancel --session <uuid>
safe-dedupe results list --session <uuid> [--json]
safe-dedupe plan create --session <uuid> [--policy default|oldest|newest|shortest]
safe-dedupe plan validate --plan <uuid> [--json]
safe-dedupe dry-run --plan <uuid> [--json]
safe-dedupe quarantine apply --plan <uuid> --confirm QUARANTINE [--quarantine-root <path>]
safe-dedupe quarantine list --project <uuid> [--json]
safe-dedupe quarantine delete-prepare --entry <uuid> [--entry <uuid> ...] [--delete-now] [--json]
safe-dedupe quarantine delete-execute --batch <uuid> --token <token>
    --confirm <exact-returned-phrase> [--json]
safe-dedupe restore (--entry <uuid>|--group <uuid>|--session <uuid>|--project <uuid>)
    --confirm RESTORE
safe-dedupe recover inspect --project <uuid> [--json]
safe-dedupe recover reconcile --transaction <uuid> --confirm RECONCILE
safe-dedupe report export --session <uuid> --format csv|json|html --destination <path>
```

Exit codes: `0` success, `2` invalid input, `3` safety precondition failed, `5` cooperative
cancellation observed, `10` database/log/state durability failure, `20` I/O/serialization or
unexpected internal failure. Per-file scan errors remain isolated in the result instead of changing
the process exit code. A mutation command returning non-zero MUST NOT report reclaimed bytes.

Pause/resume/cancel are durable cross-process requests. `resume` continues an active paused worker or
restarts an `interrupted`/`blocked` read-only session after its stale evidence is invalidated. No CLI
command permanently deletes a source file. The two deletion subcommands accept only quarantine entry
and batch UUIDs. Without `--delete-now`, retention must have expired; with it, only the retention-time
gate is bypassed and a distinct exact phrase is returned. Token, phrase, identity, size, mode, and dual
full hashes are backend-enforced.
