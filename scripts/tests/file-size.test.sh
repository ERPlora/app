#!/usr/bin/env bash
# Tests for scripts/ci/file-size.py, which keeps every tracked code file under the size cap of its
# language. Files over their cap are listed in `.file-size-allow` with their exact size, and that
# list only shrinks against the commit where the branch leaves `origin/main` (their merge-base).
#
# Run:  bash scripts/tests/file-size.test.sh
set -uo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
GUARD="$repo_root/scripts/ci/file-size.py"

pass=0
fail=0
ok()  { printf '  \033[32m✓\033[0m %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  \033[31m✗\033[0m %s\n     %s\n' "$1" "$2"; fail=$((fail + 1)); }

run() { out=$(python3 "$GUARD" "$@" 2>&1); status=$?; }
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

# cap_case <path> <cap> <kind>: an unlisted file at its cap passes, and one line more fails naming
# the file, its size and the cap of that kind ("production" or "tests").
cap_case() {
    new_repo
    lines "$2" "$1"
    run "$repo"
    expect_status "$1 at $2 lines → exit 0" 0
    lines $(($2 + 1)) "$1"
    run "$repo"
    expect_status "$1 at $(($2 + 1)) lines → exit 1" 1
    expect_says "…over the $3 cap of $2" "$1: $(($2 + 1)) lines, over the $3 cap of $2"
}

echo
echo "file-size.py (no code file over the cap of its language; the exceptions only shrink)"
echo

if [ -f "$GUARD" ]; then
    ok "scripts/ci/file-size.py exists"
else
    bad "scripts/ci/file-size.py exists" "not found: $GUARD"
    printf '\n\033[31m%s\033[0m\n\n' "the guard is missing — nothing else can be tested"
    exit 1
fi

# ── The cap of each language, for files that are not listed ──────────────────────────────────
cap_case crates/x/android/src/main/java/Plugin.kt 500 production
cap_case crates/x/android/build.gradle.kts 500 production
cap_case crates/x/android/src/test/java/PluginTest.kt 1000 tests
cap_case crates/x/android-tests/settings.gradle.kts 1000 tests
cap_case scripts/ci/preflight.py 500 production
cap_case scripts/tests/preflight.py 1000 tests
cap_case src/tests.rs 1000 tests
cap_case src/escpos/tests.rs 1000 tests
cap_case src-tauri/tests/shell_surface.rs 1000 tests
cap_case scripts/release.sh 100 production
cap_case scripts/tests/release.test.sh 1500 tests
cap_case ui/page.ts 300 production
cap_case ui/page.test.ts 500 tests
cap_case shell-dist/index.html 1000 production
cap_case .github/workflows/release.yml 1000 production
cap_case scripts/pack-msix.ps1 1000 production
cap_case src-tauri/tests/fixtures/receipt.html 1500 tests

new_repo
lines 101 scripts/deploy.sh
run "$repo"
expect_says "a shell script past its cap is told to be written in Python" "written in Python"

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
rust_file src/lib.rs 500 500 0
run "$repo"
expect_status "Rust: 500 production lines and a 500-line test block (1000 in all) → exit 0" 0

rust_file src/lib.rs 501 400 0
run "$repo"
expect_status "Rust: 501 production lines before the test block → exit 1" 1
expect_says "…the production part is named" "src/lib.rs: 501 production lines, over the production cap of 500"

rust_file src/lib.rs 300 300 201
run "$repo"
expect_status "Rust: production code AFTER the test block counts too (300 + 201) → exit 1" 1
expect_says "…501 production lines" "src/lib.rs: 501 production lines"

rust_file src/lib.rs 400 601 0
run "$repo"
expect_status "Rust: 400 production lines but 1001 in all (its tests go to tests.rs) → exit 1" 1
expect_says "…the whole file is named" "src/lib.rs: 1001 lines in all, over the whole-file cap of 1000"

new_repo
lines 501 src/contests.rs
lines 101 src/tests.rs.sh
run "$repo"
expect_status "a name that only contains tests.rs is still production → exit 1" 1
expect_says "…contests.rs keeps the production cap" "src/contests.rs: 501 production lines, over the production cap of 500"
expect_says "…tests.rs.sh keeps the production cap" "src/tests.rs.sh: 101 lines, over the production cap of 100"

# ── Listed files: the ceiling is the file's exact size ────────────────────────────────────────
new_repo '1950 src/big.sh app#1' '1505/4202 src/escpos.rs app#2' '-/2214 src/plugin.rs app#3'
lines 1950 src/big.sh
rust_file src/escpos.rs 1409 2697 96
rust_file src/plugin.rs 406 1808 0
run "$repo"
expect_status "listed files at their exact ceilings → exit 0" 0

lines 1951 src/big.sh
run "$repo"
expect_status "a listed file one line over its ceiling → exit 1" 1
expect_says "…naming file, size and ceiling" "src/big.sh: 1951 lines, over its ceiling of 1950"

lines 1950 src/big.sh
rust_file src/escpos.rs 1410 2696 96
run "$repo"
expect_status "a listed Rust file whose production part passes its ceiling → exit 1" 1
expect_says "…naming the production ceiling" "src/escpos.rs: 1506 production lines, over its ceiling of 1505"

rust_file src/escpos.rs 1409 2697 96
rust_file src/plugin.rs 501 1713 0
run "$repo"
expect_status "a listed Rust file with production «-» whose production passes 500 → exit 1" 1
expect_says "…the cap applies to that part" "src/plugin.rs: 501 production lines, over the production cap of 500"

new_repo '1700 src/big.sh app#1' '800/1800 src/usb.rs app#2'
lines 1550 src/big.sh
rust_file src/usb.rs 744 973 0
run "$repo"
expect_status "a ceiling above the file's size → exit 1" 1
expect_says "…asking to lower it to the exact size" "src/big.sh: ceiling 1700 is loose for 1550 lines: lower it to 1550"
expect_says "…for each Rust part" "src/usb.rs: ceiling 800 is loose for 744 production lines: lower it to 744"
expect_says "…the whole Rust file too" "src/usb.rs: ceiling 1800 is loose for 1717 lines in all: lower it to 1717"

new_repo '600 src/small.sh app#1'
lines 100 src/small.sh
run "$repo"
expect_status "a listed file that already fits under its cap → exit 1" 1
expect_says "…asking to remove it" "src/small.sh: 100 lines fit under the cap of 100: remove it from .file-size-allow"

new_repo '906/2214 src/plugin.rs app#1'
rust_file src/plugin.rs 406 1808 0
run "$repo"
expect_status "a listed Rust production ceiling whose part already fits → exit 1" 1
expect_says "…asking to write - for that part" "src/plugin.rs: 406 production lines fit under the cap of 500: write - for that part"

new_repo '600/1100 src/lib.rs app#1'
rust_file src/lib.rs 600 300 0
run "$repo"
expect_status "a listed Rust whole-file ceiling whose part already fits → exit 1" 1
expect_says "…asking to write - for that part" "src/lib.rs: 900 lines in all fit under the cap of 1000: write - for that part"

rust_file src/lib.rs 400 500 0
run "$repo"
expect_status "a listed Rust file whose two parts fit → exit 1" 1
expect_says "…asking to remove it" "src/lib.rs: 400 production lines and 900 in all fit under the caps: remove it from .file-size-allow"

new_repo '1200 src/gone.sh app#1'
run "$repo"
expect_status "a listed file that does not exist → exit 1" 1
expect_says "…asking to remove the line" "src/gone.sh: listed in .file-size-allow but not tracked: remove the line"

# ── The list only shrinks against origin/main ─────────────────────────────────────────────────
new_repo '1950 src/big.sh app#1'
lines 1950 src/big.sh
lines 150 src/other.sh
allow '1950 src/big.sh app#1' '150 src/other.sh app#9'
run "$repo"
expect_status "a path that origin/main's list does not have → exit 1" 1
expect_says "…as a new exception" "src/other.sh: new exception, not in the .file-size-allow of origin/main@"

new_repo '2000 src/big.sh app#1' '-/2300 src/plugin.rs app#3'
lines 2050 src/big.sh
rust_file src/plugin.rs 550 1750 0
allow '2050 src/big.sh app#1' '550/2300 src/plugin.rs app#3'
run "$repo"
expect_status "a raised ceiling, or a Rust part going from - to a number → exit 1" 1
expect_says "…the raised ceiling" "src/big.sh: raises its ceiling from 2000 to 2050"
expect_says "…the Rust part" "src/plugin.rs: raises its ceiling from -/2300 to 550/2300"

new_repo '2000 src/big.sh app#1' '1200 src/old.sh app#2'
lines 1850 src/big.sh
allow '1850 src/big.sh app#1'
run "$repo"
expect_status "a lowered ceiling and a removed line → exit 0" 0

run "$repo" refs/remotes/upstream/main
expect_status "a base that does not exist → exit 2, never a silent pass" 2
expect_says "…saying how to get it" "refs/remotes/upstream/main is not in this checkout: run git fetch origin main"

new_repo
lines 1150 src/big.sh
allow '1150 src/big.sh app#1'
run "$repo"
expect_status "origin/main without an allow-list (the list's first commit) → exit 0" 0
expect_says "…and it says so" "has no .file-size-allow: the ratchet starts with this list"

# ── When the caps come down, what main already had over them enters at its size there ────────
new_repo '-/1800 src/usb.rs app#46'
rust_file src/usb.rs 744 973 0
lines 650 src/Plugin.kt
lines 213 scripts/gate.sh
commit_base "files over today's caps, written under the old ones"
allow '744/1717 src/usb.rs app#46' '650 src/Plugin.kt app#9' '213 scripts/gate.sh app#9'
run "$repo"
expect_status "files main had over today's caps are listed at their size there → exit 0" 0

lines 651 src/Plugin.kt
rust_file src/usb.rs 745 973 0
allow '745/1718 src/usb.rs app#46' '651 src/Plugin.kt app#9' '213 scripts/gate.sh app#9'
run "$repo"
expect_status "…but never above that size → exit 1" 1
expect_says "…a file that grew in the branch" "src/Plugin.kt: raises its ceiling from 650 to 651"
expect_says "…a Rust part that grew in the branch" "src/usb.rs: raises its ceiling from 744/1800 to 745/1718"

new_repo '# the list'
lines 400 src/Small.kt
commit_base "a file under its cap"
lines 600 src/Small.kt
allow '600 src/Small.kt app#9'
run "$repo"
expect_status "a file main had under its cap never enters the list → exit 1" 1
expect_says "…as a new exception" "src/Small.kt: new exception, not in the .file-size-allow of origin/main@"

# ── A cap only comes down against origin/main ──────────────────────────────────────────────────
# A copy of this checker at origin/main with the given caps changed (sed expressions), and the
# branch bringing this checker as it is.
checker_on_main() {
    mkdir -p "$repo/scripts/ci"
    sed "$@" "$GUARD" > "$repo/scripts/ci/file-size.py"
    git -C "$repo" add -A
    commit_base "the checker"
    cp "$GUARD" "$repo/scripts/ci/file-size.py"
    git -C "$repo" add -A
}

new_repo
checker_on_main -e 's/^KOTLIN_CAP = 500$/KOTLIN_CAP = 400/' -e 's/^RUST_WHOLE_CAP = 1000$/RUST_WHOLE_CAP = 900/'
run "$repo"
expect_status "a cap above the one origin/main's checker sets → exit 1" 1
expect_says "…naming the cap" "scripts/ci/file-size.py: KOTLIN_CAP 500 is over 400, the one on origin/main@"
expect_says "…every cap" "scripts/ci/file-size.py: RUST_WHOLE_CAP 1000 is over 900, the one on origin/main@"

new_repo
checker_on_main -e 's/^KOTLIN_CAP = 500$/KOTLIN_CAP = 600/'
run "$repo"
expect_status "a cap below the one origin/main's checker sets → exit 0" 0

new_repo
run "$repo"
expect_says "origin/main without this checker (its first commit) says so" "has no scripts/ci/file-size.py: the caps start with this one"

# ── The base is where the branch leaves origin/main, not its tip ─────────────────────────────
new_repo '2000 src/big.sh app#1'
lines 2000 src/big.sh
commit_base "big file"
lines 1850 src/big.sh
allow '1850 src/big.sh app#1'
main_moves_on
run "$repo"
expect_status "an older commit, after main lowered a ceiling (a release of a tag) → exit 0" 0

lines 10 src/other.sh
git -C "$repo" commit -qm "branch work"
run "$repo"
expect_status "a branch cut before main lowered a ceiling, which keeps the old one → exit 0" 0

lines 2050 src/big.sh
allow '2050 src/big.sh app#1'
run "$repo"
expect_status "…but raising the ceiling it branched from is still caught → exit 1" 1
expect_says "…against that commit" "src/big.sh: raises its ceiling from 2000 to 2050"

new_repo '-/1800 src/usb.rs app#46'
rust_file src/usb.rs 744 973 0
commit_base "a ceiling above the file's size, from when ceilings were rounded"
allow '744/1717 src/usb.rs app#46'
main_moves_on
rust_file src/usb.rs 744 1006 0
allow '744/1750 src/usb.rs app#46'
run "$repo"
expect_status "the ceilings are the list's where the branch leaves main, not at its tip → exit 0" 0

new_repo '# the list'
lines 400 src/Small.kt
commit_base "a file under its cap"
lines 600 src/Small.kt
main_moves_on
lines 600 src/Small.kt
allow '600 src/Small.kt app#9'
run "$repo"
expect_status "sizes are read where the branch leaves main, not at its tip → exit 1" 1
expect_says "…so the file is a new exception" "src/Small.kt: new exception"

new_repo '1150 src/big.sh app#1'
lines 1150 src/big.sh
git -C "$repo" update-ref refs/remotes/origin/main \
    "$(git -C "$repo" commit-tree "$(git -C "$repo" write-tree)" -m "unrelated history")"
run "$repo"
expect_status "origin/main with no history in common with HEAD → exit 2, never a silent pass" 2
expect_says "…asking for the history" "fetch the whole history"

# ── Malformed lines and odd places ────────────────────────────────────────────────────────────
for line in '1150 src/big.sh' '11a0 src/big.sh app#1' '1150/1600 src/big.sh app#1' '1500 src/lib.rs app#1' \
    '-/- src/lib.rs app#1' '1150 src/big.sh app#1 extra'; do
    new_repo
    lines 1150 src/big.sh
    rust_file src/lib.rs 1100 400 0
    allow "$line"
    git -C "$repo" commit -qm list
    git -C "$repo" update-ref refs/remotes/origin/main HEAD
    run "$repo"
    expect_status "a malformed line («${line}») → exit 2" 2
done

new_repo '1150 src/big.sh app#1' '1150 src/big.sh app#1'
lines 1150 src/big.sh
run "$repo"
expect_status "a path listed twice → exit 2" 2
expect_says "…naming the line" "the path is listed twice"

new_repo '# comment' '' '1150 src/big.sh app#1'
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
