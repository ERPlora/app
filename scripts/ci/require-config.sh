#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# require-config.sh — fail LOUDLY when a secret or Variable a job needs is empty.
#
#   bash scripts/ci/require-config.sh secret:SELLER_ID var:MSIX_IDENTITY_NAME …
#
# Each NAME is read from the environment (the step maps `secrets.NAME` /
# `vars.NAME` into `env:`). An unset secret renders as an empty string and the
# step carries on: Microsoft Store then dies inside `msstore` with an auth error,
# Google Play inside the upload action, and Android builds UNSIGNED — none of
# them naming the secret. This names it, with the command that sets it.
#
# Exit 0 = all set · 1 = something is missing (one `::error::` per name) ·
# 2 = usage error (a malformed argument never passes as "checked").
# Tests: scripts/tests/require-config.test.sh
# ─────────────────────────────────────────────────────────────────────────────
set -uo pipefail

repo="${GITHUB_REPOSITORY:-ERPlora/app}"

if [ $# -eq 0 ]; then
    echo "usage: $0 secret:NAME|var:NAME ..." >&2
    exit 2
fi

missing=0
for arg in "$@"; do
    kind="${arg%%:*}"
    name="${arg#*:}"
    if [ "$kind" = "$arg" ] || [ -z "$name" ] || ! [[ "$name" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]]; then
        echo "usage: '$arg' is not secret:NAME or var:NAME" >&2
        exit 2
    fi
    case "$kind" in
        secret) what="secret";   fix="gh secret set -R $repo $name" ;;
        var)    what="Variable"; fix="gh variable set -R $repo $name --body <value>" ;;
        *)      echo "usage: '$arg' is not secret:NAME or var:NAME" >&2; exit 2 ;;
    esac
    value="${!name:-}"
    if [ -z "${value//[[:space:]]/}" ]; then
        echo "::error::the repository $what $name is not set (or empty) in $repo — this step cannot run without it. Set it with: $fix"
        missing=$((missing + 1))
    fi
done

if [ "$missing" -gt 0 ]; then
    echo "$missing required setting(s) missing — see the errors above"
    exit 1
fi
echo "all $# required setting(s) are set"
