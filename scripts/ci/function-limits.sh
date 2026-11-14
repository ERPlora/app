#!/usr/bin/env bash
# Fails when clippy's function limits are switched off or loosened, or when the functions excused
# from them grow in number. The limits: a function of at most 100 lines (`too_many_lines`) and 7
# arguments (`too_many_arguments`), denied for the whole workspace. A function over them today is
# excused with `#[expect(clippy::…, reason = "…")]`, which becomes an error once it is no longer
# needed (`unfulfilled_lint_expectations` is denied as well); an `allow` would stay silent forever,
# so it is refused. Against the base (default `origin/main`) no file may gain an excuse.
#
# Usage: function-limits.sh [checkout] [base]   (defaults: the current directory, origin/main)
# Exit:  0 the limits hold · 1 a setting or an excuse breaks them, one line each
#        2 cannot judge: not a git checkout or the base is missing
set -uo pipefail

root=${1:-.}
base=${2:-origin/main}

if ! top=$(git -C "$root" rev-parse --show-toplevel 2>/dev/null); then
    echo "::error::$root is not a git checkout" >&2
    exit 2
fi
cd "$top" || exit 2

if ! git rev-parse --verify --quiet "$base^{commit}" >/dev/null; then
    echo "::error::the base $base is not in this checkout: run git fetch origin main (the excuses can only shrink against it)" >&2
    exit 2
fi

found=0
error() { echo "::error file=$1::$2"; found=$((found + 1)); }

# table_has <file> <table> <key> <value>: inside that TOML table, the key is set to a line holding
# the value.
table_has() {
    awk -v table="[$2]" -v key="$3" -v value="$4" '
        /^[ \t]*\[/ { header = $0; sub(/[ \t]*(#.*)?$/, "", header); sub(/^[ \t]*/, "", header); inside = (header == table); next }
        inside && $0 ~ "^[ \t]*" key "[ \t]*=" && index($0, value) { found = 1 }
        END { exit !found }
    ' "$1" 2>/dev/null
}
denies() { table_has Cargo.toml "$1" "$2" '"deny"'; }

grep -Eq '^too-many-lines-threshold[[:space:]]*=[[:space:]]*100[[:space:]]*$' clippy.toml 2>/dev/null \
    || error clippy.toml "clippy.toml must set too-many-lines-threshold = 100"
grep -Eq '^too-many-arguments-threshold[[:space:]]*=[[:space:]]*7[[:space:]]*$' clippy.toml 2>/dev/null \
    || error clippy.toml "clippy.toml must set too-many-arguments-threshold = 7"
for lint in too_many_lines too_many_arguments; do
    denies workspace.lints.clippy "$lint" \
        || error Cargo.toml "Cargo.toml [workspace.lints.clippy] must deny $lint"
done
denies workspace.lints.rust unfulfilled_lint_expectations \
    || error Cargo.toml "Cargo.toml [workspace.lints.rust] must deny unfulfilled_lint_expectations"

# A crate without these two lines does not get the workspace lints, the limits included.
while IFS= read -r manifest; do
    table_has "$manifest" lints workspace true \
        || error "$manifest" "$manifest must inherit the workspace lints: [lints] workspace = true"
done < <(git ls-files | grep -E '(^|/)Cargo\.toml$' | grep -vx 'Cargo.toml')

# Clippy reads the clippy.toml nearest to each crate, so another one would replace the root's.
while IFS= read -r other; do
    error "$other" "$other replaces the root clippy.toml for its crate: remove it"
done < <(git ls-files | grep -E '(^|/)\.?clippy\.toml$' | grep -vx 'clippy.toml')

# An excuse names one of the limits, or a lint group that holds one of them.
lints='clippy::(too_many_lines|too_many_arguments|complexity|pedantic|all)([^A-Za-z0-9_]|$)'

rust_files=()
while IFS= read -r path; do
    [ -f "$path" ] && rust_files+=("$path")
done < <(git -c core.quotepath=off ls-files '*.rs')

if [ ${#rust_files[@]} -gt 0 ]; then
    # An attribute can be wrapped over several lines: it is read up to its closing bracket.
    bad_excuses=$(awk -v lints="$lints" '
        function depth(s,    t) { t = s; return gsub(/\[/, "", t) - gsub(/\]/, "", s) }
        /^[ \t]*#!?\[/ {
            start = FNR; attr = $0
            while (depth(attr) > 0 && (getline line) > 0) attr = attr " " line
            if (attr !~ lints) next
            if (attr ~ /#!?\[[ \t]*allow[ \t]*\(/)
                printf "::error file=%s,line=%d::%s:%d: use #[expect(clippy::…, reason = \"…\")], not allow, so that it fails once it is no longer needed\n", FILENAME, start, FILENAME, start
            else if (attr !~ /reason[ \t]*=/)
                printf "::error file=%s,line=%d::%s:%d: an excuse says why: reason = \"…\"\n", FILENAME, start, FILENAME, start
        }
    ' "${rust_files[@]}")
    if [ -n "$bad_excuses" ]; then
        echo "$bad_excuses"
        found=$((found + $(printf '%s\n' "$bad_excuses" | grep -c .)))
    fi
fi

if git cat-file -e "$base:clippy.toml" 2>/dev/null; then
    # Excuses per file, here and in the base: no file may have more than the base had.
    gained=$(awk -F: -v base="$base" '
        FILENAME == ARGV[1] { had[$1] = $2; next }
        $2 > had[$1] + 0 { printf "%s: %d function-limit excuse(s), %s has %d\n", $1, $2, base, had[$1] + 0 }
    ' <(git grep -c -E "$lints" "$base" -- '*.rs' | sed "s#^$base:##") \
        <(git grep -c -E "$lints" -- '*.rs'))
    while IFS= read -r line; do
        [ -n "$line" ] || continue
        error "${line%%:*}" "$line"
    done <<< "$gained"
else
    # Only true on the commit that brings the limits: after it, the base always has them.
    echo "$base has no clippy.toml: the function limits start here"
fi

if [ "$found" -eq 0 ]; then
    echo "clippy's function limits are on, and no file gained an excuse from them"
    exit 0
fi
cat >&2 <<'HINT'

A function over 100 lines or 7 arguments is split, or its arguments grouped in a struct. The
functions excused today carry #[expect(clippy::…, reason = "…")]: when one is fixed, its expect
fails and is removed. No new excuse is added.
HINT
exit 1
