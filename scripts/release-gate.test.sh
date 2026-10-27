#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Tests for the release gate: scripts/release-gate.sh (hub#895)
#
# Run:  scripts/release-gate.test.sh
#
# The gate is the last job of `tauri-release.yml` and the only thing that makes
# a green tag mean "published". Its whole job is to turn a *silent* skip into a
# red run, so the one thing it must never do is pass by accident — and the one
# place it can never be tried out is a tag, which is not re-runnable.
#
# So the decision lives in a script and the script is exercised here, with the
# job results fed in as data. Every case below is a shape that has happened or
# that would hide a non-publication:
#   · a channel skipped because its Variable is empty   → red, naming the Variable
#   · a channel deliberately not live yet               → green, but SAID OUT LOUD
#   · a mandatory channel someone tried to excuse       → red (the list cannot cover it)
#   · a typo in the excuse list                         → red (a guard that does not
#                                                         validate its payload fails open)
#   · a declared channel skipped with its Variable set  → green, said out loud (app#7: the
#                                                         workflow skips a declared channel's job)
# ─────────────────────────────────────────────────────────────────────────────
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GATE="$ROOT/scripts/release-gate.sh"

pass=0
fail=0
ok()  { printf '  \033[32m✓\033[0m %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  \033[31m✗\033[0m %s\n     %s\n' "$1" "$2"; fail=$((fail + 1)); }

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/erplora-release-gate-test.XXXXXX")
trap 'rm -rf "$tmp_dir"' EXIT

# The three channels of a real release, in the format the workflow feeds the gate:
#   id | display name | job result | gating Variable | Variable set? | reference
# A channel with NO gating Variable is mandatory: nothing can excuse it.
channels() {
    local store="$1" play="$2" s3="$3" store_var="${4:-false}" play_var="${5:-false}"
    cat <<EOF
store|Microsoft Store|$store|MICROSOFT_STORE_PRODUCT_ID|$store_var|hub#392
play|Google Play|$play|PLAY_PACKAGE_NAME|$play_var|hub#308
s3|Object Storage (latest.json)|$s3|||hub#400
EOF
}

builds() {
    local desktop="$1" android="$2"
    cat <<EOF
build|Desktop bundles|$desktop
build-android|Android AAB + APK|$android
EOF
}

# Runs the gate. $1 = pending list, $2 = channels, $3 = builds.
# Leaves the exit code in $status, stdout+stderr in $out and the job summary in
# $summary (the gate writes it where GITHUB_STEP_SUMMARY points, like Actions does).
run_gate() {
    local summary_file="$tmp_dir/summary.md"
    : > "$summary_file"
    out=$(
        RELEASE_CHANNELS_PENDING="$1" \
        CHANNELS="$2" \
        BUILDS="${3:-}" \
        GITHUB_STEP_SUMMARY="$summary_file" \
        bash "$GATE" 2>&1
    )
    status=$?
    summary=$(cat "$summary_file")
}

# Asserts the gate exited 0 / non-0, printing what it said when it did not.
expect_green() { [ "$status" -eq 0 ] || { bad "$1" "exit $status — the gate said: $out"; return 1; }; ok "$1"; }
expect_red()   { [ "$status" -ne 0 ] || { bad "$1" "exit 0 — a non-publication passed as green: $out"; return 1; }; ok "$1"; }
expect_says()  { case "$out" in *"$2"*) ok "$1";; *) bad "$1" "the output never mentions '$2'. It said: $out";; esac; }
expect_summary_says() { case "$summary" in *"$2"*) ok "$1";; *) bad "$1" "the job summary never mentions '$2'. It said: $summary";; esac; }

echo
echo "release-gate.sh (hub#895)"
echo

# ── The script has to exist and be runnable ──────────────────────────────────
if [ -x "$GATE" ]; then
    ok "scripts/release-gate.sh exists and is executable"
else
    bad "scripts/release-gate.sh exists and is executable" "not found or not +x: $GATE"
    printf '\n\033[31m%s\033[0m\n\n' "the gate script is missing — nothing else can be tested"
    exit 1
fi

# ── Everything published: the only case that may be green in silence ─────────
run_gate "" "$(channels success success success true true)" "$(builds success success)"
expect_green "every channel published → green"
expect_summary_says "the summary lists what was published" "Microsoft Store"

# ── A channel skipped because its Variable is empty: the bug this closes ─────
run_gate "" "$(channels skipped success success false true)" "$(builds success success)"
expect_red  "a channel skipped for a missing Variable → RED"
expect_says "the failure names the Variable that is missing" "MICROSOFT_STORE_PRODUCT_ID"
expect_says "the failure is an Actions error annotation" "::error::"

run_gate "" "$(channels success skipped success true false)" "$(builds success success)"
expect_red  "Google Play skipped for a missing Variable → RED"
expect_says "the failure names PLAY_PACKAGE_NAME" "PLAY_PACKAGE_NAME"

# ── A channel that is deliberately not live yet ──────────────────────────────
# Green, because "not published" is a decision here and not an accident — but it
# is stated in the annotations and in the summary, never swallowed.
run_gate "store" "$(channels skipped success success false true)" "$(builds success success)"
expect_green "a channel declared in RELEASE_CHANNELS_PENDING → green"
expect_says  "…and the run WARNS that it did not publish" "::warning::"
expect_says  "…naming where the decision lives" "hub#392"
expect_summary_says "…and the summary shows the channel as not published" "Microsoft Store"

# Both channels, written the way a human writes a Variable: spaces and capitals.
run_gate " Store, PLAY " "$(channels skipped skipped success false false)" "$(builds success success)"
expect_green "the pending list is case- and space-insensitive"

# ── A mandatory channel can NEVER be excused ─────────────────────────────────
# `upload-s3` writes `latest.json`, the only thing the installed app compares its
# version against (hub#400). Skipping it leaves the fleet without an update
# channel, so it has no gating Variable and the excuse list must not reach it.
run_gate "s3" "$(channels success success skipped true true)" "$(builds success success)"
expect_red  "a mandatory channel listed as pending → RED anyway"
expect_says "…and the gate says the list cannot cover it" "s3"

run_gate "" "$(channels success success skipped true true)" "$(builds success success)"
expect_red  "the mandatory upload skipped → RED"
expect_says "…naming the update manifest it did not write" "latest.json"

# ── A guard that does not validate its own payload fails open ────────────────
run_gate "stroe" "$(channels skipped success success false true)" "$(builds success success)"
expect_red  "a typo in the pending list does NOT excuse anything"
expect_says "…and the typo is named" "stroe"

# ── Pending excuses a skip, never a failure ──────────────────────────────────
run_gate "store" "$(channels failure success success true true)" "$(builds success success)"
expect_red  "a publish job that FAILED is red even if the channel is pending"

run_gate "" "$(channels cancelled success success true true)" "$(builds success success)"
expect_red  "a cancelled publish job → RED"

# ── A declared channel's job is skipped by its own `if:` (app#7) ─────────────
# Since app#7 the workflow skips the job of a channel listed in RELEASE_CHANNELS_PENDING even when
# its Variable IS set (on 27/10 Play ran while declared pending, failed, and kept 1.1.31 red). The
# gate then sees `skipped` + Variable set + declared: that is «declared pending», green and said
# out loud — never «skipped with nothing gating it out». The rows are the workflow's own, S3
# included (optional: its two Variables gate it).
workflow_channels() { # $1 store, $2 play, $3 s3 — job results, every Variable set
    cat <<EOF
store|Microsoft Store|$1|MICROSOFT_STORE_PRODUCT_ID|true|hub#392
play|Google Play|$2|PLAY_PACKAGE_NAME|true|hub#308
s3|S3 downloads (latest.json)|$3|AWS_DOWNLOADS_ROLE_ARN/DOWNLOADS_BUCKET|true|hub#400
EOF
}

run_gate "play" "$(workflow_channels success skipped success)" "$(builds success success)"
expect_green "Google Play skipped while declared pending, PLAY_PACKAGE_NAME set → green"
expect_says  "…said out loud as declared pending" "declared pending"
expect_says  "…as a warning, not in silence" "::warning::"

# `upload-s3` skipped by its `if:` has no `outputs.result`: the row falls back to the job's `skipped`.
run_gate "store,play,s3" "$(workflow_channels skipped skipped skipped)" "$(builds success success)"
expect_green "every channel declared pending and skipped, every Variable set → green"

# The other half of the rule: a channel that is NOT declared and fails still blocks the release.
run_gate "" "$(workflow_channels success failure success)" "$(builds success success)"
expect_red  "Google Play NOT declared pending and failed → RED"
expect_says "…naming the failure" "failure"

run_gate "" "$(workflow_channels success skipped success)" "$(builds success success)"
expect_red  "Google Play skipped, Variable set, NOT declared → RED (nothing excuses that skip)"

# Without the fallback an S3 job skipped by its `if:` would report an empty result: never excused.
run_gate "s3" "$(workflow_channels success success '')" "$(builds success success)"
expect_red  "an empty channel result is never excused, even declared pending → RED"

# ── The build is the root cause when it is the build ─────────────────────────
# A failed build skips every publish downstream. The gate must blame the build,
# not send anyone hunting for a Variable that is perfectly fine.
run_gate "" "$(channels skipped skipped skipped true true)" "$(builds failure success)"
expect_red  "a failed build → RED"
expect_says "…naming the build as the cause" "Desktop bundles"

# ── Stale bookkeeping is a warning, never a red ──────────────────────────────
# The channel published: turning the release red over an un-tidied Variable would
# punish exactly the outcome we want.
run_gate "store" "$(channels success success success true true)" "$(builds success success)"
expect_green "a channel that published while still listed pending → green"
expect_says  "…with a warning to clean the list" "RELEASE_CHANNELS_PENDING"

echo
printf 'passed: %d   failed: %d\n\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
