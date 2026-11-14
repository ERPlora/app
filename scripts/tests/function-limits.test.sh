#!/usr/bin/env bash
# Tests for scripts/ci/function-limits.sh, which keeps clippy's function limits switched on (a
# function of at most 100 lines and 7 arguments) and keeps the functions excused from them from
# growing in number: an excuse is an `#[expect(clippy::…, reason = "…")]`, never an `allow`, and
# against `origin/main` no file gains one.
#
# Run:  bash scripts/tests/function-limits.test.sh
set -uo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
GUARD="$repo_root/scripts/ci/function-limits.sh"

pass=0
fail=0
ok()  { printf '  \033[32m✓\033[0m %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  \033[31m✗\033[0m %s\n     %s\n' "$1" "$2"; fail=$((fail + 1)); }

run() { out=$(bash "$GUARD" "$@" 2>&1); status=$?; }
expect_status() { [ "$status" -eq "$2" ] && ok "$1" || bad "$1" "exit $status (wanted $2): $out"; }
expect_says()   { case "$out" in *"$2"*) ok "$1";; *) bad "$1" "never mentions '$2'. It said: $out";; esac; }

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/erplora-function-limits-test.XXXXXX")
trap 'rm -rf "$tmp_dir"' EXIT
repo="$tmp_dir/repo"

cargo_toml() {
    cat <<'TOML'
[workspace]
members = ["a"]

[workspace.lints.clippy]
correctness = { level = "deny", priority = -1 }
too_many_lines = "deny"
too_many_arguments = "deny"

[workspace.lints.rust]
unfulfilled_lint_expectations = "deny"
TOML
}

clippy_toml() { printf 'too-many-lines-threshold = 100\ntoo-many-arguments-threshold = 7\n'; }

excused='#[expect(clippy::too_many_lines, reason = "renders every document type in one match")]'

# A fixture repo whose `origin/main` is its first commit: the limits configured and one excused
# function in a/src/lib.rs.
new_repo() {
    rm -rf "$repo"
    mkdir -p "$repo/a/src"
    git -C "$repo" init -q
    git -C "$repo" config user.email test@example.com
    git -C "$repo" config user.name test
    cargo_toml > "$repo/Cargo.toml"
    clippy_toml > "$repo/clippy.toml"
    printf '%s\nfn render() {}\n' "$excused" > "$repo/a/src/lib.rs"
    git -C "$repo" add -A
    git -C "$repo" commit -qm base
    git -C "$repo" update-ref refs/remotes/origin/main HEAD
}

# write <path> <text>: replaces a file in the branch and stages it.
write() { mkdir -p "$(dirname "$repo/$1")"; printf '%s\n' "$2" > "$repo/$1"; git -C "$repo" add -A; }

echo
echo "function-limits.sh (clippy's function limits stay on; the excused functions only shrink)"
echo

if [ -f "$GUARD" ]; then
    ok "scripts/ci/function-limits.sh exists"
else
    bad "scripts/ci/function-limits.sh exists" "not found: $GUARD"
    printf '\n\033[31m%s\033[0m\n\n' "the guard is missing — nothing else can be tested"
    exit 1
fi

new_repo
run "$repo"
expect_status "the limits configured and the base's excuse kept → exit 0" 0

# ── The configuration ─────────────────────────────────────────────────────────────────────────
new_repo
write clippy.toml $'too-many-lines-threshold = 150\ntoo-many-arguments-threshold = 7'
run "$repo"
expect_status "a raised line threshold → exit 1" 1
expect_says "…naming the setting" "clippy.toml must set too-many-lines-threshold = 100"

new_repo
write clippy.toml 'too-many-lines-threshold = 100'
run "$repo"
expect_status "a missing argument threshold → exit 1" 1
expect_says "…naming the setting" "clippy.toml must set too-many-arguments-threshold = 7"

new_repo
write Cargo.toml "$(cargo_toml | sed 's/too_many_lines = "deny"/too_many_lines = "warn"/')"
run "$repo"
expect_status "too_many_lines below deny → exit 1" 1
expect_says "…naming the lint" "Cargo.toml [workspace.lints.clippy] must deny too_many_lines"

new_repo
write Cargo.toml "$(cargo_toml | grep -v too_many_arguments)"
run "$repo"
expect_status "too_many_arguments missing → exit 1" 1
expect_says "…naming the lint" "Cargo.toml [workspace.lints.clippy] must deny too_many_arguments"

new_repo
write Cargo.toml "$(cargo_toml | grep -v unfulfilled)"
run "$repo"
expect_status "an excuse that is no longer needed would not fail → exit 1" 1
expect_says "…naming the lint" "Cargo.toml [workspace.lints.rust] must deny unfulfilled_lint_expectations"

new_repo
write Cargo.toml "$(cargo_toml | grep -v '^too_many_lines'; printf 'too_many_lines = "deny"\n')"
run "$repo"
expect_status "too_many_lines denied in the wrong table → exit 1" 1

new_repo
write a/clippy.toml 'too-many-lines-threshold = 300'
run "$repo"
expect_status "a crate's own clippy.toml, which would shadow the root one → exit 1" 1
expect_says "…naming it" "a/clippy.toml"

# ── The excuses ───────────────────────────────────────────────────────────────────────────────
new_repo
write a/src/lib.rs $'#[allow(clippy::too_many_lines)]\nfn render() {}'
run "$repo"
expect_status "an allow instead of an expect → exit 1" 1
expect_says "…asking for expect with a reason" "a/src/lib.rs:1: use #[expect(clippy::…, reason = \"…\")], not allow"

new_repo
write a/src/lib.rs $'#[allow(\n    clippy::too_many_lines\n)]\nfn render() {}'
run "$repo"
expect_status "an allow written over several lines → exit 1" 1

new_repo
write a/src/lib.rs $'#[expect(clippy::too_many_lines)]\nfn render() {}'
run "$repo"
expect_status "an expect without a reason → exit 1" 1
expect_says "…asking for the reason" "a/src/lib.rs:1: an excuse says why: reason = \"…\""

new_repo
write a/src/lib.rs $'#[expect(\n    clippy::too_many_lines,\n    reason = "renders every document type in one match"\n)]\nfn render() {}'
run "$repo"
expect_status "the same excuse rewrapped over several lines → exit 0" 0

new_repo
write a/src/other.rs "$excused"
run "$repo"
expect_status "a file that gains an excuse → exit 1" 1
expect_says "…as a new exception" "a/src/other.rs: 1 function-limit excuse(s), origin/main has 0"

new_repo
write a/src/lib.rs $'#[expect(clippy::too_many_arguments, reason = "the row has eight columns")]\nfn upsert() {}\n'"$excused"$'\nfn render() {}'
run "$repo"
expect_status "a second excuse in the same file → exit 1" 1

new_repo
write a/src/lib.rs $'#![expect(clippy::pedantic, reason = "generated bindings")]\nfn render() {}'
run "$repo"
expect_status "excusing a whole lint group in place of the base's excuse → exit 0 (same count)" 0
write a/src/other.rs '#[expect(clippy::complexity, reason = "x")]'
run "$repo"
expect_status "a new excuse of a group that holds a limit → exit 1" 1

new_repo
write a/src/lib.rs 'fn render() {}'
run "$repo"
expect_status "an excuse removed → exit 0" 0

git -C "$repo" commit -qm "no excuses left"
git -C "$repo" update-ref refs/remotes/origin/main HEAD
write a/src/lib.rs "$excused"
run "$repo"
expect_status "a base with no excuse at all, and the branch adds one → exit 1" 1
expect_says "…as a new exception" "a/src/lib.rs: 1 function-limit excuse(s), origin/main has 0"

new_repo
write a/Cargo.toml $'[package]\nname = "a"\n\n[lints]\nworkspace = true'
run "$repo"
expect_status "a crate that inherits the workspace lints → exit 0" 0
write a/Cargo.toml $'[package]\nname = "a"'
run "$repo"
expect_status "a crate that does not inherit them escapes the limits → exit 1" 1
expect_says "…naming it" "a/Cargo.toml must inherit the workspace lints"

run "$repo" refs/remotes/upstream/main
expect_status "a base that does not exist → exit 2, never a silent pass" 2
expect_says "…saying how to get it" "git fetch"

new_repo
git -C "$repo" rm -q clippy.toml
git -C "$repo" commit -qm "no limits yet"
git -C "$repo" update-ref refs/remotes/origin/main HEAD
clippy_toml > "$repo/clippy.toml"
write a/src/other.rs "$excused"
run "$repo"
expect_status "origin/main without clippy.toml (the limits' first commit) → exit 0" 0
expect_says "…and it says so" "origin/main has no clippy.toml"

mkdir -p "$tmp_dir/not-a-repo"
run "$tmp_dir/not-a-repo"
expect_status "a folder that is not a git checkout → exit 2" 2

run "$repo_root"
expect_status "this repo keeps the limits on and its excuses within origin/main's" 0

echo
if [ "$fail" -eq 0 ]; then
    printf '\033[32m%d passed\033[0m\n\n' "$pass"
    exit 0
fi
printf '\033[31m%d failed\033[0m, %d passed\n\n' "$fail" "$pass"
exit 1
