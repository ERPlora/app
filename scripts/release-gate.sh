#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Release gate — "a green tag means it published" (hub#895)
#
# Called by the last job of `.github/workflows/tauri-release.yml`. It receives
# the result of every publishing job and decides whether the release actually
# shipped. Three tags in a row (the last one v1.0.3, hub#839) ended GREEN while
# publishing nothing: the publish jobs are gated by repository Variables, an
# empty Variable makes the job `skipped`, and a skipped job does not colour a
# run. A tag that published nothing looked exactly like a tag that published
# everything.
#
# The rule this enforces:
#
#   · a channel that published                  → fine
#   · a channel skipped, Variable empty         → RED, naming the Variable
#   · a channel skipped and DECLARED pending    → green, but warned out loud
#   · a channel that failed or was cancelled    → RED (a declaration excuses a
#                                                 skip, never a failure)
#   · a MANDATORY channel (no gating Variable)  → can never be declared pending
#
# Declaring a channel is the repository Variable `RELEASE_CHANNELS_PENDING`, a
# comma-separated list of channel ids (`store`, `play`). It is the difference
# between "we know Play is not wired yet" and "nobody noticed". Anything in it
# that is not an optional channel of this workflow is itself an error: a guard
# that does not validate its payload fails open, which is the failure mode this
# whole script exists to remove.
#
# Input, all through the environment so the workflow expressions stay in the
# YAML and the decision stays testable (scripts/release-gate.test.sh):
#
#   CHANNELS  one line per publishing channel:
#             id|display name|job result|gating Variable|Variable set?|reference
#             An empty gating Variable marks the channel MANDATORY.
#   BUILDS    one line per build job: id|display name|job result   (optional)
#   RELEASE_CHANNELS_PENDING   comma-separated ids, case/space insensitive
#
# Output: `::error::` / `::warning::` annotations, a job summary table, and an
# exit code. Exit 1 = this tag did not publish what it was supposed to.
# ─────────────────────────────────────────────────────────────────────────────
set -uo pipefail

CHANNELS="${CHANNELS:-}"
BUILDS="${BUILDS:-}"
RELEASE_CHANNELS_PENDING="${RELEASE_CHANNELS_PENDING:-}"

failures=""
notes=""
rows=""

add_failure() { failures="${failures}${1}"$'\n'; }
add_note()    { notes="${notes}${1}"$'\n'; }
add_row()     { rows="${rows}| ${1} | ${2} | ${3} |"$'\n'; }

trim() {
    local s="$1"
    s="${s#"${s%%[![:space:]]*}"}"
    s="${s%"${s##*[![:space:]]}"}"
    printf '%s' "$s"
}

# ── The channel table ────────────────────────────────────────────────────────
# Read once into lists so the pending list can be validated against the channels
# that actually exist before any of them is judged.
known_ids=""
optional_ids=""
while IFS='|' read -r id _display _result var_name _var_set _reference; do
    id="$(trim "${id:-}")"
    [ -n "$id" ] || continue
    known_ids="$known_ids $id"
    [ -n "$(trim "${var_name:-}")" ] && optional_ids="$optional_ids $id"
done <<EOF
$CHANNELS
EOF

if [ -z "$known_ids" ]; then
    echo "::error::the release gate was given no channels to check (empty CHANNELS) — it cannot tell a published tag from a silent one, which is the whole reason it exists (hub#895)"
    exit 1
fi

# ── The declared-pending list ────────────────────────────────────────────────
pending_list="$(printf '%s' "$RELEASE_CHANNELS_PENDING" \
    | tr -d '[:space:]' | tr 'A-Z' 'a-z' | tr ',' '\n' | grep -v '^$')"

in_list() { grep -qx -- "$1" <<<"$2"; }
is_pending() { in_list "$1" "$pending_list"; }

while IFS= read -r entry; do
    [ -n "$entry" ] || continue
    if in_list "$entry" "$(printf '%s\n' $optional_ids)"; then
        continue
    fi
    if in_list "$entry" "$(printf '%s\n' $known_ids)"; then
        add_failure "RELEASE_CHANNELS_PENDING lists '$entry', which is a MANDATORY channel of this release: it has no gating Variable precisely because a release without it is not a release. Remove it from the list and fix the channel."
    else
        add_failure "RELEASE_CHANNELS_PENDING lists '$entry', which is not a channel of this workflow (known:$known_ids). Nothing was excused by it — a list with a typo in it silently protects nothing, so it is an error here instead of a surprise on the next tag."
    fi
done <<EOF
$pending_list
EOF

# ── The builds ───────────────────────────────────────────────────────────────
# A failed build skips every publish downstream. Saying "the Variable is missing"
# there would send whoever reads this hunting for a Variable that is perfectly
# fine, so the build is judged first and named as the cause it is.
while IFS='|' read -r id display result; do
    id="$(trim "${id:-}")"
    [ -n "$id" ] || continue
    display="$(trim "${display:-}")"
    result="$(trim "${result:-}")"
    if [ "$result" = "success" ]; then
        add_row "$display" "built" "—"
    else
        add_row "$display" "$result" "job \`$id\`"
        add_failure "$display ($id) ended in '$result': nothing downstream of it could publish. Fix the build — the channels below did not even get the chance."
    fi
done <<EOF
$BUILDS
EOF

# ── The channels ─────────────────────────────────────────────────────────────
while IFS='|' read -r id display result var_name var_set reference; do
    id="$(trim "${id:-}")"
    [ -n "$id" ] || continue
    display="$(trim "${display:-}")"
    result="$(trim "${result:-}")"
    var_name="$(trim "${var_name:-}")"
    var_set="$(trim "${var_set:-}")"
    reference="$(trim "${reference:-}")"

    case "$result" in
    success)
        add_row "$display" "published" "$reference"
        if is_pending "$id"; then
            add_note "$display published, but '$id' is still listed in RELEASE_CHANNELS_PENDING. Remove it: an excuse nobody removes is an excuse that will cover a real outage one day ($reference)."
        fi
        ;;
    skipped)
        if is_pending "$id"; then
            add_row "$display" "NOT published — declared pending" "$reference"
            add_note "$display did NOT publish: '$id' is declared pending in RELEASE_CHANNELS_PENDING, so this tag ships without it on purpose ($reference)."
        elif [ -n "$var_name" ] && [ "$var_set" != "true" ]; then
            add_row "$display" "NOT published — \`$var_name\` empty" "$reference"
            add_failure "$display did NOT publish: the repository Variable $var_name is missing or empty, so its job was skipped and this tag shipped nothing to that channel. Either set $var_name, or declare '$id' in RELEASE_CHANNELS_PENDING to say out loud that the channel is not live yet ($reference)."
        else
            add_row "$display" "NOT published — skipped" "$reference"
            add_failure "$display did NOT publish: its job was skipped even though nothing gates it out (${var_name:-no gating Variable}${var_name:+ is set}). Something upstream of it did not produce what it needed ($reference)."
        fi
        ;;
    *)
        add_row "$display" "NOT published — $result" "$reference"
        add_failure "$display did NOT publish: its job ended in '$result'. A channel declared pending excuses a skip, never a failure — read that job's log ($reference)."
        ;;
    esac
done <<EOF
$CHANNELS
EOF

# ── Report ───────────────────────────────────────────────────────────────────
report="| Channel | Status | Where |"$'\n'"| --- | --- | --- |"$'\n'"$rows"

echo "Release channels for this tag:"
echo
printf '%s' "$report"
echo

while IFS= read -r note; do
    [ -n "$note" ] || continue
    echo "::warning::$note"
done <<EOF
$notes
EOF

while IFS= read -r failure; do
    [ -n "$failure" ] || continue
    echo "::error::$failure"
done <<EOF
$failures
EOF

if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
    {
        echo "## Release gate"
        echo
        printf '%s' "$report"
        echo
        while IFS= read -r note; do
            [ -n "$note" ] || continue
            echo "- ⚠️ $note"
        done <<EOF
$notes
EOF
        while IFS= read -r failure; do
            [ -n "$failure" ] || continue
            echo "- ❌ $failure"
        done <<EOF
$failures
EOF
    } >> "$GITHUB_STEP_SUMMARY"
fi

if [ -n "$failures" ]; then
    echo
    echo "This tag did NOT publish everything it was supposed to. A green run here would have"
    echo "meant 'released' to everyone who looked at it, which is why this job is red instead."
    exit 1
fi

declared=$(printf '%s' "$pending_list" | grep -c '[^[:space:]]')
if [ "$declared" -gt 0 ]; then
    echo "Every channel that was expected to publish did ✓ — with $declared declared pending (see the warnings above)"
else
    echo "Every expected channel published ✓"
fi
