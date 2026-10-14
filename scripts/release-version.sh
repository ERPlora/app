#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Version gate — does this push to `main` release the app?
#
# Called by the first job (`version`) of `.github/workflows/tauri-release.yml`,
# which runs on EVERY push to `main`. The version lives in one place,
# `src-tauri/tauri.conf.json` (`version`), and a released version is marked by
# its tag `v<version>`, created by the last job of the release:
#
#   · tag `v<version>` exists                → release=false, build=false
#                                              (a docs or CI push ships nothing)
#   · tag missing, version > latest `v*` tag → release=true,  build=true
#   · tag missing, version <= latest tag     → RED: Microsoft Store and Google
#                                              Play only take a higher version,
#                                              and the fleet only updates upward
#   · not X.Y.Z, or minor/patch >= 1000      → RED: Play's versionCode is
#                                              major*1000000 + minor*1000 + patch
#                                              (ADR-0160), checked on the APK
#
# A manual dispatch with `publish=false` only builds (packaging check): it never
# releases, whatever the tags say.
#
# Input (environment), so the decision is testable (scripts/release-version.test.sh):
#   VERSION      the version; default: read from TAURI_CONF
#   TAURI_CONF   default: <repo>/src-tauri/tauri.conf.json
#   TAGS         the repository's tags, one per line (or space separated)
#   EVENT_NAME   `push` or `workflow_dispatch`
#   PUBLISH      the dispatch input (`true`/`false`); empty on a push
#
# Output: `version`, `tag`, `build`, `release` in $GITHUB_OUTPUT, a job summary
# in $GITHUB_STEP_SUMMARY, and the exit code (1 = this version cannot ship).
# ─────────────────────────────────────────────────────────────────────────────
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TAURI_CONF="${TAURI_CONF:-$ROOT/src-tauri/tauri.conf.json}"
TAGS="${TAGS:-}"
EVENT_NAME="${EVENT_NAME:-push}"
PUBLISH="${PUBLISH:-}"
GITHUB_OUTPUT="${GITHUB_OUTPUT:-/dev/null}"

SEMVER='^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'

summary() {
    if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
        printf '%s\n' "$@" >> "$GITHUB_STEP_SUMMARY"
    fi
}

emit() {
    local version="$1" build="$2" release="$3"
    {
        echo "version=$version"
        echo "tag=v$version"
        echo "build=$build"
        echo "release=$release"
    } >> "$GITHUB_OUTPUT"
}

fail() {
    echo "::error::$1"
    summary "## Version gate" "" "❌ $1"
    emit "${version:-}" false false
    exit 1
}

# ── The version ──────────────────────────────────────────────────────────────
version="${VERSION:-}"
if [ -z "$version" ]; then
    if [ ! -r "$TAURI_CONF" ]; then
        version=""
        fail "cannot read $TAURI_CONF: the version of the app lives there and nothing else may guess it"
    fi
    version="$(python3 -c 'import json, sys; print(json.load(open(sys.argv[1])).get("version", ""))' "$TAURI_CONF" 2>/dev/null)" \
        || fail "cannot parse $TAURI_CONF as JSON: the version of the app lives there"
fi

if ! [[ "$version" =~ $SEMVER ]]; then
    fail "version '$version' in src-tauri/tauri.conf.json is not X.Y.Z (three numbers, no leading zeros, no pre-release): Microsoft Store and Google Play only take final versions"
fi
major="${BASH_REMATCH[1]}"
minor="${BASH_REMATCH[2]}"
patch="${BASH_REMATCH[3]}"
if [ "$minor" -ge 1000 ] || [ "$patch" -ge 1000 ]; then
    fail "version $version: minor and patch must stay below 1000, or the Android versionCode (major*1000000 + minor*1000 + patch, ADR-0160) collides with another version for ever"
fi
tag="v$version"

# ── The tags ─────────────────────────────────────────────────────────────────
tagged=false
latest=""
latest_key=""
for t in $TAGS; do
    [ "$t" = "$tag" ] && tagged=true
    [[ "$t" =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || continue
    key=$(printf '%010d%010d%010d' "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}" "${BASH_REMATCH[3]}")
    if [ -z "$latest_key" ] || [[ "$key" > "$latest_key" ]]; then
        latest="$t"
        latest_key="$key"
    fi
done
version_key=$(printf '%010d%010d%010d' "$major" "$minor" "$patch")

publish_requested=true
if [ "$EVENT_NAME" = "workflow_dispatch" ] && [ "$PUBLISH" = "false" ]; then
    publish_requested=false
fi

# ── The decision ─────────────────────────────────────────────────────────────
if [ "$tagged" = true ]; then
    if [ "$publish_requested" = false ]; then
        echo "$tag is already released; dispatch with publish=false: building only, nothing is published"
        summary "## Version gate" "" "🔧 **$tag** is already released. Manual dispatch with \`publish=false\`: build only, nothing is published."
        emit "$version" true false
        exit 0
    fi
    echo "$tag is already released: nothing to publish on this run"
    summary "## Version gate" "" \
        "⏭️ **$tag is already released** (the tag exists): nothing to build or publish on this run." "" \
        "To release, bump \`version\` in \`src-tauri/tauri.conf.json\` (above ${latest:-$tag}) and push to \`main\`."
    emit "$version" false false
    exit 0
fi

if [ -n "$latest" ] && ! [[ "$version_key" > "$latest_key" ]]; then
    fail "version $version in src-tauri/tauri.conf.json is not released (no tag $tag) and is not greater than the latest released version $latest. Microsoft Store and Google Play only accept a higher version: bump it above $latest"
fi

if [ "$publish_requested" = false ]; then
    echo "$tag: dispatch with publish=false: building only, nothing is published"
    summary "## Version gate" "" "🔧 **$tag** — manual dispatch with \`publish=false\`: build only, nothing is published or tagged."
    emit "$version" true false
    exit 0
fi

echo "$tag is new (latest released: ${latest:-none}): building, publishing and tagging it"
summary "## Version gate" "" \
    "🚀 **Releasing $tag** (latest released: ${latest:-none}). The tag and the GitHub Release are created last, once every channel has published."
emit "$version" true true
exit 0
