#!/usr/bin/env bash
# Fails when a tracked file other than a Markdown document cites something the reader does not
# have: a plan section, a dated decision round, a numbered slice, a decision record, an issue of
# another repo or of a private tracker, a flow or task id, or a path of the old monorepo. The code
# is read by people who have never seen any of those, so a comment has to say what the code does
# and why on its own. File contents are checked, not file names.
#
# Usage: planning-references.sh [checkout]   (default: the current directory)
# Exit:  0 nothing cited · 1 citations found, one line each · 2 not a git checkout
set -uo pipefail

root=${1:-.}
if ! top=$(git -C "$root" rev-parse --show-toplevel 2>/dev/null); then
    echo "::error::$root is not a git checkout" >&2
    exit 2
fi

pattern='§ ?[0-9]|(Plan|PLAN)(-[A-Z]+)?(\.md)? ?§|[Rr](onda|ound) [0-9]{4}-[0-9]{2}|[Tt]ajada S?[0-9]'
pattern+='|ADR-[0-9]{3,4}'
pattern+='|(^|[^A-Za-z0-9_])(hub|saas|pm|knowledge|infra|bridge|aws)#[0-9]'
pattern+='|[A-Z][A-Z_]*-F[0-9]{2}|FIX-[0-9A-Z]'
pattern+='|(^|[^A-Za-z0-9_-])apps/[t]auri'

# Data, not prose: a match on one of these lines is not reported. Each entry is a file and a fixed
# piece of the line.
#   · The release gate's channel rows end with the issue a missing channel points at, after a `|`,
#     and `release-gate.sh` prints it in the run summary. Its test feeds it rows of the same shape
#     and checks that the reference is printed.
#   · `package.json` has no comments: its `description` is manifest data and stays as it is.
exempt_files=(
    .github/workflows/tauri-release.yml
    scripts/release-gate.test.sh
    scripts/release-gate.test.sh
    package.json
)
exempt_texts=(
    '|hub#'
    '|hub#'
    'naming where the decision lives" "hub#'
    '"description": '
)

is_exempt() {
    local i
    for i in "${!exempt_files[@]}"; do
        if [ "$1" = "${exempt_files[$i]}" ] && [[ "$2" == *"${exempt_texts[$i]}"* ]]; then
            return 0
        fi
    done
    return 1
}

matches=$(git -C "$top" grep -nIE -e "$pattern" -- ':!*.md')
status=$?
if [ "$status" -gt 1 ]; then
    echo "::error::git grep failed (exit $status)" >&2
    exit 2
fi

found=0
while IFS= read -r match; do
    [ -n "$match" ] || continue
    file=${match%%:*}
    rest=${match#*:}
    line=${rest%%:*}
    text=${rest#*:}
    is_exempt "$file" "$text" && continue
    echo "::error file=$file,line=$line::cites something the reader does not have"
    echo "$match"
    found=$((found + 1))
done <<< "$matches"

if [ "$found" -eq 0 ]; then
    echo "no tracked code cites a planning document, a decision record or another tracker"
    exit 0
fi

cat >&2 <<'HINT'

Say what the code does and why in plain sentences that stand on their own. Plan sections,
decision rounds, slices, decision records, other trackers' issues and flow ids belong in the
commit message or the pull request. An external standard is cited by name and «section 5.4»,
without the sign; an upstream issue keeps its owner/repo#N.
HINT
exit 1
