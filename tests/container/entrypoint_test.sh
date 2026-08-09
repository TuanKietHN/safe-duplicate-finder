#!/bin/sh
set -eu

entrypoint="${1:?entrypoint path is required}"
sandbox="$(mktemp -d)"
trap 'rm -rf "$sandbox"' EXIT HUP INT TERM

cat >"$sandbox/findmnt" <<'EOF'
#!/bin/sh
printf '%s\n' "${FAKE_MOUNT_OPTIONS:?}"
EOF
chmod 0700 "$sandbox/findmnt"

run_expected() {
  expected="$1"
  shift
  set +e
  "$@" >"$sandbox/stdout" 2>"$sandbox/stderr"
  actual=$?
  set -e
  if [ "$actual" -ne "$expected" ]; then
    cat "$sandbox/stdout" >&2
    cat "$sandbox/stderr" >&2
    echo "expected exit $expected, got $actual" >&2
    exit 1
  fi
}

common="PATH=$sandbox:/usr/bin:/bin"
run_expected 0 env "$common" FAKE_MOUNT_OPTIONS=ro SAFE_DEDUPE_MODE=scan \
  SAFE_DEDUPE_BINARY=/bin/echo "$entrypoint" check
grep -q -- '--database /data/state.db --log-directory /data/logs check' "$sandbox/stdout"

run_expected 3 env "$common" FAKE_MOUNT_OPTIONS=rw SAFE_DEDUPE_MODE=scan \
  SAFE_DEDUPE_BINARY=/bin/echo "$entrypoint" check
run_expected 3 env "$common" FAKE_MOUNT_OPTIONS=ro SAFE_DEDUPE_MODE=quarantine \
  SAFE_DEDUPE_QUARANTINE_ROOT=/scan/.safe-duplicate-finder-quarantine \
  SAFE_DEDUPE_BINARY=/bin/echo "$entrypoint" check
run_expected 3 env "$common" FAKE_MOUNT_OPTIONS=rw SAFE_DEDUPE_MODE=quarantine \
  SAFE_DEDUPE_BINARY=/bin/echo "$entrypoint" check
run_expected 3 env "$common" FAKE_MOUNT_OPTIONS=rw SAFE_DEDUPE_MODE=quarantine \
  SAFE_DEDUPE_QUARANTINE_ROOT=/other/quarantine SAFE_DEDUPE_BINARY=/bin/echo \
  "$entrypoint" check
run_expected 3 env "$common" FAKE_MOUNT_OPTIONS=rw SAFE_DEDUPE_MODE=quarantine \
  SAFE_DEDUPE_QUARANTINE_ROOT=/scan/.safe-duplicate-finder-quarantine \
  SAFE_DEDUPE_BINARY=/bin/echo "$entrypoint" scan start --project ignored
run_expected 3 env "$common" FAKE_MOUNT_OPTIONS=rw SAFE_DEDUPE_MODE=quarantine \
  SAFE_DEDUPE_QUARANTINE_ROOT=/scan/.safe-duplicate-finder-quarantine \
  SAFE_DEDUPE_BINARY=/bin/echo "$entrypoint" quarantine delete-prepare --entry ignored
run_expected 3 env "$common" FAKE_MOUNT_OPTIONS=rw SAFE_DEDUPE_MODE=quarantine \
  SAFE_DEDUPE_QUARANTINE_ROOT=/scan/.safe-duplicate-finder-quarantine \
  SAFE_DEDUPE_BINARY=/bin/echo "$entrypoint" quarantine delete-execute --batch ignored

run_expected 0 env "$common" FAKE_MOUNT_OPTIONS=rw SAFE_DEDUPE_MODE=quarantine \
  SAFE_DEDUPE_QUARANTINE_ROOT=/scan/.safe-duplicate-finder-quarantine \
  SAFE_DEDUPE_BINARY=/bin/echo "$entrypoint" quarantine apply --plan ignored --confirm QUARANTINE
grep -q -- '--quarantine-root /scan/.safe-duplicate-finder-quarantine' "$sandbox/stdout"
