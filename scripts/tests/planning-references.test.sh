#!/usr/bin/env bash
# Tests for scripts/ci/planning-references.sh, which fails when a tracked file other than a Markdown
# document cites something the reader does not have: a plan section, a dated decision round, a
# numbered slice, a decision record, an issue of another repo, a flow or task id, or a path of the
# old monorepo. The code is read by people who have never seen any of those.
#
# Run:  bash scripts/tests/planning-references.test.sh
#
# The fixtures build every citation at run time, so this file never contains one itself.
set -uo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
GUARD="$repo_root/scripts/ci/planning-references.sh"

pass=0
fail=0
ok()  { printf '  \033[32m✓\033[0m %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  \033[31m✗\033[0m %s\n     %s\n' "$1" "$2"; fail=$((fail + 1)); }

run() { out=$(bash "$GUARD" "$@" 2>&1); status=$?; }
expect_status() { [ "$status" -eq "$2" ] && ok "$1" || bad "$1" "exit $status (wanted $2): $out"; }
expect_says()   { case "$out" in *"$2"*) ok "$1";; *) bad "$1" "never mentions '$2'. It said: $out";; esac; }
expect_silent_about() { case "$out" in *"$2"*) bad "$1" "mentions '$2': $out";; *) ok "$1";; esac; }

section=$'\xc2\xa7'
year=2026
slice=ajada
record=ADR
old_tree="apps/t""auri"

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/erplora-planning-refs-test.XXXXXX")
trap 'rm -rf "$tmp_dir"' EXIT

new_repo() {
    rm -rf "$tmp_dir/repo"
    mkdir -p "$tmp_dir/repo/src"
    git -C "$tmp_dir/repo" init -q
}

echo
echo "planning-references.sh (code never cites a planning document)"
echo

if [ -f "$GUARD" ]; then
    ok "scripts/ci/planning-references.sh exists"
else
    bad "scripts/ci/planning-references.sh exists" "not found: $GUARD"
    printf '\n\033[31m%s\033[0m\n\n' "the guard is missing — nothing else can be tested"
    exit 1
fi

new_repo
printf 'fn main() {}\n// The printer answers in under a second.\n' > "$tmp_dir/repo/src/main.rs"
git -C "$tmp_dir/repo" add -A
run "$tmp_dir/repo"
expect_status "a repo whose code cites nothing → exit 0" 0

new_repo
{
    printf '// Voice and video calls (Plan %s66, M6)\n' "$section"
    printf '// the transport table (%s2.7)\n' "$section"
    printf '// see %s 3 of the contract\n' "$section"
    printf '// as PLAN-HUB %s84 says\n' "$section"
    printf '// as PLAN.md %s says\n' "$section"
    printf '// the system voice (round %s-11-04a)\n' "$year"
    printf '// Ronda %s-11-10 moved it\n' "$year"
    printf '// t%s S3 of the tenancy\n' "$slice"
    printf '// T%s 2 brought it\n' "$slice"
} > "$tmp_dir/repo/src/lib.rs"
git -C "$tmp_dir/repo" add -A
run "$tmp_dir/repo"
expect_status "a file that cites planning documents → exit 1" 1
expect_says "…naming the plan section with its line" "src/lib.rs:1:"
expect_says "…a bare section number" "src/lib.rs:2:"
expect_says "…a section number after a space" "src/lib.rs:3:"
expect_says "…a named plan's section" "src/lib.rs:4:"
expect_says "…a plan file's section" "src/lib.rs:5:"
expect_says "…a dated round" "src/lib.rs:6:"
expect_says "…a dated round written in Spanish" "src/lib.rs:7:"
expect_says "…a numbered slice" "src/lib.rs:8:"
expect_says "…a numbered slice with a capital letter" "src/lib.rs:9:"
expect_says "…as a CI annotation on the line" "::error file=src/lib.rs,line=1::"

new_repo
for ext in kt kts toml yml xml ps1 sh; do
    printf 'x (round %s-11-04a)\n' "$year" > "$tmp_dir/repo/src/file.$ext"
done
git -C "$tmp_dir/repo" add -A
run "$tmp_dir/repo"
expect_status "a citation in Kotlin, Gradle, TOML, YAML, XML, PowerShell or shell → exit 1" 1
for ext in kt kts toml yml xml ps1 sh; do
    expect_says "…the .$ext file is named" "src/file.$ext:1:"
done

new_repo
printf '# Notes\nThe transports table is in %s2.7.\n' "$section" > "$tmp_dir/repo/README.md"
printf '%s2.7\n' "$section" > "$tmp_dir/repo/src/docs.md"
printf 'fn main() {}\n' > "$tmp_dir/repo/src/main.rs"
git -C "$tmp_dir/repo" add -A
printf '// see %s2.7\n' "$section" > "$tmp_dir/repo/src/untracked.rs"
run "$tmp_dir/repo"
expect_status "Markdown documents and untracked files are not code under review → exit 0" 0
expect_silent_about "…and the Markdown file is not named" "README.md"
expect_silent_about "…nor the untracked file" "untracked.rs"

new_repo
{
    printf '// the system voice (%s-0204)\n' "$record"
    printf '// the scope of the request (%s#758)\n' hub
    printf '// see ERPlora/%s#1718 for the reviewer\n' hub
    printf '// mark it as a review account (%s#132)\n' saas
    printf '// booked in %s#145\n' pm
    printf '// the board (%s#12)\n' knowledge
    printf '// the apt lock (%s#313)\n' infra
    printf '// listening with the screen off (HUB_APP-F%s)\n' 26
    printf '// dictating to the assistant (BASE-F%s)\n' 94
    printf '// the task that brought it (FIX-%s)\n' 1883
    printf '// guarded by `%s/src-tauri/tests/x.rs`\n' "$old_tree"
} > "$tmp_dir/repo/src/refs.rs"
git -C "$tmp_dir/repo" add -A
run "$tmp_dir/repo"
expect_status "a file that cites records, other repos' issues, flow ids or old paths → exit 1" 1
expect_says "…a decision record" "src/refs.rs:1:"
expect_says "…an issue of another repo" "src/refs.rs:2:"
expect_says "…an issue of another repo named with its owner" "src/refs.rs:3:"
expect_says "…an issue of the SaaS" "src/refs.rs:4:"
expect_says "…an issue of the old board" "src/refs.rs:5:"
expect_says "…an issue of the knowledge base" "src/refs.rs:6:"
expect_says "…an issue of the infrastructure repo" "src/refs.rs:7:"
expect_says "…a flow id" "src/refs.rs:8:"
expect_says "…a flow id of the platform" "src/refs.rs:9:"
expect_says "…a task id" "src/refs.rs:10:"
expect_says "…a path of the old monorepo" "src/refs.rs:11:"

new_repo
{
    printf '// the plugin never sets sourceJson (tauri-apps/plugins-workspace#%s)\n' 3550
    printf '// pending channels are declared since app#%s\n' 7
    printf 'assert!(parse_print_target("usb:star#%s").is_err());\n' 1
    printf '// HUB_APP is the app of the hub, and %s-like names are not ids\n' "$record"
    printf '        uses: tauri-%s/tauri-action@84b9d35b # v0.6.2\n' apps
} > "$tmp_dir/repo/src/fine.rs"
git -C "$tmp_dir/repo" add -A
run "$tmp_dir/repo"
expect_status "an upstream issue, an issue of this repo and look-alike data → exit 0" 0

new_repo
mkdir -p "$tmp_dir/repo/.github/workflows"
{
    printf '          CHANNELS: |\n'
    printf '            store|Microsoft Store|x|MICROSOFT_STORE_PRODUCT_ID|true|%s#392\n' hub
    printf '            play|Google Play|x|PLAY_PACKAGE_NAME|true|%s#308\n' hub
    printf '            s3|S3 downloads (latest.json)|x|AWS_DOWNLOADS_ROLE_ARN|true|%s#400\n' hub
} > "$tmp_dir/repo/.github/workflows/tauri-release.yml"
mkdir -p "$tmp_dir/repo/scripts"
{
    printf 'store|Microsoft Store|$store|MICROSOFT_STORE_PRODUCT_ID|$store_var|%s#392\n' hub
    printf 's3|Object Storage (latest.json)|$s3|||%s#400\n' hub
    printf 'expect_says  "…naming where the decision lives" "%s#392"\n' hub
} > "$tmp_dir/repo/scripts/release-gate.test.sh"
printf '{\n  "description": "thin shell (%s#878)"\n}\n' hub > "$tmp_dir/repo/package.json"
git -C "$tmp_dir/repo" add -A
run "$tmp_dir/repo"
expect_status "the release gate's channel rows, their test fixtures and package.json's description are data → exit 0" 0

printf '      # the release gate (%s#895)\n' hub >> "$tmp_dir/repo/.github/workflows/tauri-release.yml"
printf '// store|Microsoft Store| row (%s#392)\n' hub > "$tmp_dir/repo/src/copy.rs"
git -C "$tmp_dir/repo" add -A
run "$tmp_dir/repo"
expect_status "…but a comment in the same workflow, or the same row elsewhere, is still caught → exit 1" 1
expect_says "…the comment in the workflow" ".github/workflows/tauri-release.yml:5:"
expect_says "…the copied row" "src/copy.rs:1:"
expect_silent_about "…and not the exempt rows" "tauri-release.yml:2:"

new_repo
printf '// IPP printer-state, RFC 8011 section 5.4.11\n' > "$tmp_dir/repo/src/ipp.rs"
git -C "$tmp_dir/repo" add -A
run "$tmp_dir/repo"
expect_status "an external standard cited as «section 5.4.11» → exit 0" 0

mkdir -p "$tmp_dir/not-a-repo"
run "$tmp_dir/not-a-repo"
expect_status "a folder that is not a git checkout → exit 2, never a silent pass" 2

run "$repo_root"
expect_status "this repo's own tracked code cites no planning document" 0

echo
if [ "$fail" -eq 0 ]; then
    printf '\033[32m%d passed\033[0m\n\n' "$pass"
    exit 0
fi
printf '\033[31m%d failed\033[0m, %d passed\n\n' "$fail" "$pass"
exit 1
