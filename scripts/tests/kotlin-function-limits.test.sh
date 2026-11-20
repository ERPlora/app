#!/usr/bin/env bash
# Tests for scripts/ci/kotlin-function-limits.py, which keeps detekt's function and class limits of
# the Android plugin at their defaults and keeps the code excused from them from growing: the
# excuses live in detekt's baseline, never in a `@Suppress`, and the baseline's count of each rule
# does not grow against the commit where HEAD leaves `origin/main` (moving an excused function to
# another file is fine).
#
# Run:  bash scripts/tests/kotlin-function-limits.test.sh
set -uo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
GUARD="$repo_root/scripts/ci/kotlin-function-limits.py"

pass=0
fail=0
ok()  { printf '  \033[32m✓\033[0m %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  \033[31m✗\033[0m %s\n     %s\n' "$1" "$2"; fail=$((fail + 1)); }

run() { out=$(python3 "$GUARD" "$@" 2>&1); status=$?; }
expect_status() { [ "$status" -eq "$2" ] && ok "$1" || bad "$1" "exit $status (wanted $2): $out"; }
expect_says()   { case "$out" in *"$2"*) ok "$1";; *) bad "$1" "never mentions '$2'. It said: $out";; esac; }

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/erplora-kotlin-function-limits-test.XXXXXX")
trap 'rm -rf "$tmp_dir"' EXIT
repo="$tmp_dir/repo"

tests_root=crates/tauri-plugin-erplora-android/android-tests
config="$tests_root/detekt.yml"
baseline="$tests_root/detekt-baseline.xml"
source=crates/tauri-plugin-erplora-android/android/src/main/java/com/erplora/android/Plugin.kt

detekt_yml() {
    cat <<'YAML'
# Only the function and class limits run.
style:
  active: false
complexity:
  ComplexCondition:
    active: false
YAML
}

# baseline_xml <id>…: a detekt baseline whose current issues are those ids.
baseline_xml() {
    printf '<?xml version="1.0" ?>\n<SmellBaseline>\n  <ManuallySuppressedIssues/>\n  <CurrentIssues>\n'
    for id in "$@"; do printf '    <ID>%s</ID>\n' "$id"; done
    printf '  </CurrentIssues>\n</SmellBaseline>\n'
}

excused_class='TooManyFunctions:Plugin.kt$Plugin : Plugin'
excused_function='NestedBlockDepth:Plugin.kt$Plugin$@Command fun bondedPrinters(invoke: Invoke)'

# A fixture repo whose `origin/main` is its first commit: the config, a baseline with one excused
# class and one excused function, and a Kotlin source with no suppression.
new_repo() {
    rm -rf "$repo"
    mkdir -p "$repo/$tests_root" "$(dirname "$repo/$source")"
    git -C "$repo" init -q
    git -C "$repo" config user.email test@example.com
    git -C "$repo" config user.name test
    detekt_yml > "$repo/$config"
    baseline_xml "$excused_class" "$excused_function" > "$repo/$baseline"
    printf '@Suppress("UNCHECKED_CAST")\nclass Plugin\n' > "$repo/$source"
    git -C "$repo" add -A
    git -C "$repo" commit -qm base
    git -C "$repo" update-ref refs/remotes/origin/main HEAD
}

# write <path> <text>: replaces a file in the branch and stages it.
write() { mkdir -p "$(dirname "$repo/$1")"; printf '%s\n' "$2" > "$repo/$1"; git -C "$repo" add -A; }

# main_moves_on: what is staged becomes a new commit of origin/main on top of HEAD, while HEAD and
# the working tree stay where they were (a release of an older commit, or a branch cut before it).
main_moves_on() {
    local tree commit
    tree=$(git -C "$repo" write-tree)
    commit=$(git -C "$repo" commit-tree "$tree" -p HEAD -m "main moved on")
    git -C "$repo" update-ref refs/remotes/origin/main "$commit"
    git -C "$repo" read-tree -u --reset HEAD
}

echo
echo "kotlin-function-limits.py (detekt's limits stay at their defaults; the excused code only shrinks)"
echo

if [ -f "$GUARD" ]; then
    ok "scripts/ci/kotlin-function-limits.py exists"
else
    bad "scripts/ci/kotlin-function-limits.py exists" "not found: $GUARD"
    printf '\n\033[31m%s\033[0m\n\n' "the guard is missing — nothing else can be tested"
    exit 1
fi

new_repo
run "$repo"
expect_status "the config switching other rules off and the base's excuses kept → exit 0" 0

# ── The configuration ─────────────────────────────────────────────────────────────────────────
new_repo
write "$config" "$(detekt_yml; printf '  LongMethod:\n    threshold: 120\n')"
run "$repo"
expect_status "a raised threshold → exit 1" 1
expect_says "…naming the line" "$config:8: detekt.yml only switches rules off (active: false), never sets a value: threshold: 120"

new_repo
write "$config" "$(detekt_yml; printf 'build:\n  maxIssues: 5\n')"
run "$repo"
expect_status "findings tolerated before failing (maxIssues) → exit 1" 1
expect_says "…naming the line" "never sets a value: maxIssues: 5"

new_repo
write "$config" "$(detekt_yml; printf '  TooManyFunctions:\n    excludes: ["**/main/**"]\n')"
run "$repo"
expect_status "a limit that skips the production code (excludes) → exit 1" 1
expect_says "…naming the line" "never sets a value: excludes:"

new_repo
write "$config" "$(detekt_yml; printf '  LargeClass:\n    excludes:\n      - "**/main/**"\n')"
run "$repo"
expect_status "the same, written as a block list → exit 1" 1
expect_says "…naming the item" "never sets a value: - \"**/main/**\""

new_repo
write "$config" $'complexity:\n    ComplexCondition:\n        active: false\n    active: false'
run "$repo"
expect_status "the rule set switched off after a rule, indented by four → exit 1" 1
expect_says "…naming the rule set" "detekt.yml switches off complexity"

new_repo
write "$config" "$(detekt_yml; printf '  LongParameterList:\n    active: false\n')"
run "$repo"
expect_status "one of the limits switched off → exit 1" 1
expect_says "…naming the rule" "detekt.yml switches off LongParameterList, one of the function and class limits"

new_repo
write "$config" $'complexity:\n  active: false\n  ComplexCondition:\n    active: false'
run "$repo"
expect_status "the complexity rule set switched off, every limit with it → exit 1" 1
expect_says "…naming the rule set" "detekt.yml switches off complexity"

new_repo
write "$config" "$(detekt_yml; printf 'naming:\n  active: false\n')"
run "$repo"
expect_status "another rule set switched off → exit 0" 0

new_repo
git -C "$repo" rm -q "$config"
run "$repo"
expect_status "no detekt.yml → exit 1" 1
expect_says "…naming the file" "$config is missing"

# ── The suppressions ──────────────────────────────────────────────────────────────────────────
new_repo
write "$source" $'class Plugin {\n    @Suppress("LongMethod")\n    fun render() {}\n}'
run "$repo"
expect_status "a @Suppress of a limit → exit 1" 1
expect_says "…naming the file, the line and the rule" "$source:2: @Suppress silences LongMethod"

new_repo
write "$source" $'@file:Suppress("detekt:TooManyFunctions")\nclass Plugin'
run "$repo"
expect_status "a whole file suppressed, with detekt's prefix → exit 1" 1
expect_says "…naming the rule" "@Suppress silences TooManyFunctions"

new_repo
write "$source" $'class Plugin {\n    @Suppress(\n        "UNUSED_PARAMETER",\n        "complexity:LongParameterList",\n    )\n    fun render(a: Int) {}\n}'
run "$repo"
expect_status "a suppression written over several lines, with the rule set's prefix → exit 1" 1
expect_says "…at the annotation's line" "$source:2: @Suppress silences LongParameterList"

new_repo
write "$source" $'@SuppressWarnings("complexity")\nclass Plugin'
run "$repo"
expect_status "the whole complexity rule set suppressed → exit 1" 1
expect_says "…naming the rule set" "@Suppress silences complexity"

new_repo
write "$source" $'@Suppress("all")\nclass Plugin'
run "$repo"
expect_status "every rule suppressed at once → exit 1" 1

new_repo
write crates/other/build.gradle.kts '@file:Suppress("CyclomaticComplexMethod")'
run "$repo"
expect_status "a suppression in a Gradle script (.kts) → exit 1" 1

new_repo
write "$source" $'// @Suppress("LongMethod") is not how a long function is excused here.\nclass Plugin'
run "$repo"
expect_status "a comment that mentions @Suppress → exit 0" 0

# ── The baseline ──────────────────────────────────────────────────────────────────────────────
new_repo
write "$baseline" "$(baseline_xml "$excused_class" "$excused_function" 'NestedBlockDepth:Plugin.kt$Plugin$fun print()')"
run "$repo"
expect_status "a new excused function, the base's ones kept → exit 1" 1
expect_says "…counting per rule against the base" "NestedBlockDepth: 2 excused in $baseline, origin/main@"

new_repo
write "$baseline" "$(baseline_xml "$excused_class" "$excused_function" 'LongMethod:Plugin.kt$Plugin$fun print()')"
run "$repo"
expect_status "an excuse of a rule the base never excused → exit 1" 1
expect_says "…as a new exception" "LongMethod: 1 excused in $baseline, origin/main@"

new_repo
write "$baseline" "$(baseline_xml "$excused_class" "$excused_function" \
    | awk '/<ManuallySuppressedIssues\/>/ { print "  <ManuallySuppressedIssues>"; print "    <ID>LongMethod:Plugin.kt$Plugin$fun print()</ID>"; print "  </ManuallySuppressedIssues>"; next } { print }')"
run "$repo"
expect_status "an excuse added by hand to the manually suppressed issues → exit 1" 1
expect_says "…counted as well" "LongMethod: 1 excused in $baseline"

new_repo
write "$baseline" "$(baseline_xml "$excused_class" 'NestedBlockDepth:Printing.kt$Printing$@Command fun bondedPrinters(invoke: Invoke)')"
run "$repo"
expect_status "an excused function moved to another file (a file split) → exit 0" 0

new_repo
write "$baseline" "$(baseline_xml "$excused_class")"
run "$repo"
expect_status "an excuse removed → exit 0" 0

new_repo
git -C "$repo" rm -q "$baseline"
run "$repo"
expect_status "no baseline at all (nothing excused) → exit 0" 0

new_repo
write "$baseline" "$(baseline_xml "$excused_class")"
main_moves_on
run "$repo"
expect_status "an older commit that still has an excuse main removed later (a release of a tag) → exit 0" 0

new_repo
git -C "$repo" rm -q "$baseline"
git -C "$repo" commit -qm "no limits yet"
git -C "$repo" update-ref refs/remotes/origin/main HEAD
write "$baseline" "$(baseline_xml "$excused_class")"
run "$repo"
expect_status "origin/main without a baseline (the limits' first commit) → exit 0" 0
expect_says "…and it says so" "has no $baseline: the excused code starts here"

new_repo
git -C "$repo" update-ref refs/remotes/origin/main \
    "$(git -C "$repo" commit-tree "$(git -C "$repo" write-tree)" -m "unrelated history")"
run "$repo"
expect_status "origin/main with no history in common with HEAD → exit 2, never a silent pass" 2
expect_says "…asking for the history" "fetch the whole history"

new_repo
run "$repo" refs/remotes/upstream/main
expect_status "a base that does not exist → exit 2, never a silent pass" 2
expect_says "…saying how to get it" "git fetch"

mkdir -p "$tmp_dir/not-a-repo"
run "$tmp_dir/not-a-repo"
expect_status "a folder that is not a git checkout → exit 2" 2

run "$repo_root"
expect_status "this repo keeps detekt's limits at their defaults and its excuses within origin/main's" 0

echo
if [ "$fail" -eq 0 ]; then
    printf '\033[32m%d passed\033[0m\n\n' "$pass"
    exit 0
fi
printf '\033[31m%d failed\033[0m, %d passed\n\n' "$fail" "$pass"
exit 1
