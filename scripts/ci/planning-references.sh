#!/usr/bin/env bash
# Fails when a tracked file other than a Markdown document cites a planning document: a section
# sign before a number, a dated decision round or a numbered slice. The code is read by people who
# have never seen those documents, so a comment has to say what the code does and why on its own.
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

matches=$(git -C "$top" grep -nIE -e "$pattern" -- ':!*.md')
status=$?
if [ "$status" -eq 1 ]; then
    echo "no tracked code cites a planning document"
    exit 0
fi
if [ "$status" -ne 0 ]; then
    echo "::error::git grep failed (exit $status)" >&2
    exit 2
fi

while IFS= read -r match; do
    file=${match%%:*}
    rest=${match#*:}
    line=${rest%%:*}
    echo "::error file=$file,line=$line::cites a planning document the reader does not have"
    echo "$match"
done <<< "$matches"

cat >&2 <<'HINT'

Say what the code does and why in plain sentences that stand on their own. Plan sections,
decision rounds and slices belong in the commit message or the pull request. An external
standard is cited by name and «section 5.4», without the sign.
HINT
exit 1
