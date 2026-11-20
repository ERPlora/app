#!/usr/bin/env bash
# Tests for scripts/ci/ratchet.py, shared by the limit checks of scripts/ci: a check runs from the
# top of a git checkout and judges against the commit where HEAD leaves `main` (their merge-base);
# when it cannot get there it exits 2, never a silent pass.
#
# Run:  bash scripts/tests/ratchet.test.sh
set -uo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
MODULE_DIR="$repo_root/scripts/ci"

pass=0
fail=0
ok()  { printf '  \033[32m✓\033[0m %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  \033[31m✗\033[0m %s\n     %s\n' "$1" "$2"; fail=$((fail + 1)); }

# A check whose verdict is exit 0 and whose output is where it ran and the base it got.
CHECK='
import os, sys
sys.path.insert(0, sys.argv[1])
import ratchet

def judge(main):
    base = ratchet.merge_base(main, "the list")
    print(f"ran in {os.getcwd()} against {base}")
    return 0

sys.exit(ratchet.run_check(sys.argv[1:], judge))
'

run() { out=$(python3 -c "$CHECK" "$MODULE_DIR" "$@" 2>&1); status=$?; }
expect_status() { [ "$status" -eq "$2" ] && ok "$1" || bad "$1" "exit $status (wanted $2): $out"; }
expect_says()   { case "$out" in *"$2"*) ok "$1";; *) bad "$1" "never mentions '$2'. It said: $out";; esac; }

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/erplora-ratchet-test.XXXXXX")
trap 'rm -rf "$tmp_dir"' EXIT
repo="$tmp_dir/repo"

new_repo() {
    rm -rf "$repo"
    mkdir -p "$repo/sub"
    git -C "$repo" init -q
    git -C "$repo" config user.email test@example.com
    git -C "$repo" config user.name test
    printf 'a\n' > "$repo/sub/a.txt"
    git -C "$repo" add -A
    git -C "$repo" commit -qm base
    git -C "$repo" update-ref refs/remotes/origin/main HEAD
}

echo
echo "ratchet.py (where a limit check runs and the base its list only shrinks against)"
echo

if [ -f "$MODULE_DIR/ratchet.py" ]; then
    ok "scripts/ci/ratchet.py exists"
else
    bad "scripts/ci/ratchet.py exists" "not found: $MODULE_DIR/ratchet.py"
    printf '\n\033[31m%s\033[0m\n\n' "the module is missing — nothing else can be tested"
    exit 1
fi

new_repo
top=$(git -C "$repo" rev-parse --show-toplevel)
base=$(git -C "$repo" rev-parse HEAD)
run "$repo/sub"
expect_status "a checkout with origin/main → the check's own verdict" 0
expect_says "…judged from the top of the checkout, against the merge-base" "ran in $top against $base"

git -C "$repo" commit -q --allow-empty -m "branch work"
run "$repo"
expect_says "a branch ahead of main is judged against where it left main" "against $base"

new_repo
run "$repo" refs/remotes/upstream/main
expect_status "a base that does not exist → exit 2" 2
expect_says "…saying how to get it, and for what" "git fetch origin main (the list can only shrink against it)"

new_repo
git -C "$repo" update-ref refs/remotes/origin/main \
    "$(git -C "$repo" commit-tree "$(git -C "$repo" write-tree)" -m "unrelated history")"
run "$repo"
expect_status "origin/main with no history in common with HEAD → exit 2" 2
expect_says "…asking for the whole history" "fetch the whole history (git fetch --unshallow origin) so the list has a base"

mkdir -p "$tmp_dir/not-a-repo"
run "$tmp_dir/not-a-repo"
expect_status "a folder that is not a git checkout → exit 2" 2
expect_says "…naming it" "$tmp_dir/not-a-repo is not a git checkout"

echo
if [ "$fail" -eq 0 ]; then
    printf '\033[32m%d passed\033[0m\n\n' "$pass"
    exit 0
fi
printf '\033[31m%d failed\033[0m, %d passed\n\n' "$fail" "$pass"
exit 1
