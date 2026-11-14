#!/usr/bin/env bash
# Tests for scripts/ci/file-size.sh, which keeps every tracked code file under its size cap:
# production 1000 lines, tests 1500. In a Rust source file the code outside its `#[cfg(test)]`
# blocks is production (cap 1000) and the whole file is capped at 1500, and a module's `tests.rs`
# is a test file. Files over their cap today
# are listed in `.file-size-allow` with a ceiling, and that list can only shrink against the commit
# where the branch leaves `origin/main` (their merge-base).
#
# Run:  bash scripts/tests/file-size.test.sh
set -uo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
GUARD="$repo_root/scripts/ci/file-size.sh"

pass=0
fail=0
ok()  { printf '  \033[32m✓\033[0m %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  \033[31m✗\033[0m %s\n     %s\n' "$1" "$2"; fail=$((fail + 1)); }

run() { out=$(bash "$GUARD" "$@" 2>&1); status=$?; }
expect_status() { [ "$status" -eq "$2" ] && ok "$1" || bad "$1" "exit $status (wanted $2): $out"; }
expect_says()   { case "$out" in *"$2"*) ok "$1";; *) bad "$1" "never mentions '$2'. It said: $out";; esac; }
expect_silent_about() { case "$out" in *"$2"*) bad "$1" "mentions '$2': $out";; *) ok "$1";; esac; }

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/erplora-file-size-test.XXXXXX")
trap 'rm -rf "$tmp_dir"' EXIT
repo="$tmp_dir/repo"

# A fixture repo whose `origin/main` is its first commit, with the given allow-list (or none).
new_repo() {
    rm -rf "$repo"
    mkdir -p "$repo"
    git -C "$repo" init -q
    git -C "$repo" config user.email test@example.com
    git -C "$repo" config user.name test
    if [ $# -gt 0 ]; then printf '%s\n' "$@" > "$repo/.file-size-allow"; fi
    printf 'fn main() {}\n' > "$repo/main.rs"
    git -C "$repo" add -A
    git -C "$repo" commit -qm base
    git -C "$repo" update-ref refs/remotes/origin/main HEAD
}

# lines <count> <path>: a tracked file of exactly <count> lines.
lines() {
    mkdir -p "$(dirname "$repo/$2")"
    awk -v n="$1" 'BEGIN { for (i = 1; i <= n; i++) print "x" }' > "$repo/$2"
    git -C "$repo" add -A
}

# rust_file <path> <before> <test block> <after>: production lines, then a `#[cfg(test)]` module of
# <test block> lines (attribute and braces included), then more production lines.
rust_file() {
    mkdir -p "$(dirname "$repo/$1")"
    {
        awk -v n="$2" 'BEGIN { for (i = 1; i <= n; i++) print "fn p" i "() {}" }'
        printf '#[cfg(test)]\nmod tests {\n'
        awk -v n="$(($3 - 3))" 'BEGIN { for (i = 1; i <= n; i++) print "    // test line" }'
        printf '}\n'
        awk -v n="$4" 'BEGIN { for (i = 1; i <= n; i++) print "fn q" i "() {}" }'
    } > "$repo/$1"
    git -C "$repo" add -A
}

# allow <line>...: replaces the working allow-list (the branch's), leaving origin/main's as it was.
allow() { printf '%s\n' "$@" > "$repo/.file-size-allow"; git -C "$repo" add -A; }

# commit_base <message>: commits what is staged and makes it origin/main as well.
commit_base() {
    git -C "$repo" commit -qm "$1"
    git -C "$repo" update-ref refs/remotes/origin/main HEAD
}

# main_moves_on: what is staged becomes a new commit of origin/main on top of HEAD, while HEAD and
# the working tree stay where they were (a release of an older commit, or a branch cut before it).
main_moves_on() {
    local tree commit
    git -C "$repo" add -A
    tree=$(git -C "$repo" write-tree)
    commit=$(git -C "$repo" commit-tree "$tree" -p HEAD -m "main moved on")
    git -C "$repo" update-ref refs/remotes/origin/main "$commit"
    git -C "$repo" read-tree -u --reset HEAD
}

echo
echo "file-size.sh (no code file over its size cap; the exceptions only shrink)"
echo

if [ -f "$GUARD" ]; then
    ok "scripts/ci/file-size.sh exists"
else
    bad "scripts/ci/file-size.sh exists" "not found: $GUARD"
    printf '\n\033[31m%s\033[0m\n\n' "the guard is missing — nothing else can be tested"
    exit 1
fi

# ── Caps of files that are not listed ─────────────────────────────────────────────────────────
new_repo
lines 1000 src/ok.sh
lines 1500 scripts/tests/ok.test.sh
lines 1500 android/src/test/java/OkTest.kt
lines 1500 src-tauri/tests/ok.rs
run "$repo"
expect_status "production at 1000 lines and tests at 1500 → exit 0" 0

lines 1001 src/big.sh
run "$repo"
expect_status "a production file of 1001 lines → exit 1" 1
expect_says "…naming the file, its size and its cap" "src/big.sh: 1001 lines, over the production cap of 1000"
expect_says "…as a CI annotation" "::error file=src/big.sh::"

new_repo
lines 1501 scripts/tests/big.test.sh
lines 1501 android/src/test/java/BigTest.kt
lines 1501 src-tauri/tests/big.rs
run "$repo"
expect_status "test files of 1501 lines → exit 1" 1
for f in scripts/tests/big.test.sh android/src/test/java/BigTest.kt src-tauri/tests/big.rs; do
    expect_says "…$f is over the tests cap" "$f: 1501 lines, over the tests cap of 1500"
done

exts="rs kt kts java gradle ts tsx js jsx mjs cjs sh html css py ps1 yml"
new_repo
for ext in $exts; do lines 1001 "src/file.$ext"; done
run "$repo"
expect_status "every code extension is measured → exit 1" 1
for ext in $exts; do expect_says "…the .$ext file" "src/file.$ext: 1001"; done

new_repo
lines 3000 src-tauri/gen/android/app/Big.kt
lines 3000 docs/BIG.md
lines 3000 Cargo.lock
mkdir -p "$repo/src"
awk 'BEGIN { for (i = 1; i <= 3000; i++) print "x" }' > "$repo/src/untracked.sh"
run "$repo"
expect_status "generated code, documents, lockfiles and untracked files are not measured → exit 0" 0
expect_silent_about "…the generated file is not named" "Big.kt"

# ── Rust: production outside the test blocks, the whole file apart ─────────────────────────────
new_repo
rust_file src/lib.rs 1000 499 0
run "$repo"
expect_status "Rust: 1000 production lines and a 499-line test block (1499 in all) → exit 0" 0

rust_file src/lib.rs 1001 400 0
run "$repo"
expect_status "Rust: 1001 production lines before the test block → exit 1" 1
expect_says "…the production part is named" "src/lib.rs: 1001 production lines, over the production cap of 1000"

rust_file src/lib.rs 600 300 401
run "$repo"
expect_status "Rust: production code AFTER the test block counts too (600 + 401) → exit 1" 1
expect_says "…1001 production lines" "src/lib.rs: 1001 production lines"

rust_file src/lib.rs 900 601 0
run "$repo"
expect_status "Rust: 900 production lines but 1501 in all → exit 1" 1
expect_says "…the whole file is named" "src/lib.rs: 1501 lines in all, over the whole-file cap of 1500"

# A module's tests moved out to their own file (`#[cfg(test)] mod tests;` in the module) carry no
# `#[cfg(test)]` inside: only the file's name says they are tests.
new_repo
lines 1500 src/tests.rs
lines 1500 src/escpos/tests.rs
run "$repo"
expect_status "Rust: a module's tests.rs of 1500 lines is a test file → exit 0" 0

lines 1501 src/escpos/tests.rs
run "$repo"
expect_status "Rust: a tests.rs of 1501 lines → exit 1" 1
expect_says "…over the tests cap" "src/escpos/tests.rs: 1501 lines, over the tests cap of 1500"

new_repo
lines 1001 src/contests.rs
lines 1001 src/tests.rs.sh
run "$repo"
expect_status "a name that only contains tests.rs is still production → exit 1" 1
expect_says "…contests.rs keeps the production cap" "src/contests.rs: 1001 production lines, over the production cap of 1000"
expect_says "…tests.rs.sh keeps the production cap" "src/tests.rs.sh: 1001 lines, over the production cap of 1000"

# ── Listed files ──────────────────────────────────────────────────────────────────────────────
new_repo '2000 src/big.sh app#1' '1600/4300 src/escpos.rs app#2' '-/2300 src/plugin.rs app#3'
lines 1950 src/big.sh
rust_file src/escpos.rs 1409 2697 96
rust_file src/plugin.rs 906 1308 0
run "$repo"
expect_status "listed files within their ceilings → exit 0" 0

lines 2001 src/big.sh
run "$repo"
expect_status "a listed file over its ceiling → exit 1" 1
expect_says "…naming file, size and ceiling" "src/big.sh: 2001 lines, over its ceiling of 2000"

lines 1950 src/big.sh
rust_file src/escpos.rs 1520 2697 96
run "$repo"
expect_status "a listed Rust file whose production part passes its ceiling → exit 1" 1
expect_says "…naming the production ceiling" "src/escpos.rs: 1616 production lines, over its ceiling of 1600"

rust_file src/escpos.rs 1409 2697 96
rust_file src/plugin.rs 1001 1200 0
run "$repo"
expect_status "a listed Rust file with production «-» whose production passes 1000 → exit 1" 1
expect_says "…the cap applies to that part" "src/plugin.rs: 1001 production lines, over the production cap of 1000"

new_repo '1100 src/small.sh app#1'
lines 900 src/small.sh
run "$repo"
expect_status "a listed file that already fits under its cap → exit 1" 1
expect_says "…asking to remove it" "src/small.sh: 900 lines fit under the cap of 1000: remove it from .file-size-allow"

new_repo '1600/2300 src/plugin.rs app#1'
rust_file src/plugin.rs 906 1308 0
run "$repo"
expect_status "a listed Rust production ceiling whose part already fits → exit 1" 1
expect_says "…asking to write - for that part" "src/plugin.rs: 906 production lines fit under the cap of 1000: write - for that part"

new_repo '1200 src/gone.sh app#1'
run "$repo"
expect_status "a listed file that does not exist → exit 1" 1
expect_says "…asking to remove the line" "src/gone.sh: listed in .file-size-allow but not tracked: remove the line"

new_repo '1700 src/big.sh app#1'
lines 1550 src/big.sh
run "$repo"
expect_status "a ceiling more than the size rounded up to the hundred → exit 1" 1
expect_says "…asking to lower it" "src/big.sh: ceiling 1700 is loose for 1550 lines: lower it to 1600"

# ── The list only shrinks against origin/main ─────────────────────────────────────────────────
new_repo '2000 src/big.sh app#1'
lines 1950 src/big.sh
lines 1150 src/other.sh
allow '2000 src/big.sh app#1' '1200 src/other.sh app#9'
run "$repo"
expect_status "a path that origin/main's list does not have → exit 1" 1
expect_says "…as a new exception" "src/other.sh: new exception, not in the .file-size-allow of origin/main@"

new_repo '2000 src/big.sh app#1' '-/2300 src/plugin.rs app#3'
lines 2050 src/big.sh
rust_file src/plugin.rs 1050 1200 0
allow '2100 src/big.sh app#1' '1100/2300 src/plugin.rs app#3'
run "$repo"
expect_status "a raised ceiling, or a Rust part going from - to a number → exit 1" 1
expect_says "…the raised ceiling" "src/big.sh: raises its ceiling from 2000 to 2100"
expect_says "…the Rust part" "src/plugin.rs: raises its ceiling from -/2300 to 1100/2300"

new_repo '2000 src/big.sh app#1' '1200 src/old.sh app#2'
lines 1850 src/big.sh
allow '1900 src/big.sh app#1'
run "$repo"
expect_status "a lowered ceiling and a removed line → exit 0" 0

run "$repo" refs/remotes/upstream/main
expect_status "a base that does not exist → exit 2, never a silent pass" 2
expect_says "…saying how to get it" "git fetch"

new_repo
lines 1150 src/big.sh
allow '1200 src/big.sh app#1'
run "$repo"
expect_status "origin/main without an allow-list (the list's first commit) → exit 0" 0
expect_says "…and it says so" "has no .file-size-allow: the ratchet starts with this list"

# ── The base is where the branch leaves origin/main, not its tip ─────────────────────────────
new_repo '2000 src/big.sh app#1'
lines 1950 src/big.sh
commit_base "big file"
lines 1850 src/big.sh
allow '1900 src/big.sh app#1'
main_moves_on
run "$repo"
expect_status "an older commit, after main lowered a ceiling (a release of a tag) → exit 0" 0

lines 10 src/other.sh
git -C "$repo" commit -qm "branch work"
run "$repo"
expect_status "a branch cut before main lowered a ceiling, which keeps the old one → exit 0" 0

lines 2050 src/big.sh
allow '2100 src/big.sh app#1'
run "$repo"
expect_status "…but raising the ceiling it branched from is still caught → exit 1" 1
expect_says "…against that commit" "src/big.sh: raises its ceiling from 2000 to 2100"

new_repo '1200 src/big.sh app#1'
lines 1150 src/big.sh
git -C "$repo" update-ref refs/remotes/origin/main \
    "$(git -C "$repo" commit-tree "$(git -C "$repo" write-tree)" -m "unrelated history")"
run "$repo"
expect_status "origin/main with no history in common with HEAD → exit 2, never a silent pass" 2
expect_says "…asking for the history" "fetch the whole history"

# ── Malformed lines and odd places ────────────────────────────────────────────────────────────
for line in '1200 src/big.sh' '12a0 src/big.sh app#1' '1200/1600 src/big.sh app#1' '1600 src/lib.rs app#1' \
    '-/- src/lib.rs app#1' '1200 src/big.sh app#1 extra'; do
    new_repo
    lines 1150 src/big.sh
    rust_file src/lib.rs 1100 400 0
    allow "$line"
    git -C "$repo" commit -qm list
    git -C "$repo" update-ref refs/remotes/origin/main HEAD
    run "$repo"
    expect_status "a malformed line («${line}») → exit 2" 2
done

new_repo '# comment' '' '1200 src/big.sh app#1'
lines 1150 src/big.sh
run "$repo"
expect_status "comments and blank lines in the list are ignored → exit 0" 0

mkdir -p "$tmp_dir/not-a-repo"
run "$tmp_dir/not-a-repo"
expect_status "a folder that is not a git checkout → exit 2" 2

run "$repo_root"
expect_status "this repo's own code is within its caps and its list" 0

echo
if [ "$fail" -eq 0 ]; then
    printf '\033[32m%d passed\033[0m\n\n' "$pass"
    exit 0
fi
printf '\033[31m%d failed\033[0m, %d passed\n\n' "$fail" "$pass"
exit 1
