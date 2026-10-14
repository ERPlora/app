#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Tests for the version gate: scripts/release-version.sh
#
# Run:  scripts/release-version.test.sh
#
# `tauri-release.yml` runs on every push to `main`. Its first job asks this
# script one question: does THIS push release? The answer lives in two places
# only — `version` in `src-tauri/tauri.conf.json` (the single source of truth)
# and the `v*` tags of the repository (one per version already released):
#
#   · tag `v<version>` exists                → nothing to release (docs/CI push)
#   · tag missing, version > latest tag      → build, publish, then tag
#   · tag missing, version <= latest tag     → RED: the stores only go forward
#   · not X.Y.Z, or minor/patch >= 1000      → RED: Play's versionCode formula
#
# Every case is fed as data (the version and the tag list), so the decision can
# be tried here instead of on `main`, where a wrong "yes" publishes to Microsoft
# Store and Google Play irreversibly.
# ─────────────────────────────────────────────────────────────────────────────
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GATE="$ROOT/scripts/release-version.sh"

pass=0
fail=0
ok()  { printf '  \033[32m✓\033[0m %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  \033[31m✗\033[0m %s\n     %s\n' "$1" "$2"; fail=$((fail + 1)); }

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/erplora-release-version-test.XXXXXX")
trap 'rm -rf "$tmp_dir"' EXIT

# Runs the gate. $1 = version, $2 = existing tags, $3 = event, $4 = publish input.
# Leaves the exit code in $status, stdout+stderr in $out, the step outputs in
# $outputs and the job summary in $summary — where Actions would put them.
run_gate() {
    local outputs_file="$tmp_dir/outputs" summary_file="$tmp_dir/summary.md"
    : > "$outputs_file"
    : > "$summary_file"
    out=$(
        VERSION="$1" \
        TAGS="$2" \
        EVENT_NAME="${3:-push}" \
        PUBLISH="${4:-}" \
        GITHUB_OUTPUT="$outputs_file" \
        GITHUB_STEP_SUMMARY="$summary_file" \
        bash "$GATE" 2>&1
    )
    status=$?
    outputs=$(cat "$outputs_file")
    summary=$(cat "$summary_file")
}

expect_green() { [ "$status" -eq 0 ] || { bad "$1" "exit $status — the gate said: $out"; return 1; }; ok "$1"; }
expect_red()   { [ "$status" -ne 0 ] || { bad "$1" "exit 0 — the gate let it through: $out"; return 1; }; ok "$1"; }
expect_output() {
    if grep -qx -- "$2" <<<"$outputs"; then ok "$1"; else bad "$1" "no output line '$2'. Outputs were: $outputs"; fi
}
expect_says()  { case "$out" in *"$2"*) ok "$1";; *) bad "$1" "the output never mentions '$2'. It said: $out";; esac; }
expect_summary_says() { case "$summary" in *"$2"*) ok "$1";; *) bad "$1" "the job summary never mentions '$2'. It said: $summary";; esac; }

TAGS_TODAY=$'v1.1.28\nv1.1.29\nv1.1.30'

echo
echo "release-version.sh (push to main releases only a new version)"
echo

if [ -x "$GATE" ]; then
    ok "scripts/release-version.sh exists and is executable"
else
    bad "scripts/release-version.sh exists and is executable" "not found or not +x: $GATE"
    printf '\n\033[31m%s\033[0m\n\n' "the version gate script is missing — nothing else can be tested"
    exit 1
fi

# ── A push whose version is already tagged releases nothing ──────────────────
run_gate "1.1.30" "$TAGS_TODAY" push
expect_green  "version already tagged → green"
expect_output "…does not build" "build=false"
expect_output "…does not release" "release=false"
expect_output "…still names the version" "version=1.1.30"
expect_output "…and its tag" "tag=v1.1.30"
expect_summary_says "the summary says it is already released" "already released"
expect_summary_says "…and how to release (bump tauri.conf.json)" "tauri.conf.json"

# ── A new, greater version releases ──────────────────────────────────────────
run_gate "1.1.31" "$TAGS_TODAY" push
expect_green  "a new version greater than the latest tag → green"
expect_output "…builds" "build=true"
expect_output "…releases" "release=true"
expect_output "…with the tag it will create" "tag=v1.1.31"
expect_summary_says "the summary announces the release" "v1.1.31"

run_gate "1.0.0" "" push
expect_green  "the first version of a repo with no tags releases"
expect_output "…releases" "release=true"

# ── The stores only go forward ───────────────────────────────────────────────
run_gate "1.0.2" "$TAGS_TODAY" push
expect_red  "a new version LOWER than the latest tag → RED"
expect_says "…as an error annotation" "::error::"
expect_says "…naming the version" "1.0.2"
expect_says "…and the latest tag it must exceed" "v1.1.30"
expect_output "…and nothing is released" "release=false"

# Numbers, not strings: `1.10.0` sorts before `1.9.0` as text.
run_gate "1.10.0" $'v1.9.0\nv1.2.0' push
expect_green  "1.10.0 > 1.9.0 (numeric compare, not text)"
expect_output "…releases" "release=true"

run_gate "1.9.9" $'v1.10.0' push
expect_red "1.9.9 < 1.10.0 (numeric compare, not text) → RED"

run_gate "2.0.0" $'v1.999.999' push
expect_green "2.0.0 > 1.999.999"

# Tags that are not a release (`v1.2.0-rc.1`, `latest`) do not move the floor.
run_gate "1.1.31" $'v1.1.30\nv9.0.0-rc.1\nlatest\nv2' push
expect_green  "pre-release and non-version tags are ignored"
expect_output "…and the release goes ahead" "release=true"

# ── What the stores cannot take ──────────────────────────────────────────────
run_gate "1.2" "$TAGS_TODAY" push
expect_red  "a version that is not X.Y.Z → RED"
expect_says "…saying what it must look like" "X.Y.Z"

run_gate "1.2.0-rc.1" "$TAGS_TODAY" push
expect_red "a pre-release version → RED (Store and Play versions are final)"

run_gate "1.02.0" "$TAGS_TODAY" push
expect_red "a version with a leading zero → RED"

run_gate "1.1000.0" "$TAGS_TODAY" push
expect_red  "minor >= 1000 → RED"
expect_says "…naming the Android versionCode it would break" "versionCode"

run_gate "1.2.1000" "$TAGS_TODAY" push
expect_red "patch >= 1000 → RED"

# ── Manual dispatch ──────────────────────────────────────────────────────────
run_gate "1.1.31" "$TAGS_TODAY" workflow_dispatch true
expect_green  "dispatch with publish=true and a new version → releases"
expect_output "…releases" "release=true"

run_gate "1.1.30" "$TAGS_TODAY" workflow_dispatch true
expect_green  "dispatch with publish=true and a tagged version → nothing to do"
expect_output "…does not build" "build=false"
expect_output "…does not release" "release=false"

run_gate "1.1.31" "$TAGS_TODAY" workflow_dispatch false
expect_green  "dispatch with publish=false → builds only"
expect_output "…builds" "build=true"
expect_output "…and never publishes" "release=false"
expect_summary_says "…and the summary says nothing is published" "publish=false"

run_gate "1.1.30" "$TAGS_TODAY" workflow_dispatch false
expect_green  "dispatch with publish=false on a tagged version → builds only (packaging check)"
expect_output "…builds" "build=true"
expect_output "…and never publishes" "release=false"

# ── The version comes from tauri.conf.json when not given ────────────────────
mkdir -p "$tmp_dir/conf"
printf '{\n  "productName": "ERPlora",\n  "version": "1.4.2"\n}\n' > "$tmp_dir/conf/tauri.conf.json"
outputs_file="$tmp_dir/outputs"
: > "$outputs_file"
out=$(
    TAURI_CONF="$tmp_dir/conf/tauri.conf.json" \
    TAGS="$TAGS_TODAY" \
    EVENT_NAME=push \
    GITHUB_OUTPUT="$outputs_file" \
    GITHUB_STEP_SUMMARY="$tmp_dir/summary.md" \
    bash "$GATE" 2>&1
)
status=$?
outputs=$(cat "$outputs_file")
expect_green  "reads the version from tauri.conf.json"
expect_output "…and uses it" "version=1.4.2"

: > "$outputs_file"
out=$(
    TAURI_CONF="$tmp_dir/conf/missing.json" \
    TAGS="$TAGS_TODAY" \
    EVENT_NAME=push \
    GITHUB_OUTPUT="$outputs_file" \
    GITHUB_STEP_SUMMARY="$tmp_dir/summary.md" \
    bash "$GATE" 2>&1
)
status=$?
expect_red "an unreadable tauri.conf.json → RED, never a guessed version"

# The real file of this repo must hold a version the gate accepts.
: > "$outputs_file"
out=$(TAGS="" EVENT_NAME=push GITHUB_OUTPUT="$outputs_file" GITHUB_STEP_SUMMARY="$tmp_dir/summary.md" bash "$GATE" 2>&1)
status=$?
expect_green "this repo's src-tauri/tauri.conf.json holds a valid X.Y.Z"

echo
printf 'passed: %d   failed: %d\n\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
