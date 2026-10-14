#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Tests for scripts/ci/s3-version-guard.sh — a version already on S3 is never
# overwritten by ANOTHER run.
#
# Run:  bash scripts/tests/s3-version-guard.test.sh
#
# `upload-s3` writes `downloads/app/v<version>/…` and then `latest/…`, which the
# whole fleet reads. If a release fails halfway (Play and S3 done, Store red) a
# later run of the same version would rebuild OTHER code and overwrite both. The
# guard asks S3 for `downloads/app/v<version>/latest.json` (written with the
# metadata `run-id`) before the first copy:
#
#   · not there (404)                    → go (first upload of this version)
#   · there, written by THIS run         → go (a re-run of the same run)
#   · there, written by another run      → RED, naming the run
#   · there, with no run id              → RED (cannot prove it is ours)
#   · S3 cannot be asked (403, network…) → RED (fail closed, never assume 404)
#
# `aws` is a fake on PATH: no network, no credentials.
# ─────────────────────────────────────────────────────────────────────────────
set -uo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
GUARD="$repo_root/scripts/ci/s3-version-guard.sh"

pass=0
fail=0
ok()  { printf '  \033[32m✓\033[0m %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  \033[31m✗\033[0m %s\n     %s\n' "$1" "$2"; fail=$((fail + 1)); }

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/erplora-s3-guard-test.XXXXXX")
trap 'rm -rf "$tmp_dir"' EXIT

# The fake `aws`: FAKE_S3 picks what `head-object` answers; every call is logged.
mkdir -p "$tmp_dir/bin"
cat > "$tmp_dir/bin/aws" <<'FAKE'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$FAKE_LOG"
case "$FAKE_S3" in
    missing) echo "An error occurred (404) when calling the HeadObject operation: Not Found" >&2; exit 254 ;;
    denied)  echo "An error occurred (403) when calling the HeadObject operation: Forbidden" >&2; exit 254 ;;
    nometa)  echo "None" ;;
    *)       echo "$FAKE_S3" ;;
esac
FAKE
chmod +x "$tmp_dir/bin/aws"

run() {
    : > "$tmp_dir/log"
    out=$(
        PATH="$tmp_dir/bin:$PATH" FAKE_LOG="$tmp_dir/log" FAKE_S3="$1" \
        DOWNLOADS_BUCKET="${BUCKET-erplora-downloads}" VERSION="${GUARD_VERSION-1.1.31}" GITHUB_RUN_ID="${RUN-222}" \
        bash "$GUARD" 2>&1
    )
    status=$?
    calls=$(cat "$tmp_dir/log")
}
expect_status() { [ "$status" -eq "$2" ] && ok "$1" || bad "$1" "exit $status (wanted $2): $out"; }
expect_says()   { case "$out" in *"$2"*) ok "$1";; *) bad "$1" "never mentions '$2'. It said: $out";; esac; }

echo
echo "s3-version-guard.sh (another run never overwrites a published version)"
echo

if [ -f "$GUARD" ]; then
    ok "scripts/ci/s3-version-guard.sh exists"
else
    bad "scripts/ci/s3-version-guard.sh exists" "not found: $GUARD"
    printf '\n\033[31m%s\033[0m\n\n' "the guard is missing — nothing else can be tested"
    exit 1
fi

run missing
expect_status "the version is not on S3 yet (404) → go" 0
case "$calls" in
    *"head-object"*"--bucket erplora-downloads"*"--key downloads/app/v1.1.31/latest.json"*) ok "…it asked S3 for downloads/app/v<version>/latest.json" ;;
    *) bad "…it asked S3 for downloads/app/v<version>/latest.json" "calls were: $calls" ;;
esac
case "$calls" in
    *"run-id"*) ok "…reading the object's run-id metadata" ;;
    *) bad "…reading the object's run-id metadata" "calls were: $calls" ;;
esac

run 222
expect_status "already there, written by THIS run (a re-run) → go" 0

run 111
expect_status "already there, written by ANOTHER run → RED" 1
expect_says "…as an error annotation" "::error::"
expect_says "…naming that run" "111"
expect_says "…and the version" "1.1.31"

run nometa
expect_status "already there with no run id → RED (cannot prove it is ours)" 1

run denied
expect_status "S3 cannot be asked (403) → RED, never read as «not there»" 1
expect_says "…saying what S3 answered" "403"

BUCKET= run missing
expect_status "no bucket given → usage error (exit 2)" 2

GUARD_VERSION= run missing
expect_status "no version given → usage error (exit 2)" 2

RUN= run missing
expect_status "no run id → usage error (exit 2)" 2

echo
printf 'passed: %d   failed: %d\n\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
