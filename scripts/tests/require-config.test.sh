#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Tests for scripts/ci/require-config.sh — the store jobs fail LOUDLY, naming the
# secret or Variable that is missing, instead of failing somewhere else.
#
# Run:  bash scripts/tests/require-config.test.sh
#
# An empty secret does not stop a step: `${{ secrets.X }}` renders as an empty
# string. Without this check the Microsoft Store job dies inside `msstore` with an
# authentication error that never names the secret, Google Play inside the upload
# action, and Android signs nothing (the Gradle config builds UNSIGNED when there
# is no keystore) — an APK nobody can install and an AAB Play rejects.
#
# Contract: `require-config.sh secret:NAME var:NAME …` reads each NAME from the
# environment and exits 1 when any is empty, with one `::error::` per missing
# name and the exact `gh secret set` / `gh variable set` command that fixes it.
# A malformed argument is a usage error (exit 2): a typo must never pass.
# ─────────────────────────────────────────────────────────────────────────────
set -uo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
SCRIPT="$repo_root/scripts/ci/require-config.sh"

pass=0
fail=0
ok()  { printf '  \033[32m✓\033[0m %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  \033[31m✗\033[0m %s\n     %s\n' "$1" "$2"; fail=$((fail + 1)); }

run() { out=$(env "$@" 2>&1); status=$?; }
expect_status() { [ "$status" -eq "$2" ] && ok "$1" || bad "$1" "exit $status (wanted $2): $out"; }
expect_says()   { case "$out" in *"$2"*) ok "$1";; *) bad "$1" "never mentions '$2'. It said: $out";; esac; }
expect_silent_about() { case "$out" in *"$2"*) bad "$1" "mentions '$2' although it is set: $out";; *) ok "$1";; esac; }

echo
echo "require-config.sh (store jobs fail loudly, naming what is missing)"
echo

if [ -f "$SCRIPT" ]; then
    ok "scripts/ci/require-config.sh exists"
else
    bad "scripts/ci/require-config.sh exists" "not found: $SCRIPT"
    printf '\n\033[31m%s\033[0m\n\n' "the script is missing — nothing else can be tested"
    exit 1
fi

run GITHUB_REPOSITORY=ERPlora/app SELLER_ID=123 MSIX_IDENTITY_NAME=ERPlora.ERPlora \
    bash "$SCRIPT" secret:SELLER_ID var:MSIX_IDENTITY_NAME
expect_status "every name set → exit 0" 0

run GITHUB_REPOSITORY=ERPlora/app SELLER_ID= AZURE_AD_TENANT_ID=t \
    bash "$SCRIPT" secret:SELLER_ID secret:AZURE_AD_TENANT_ID
expect_status "an empty secret → exit 1" 1
expect_says "…as an error annotation" "::error::"
expect_says "…naming the secret" "SELLER_ID"
expect_says "…with the command that sets it" "gh secret set -R ERPlora/app SELLER_ID"
expect_silent_about "…and nothing about the one that is set" "AZURE_AD_TENANT_ID"

run GITHUB_REPOSITORY=ERPlora/app bash "$SCRIPT" secret:PLAY_SERVICE_ACCOUNT_JSON
expect_status "an unset secret (not even in the environment) → exit 1" 1
expect_says "…naming it" "PLAY_SERVICE_ACCOUNT_JSON"

run GITHUB_REPOSITORY=ERPlora/app MSIX_IDENTITY_NAME= bash "$SCRIPT" var:MSIX_IDENTITY_NAME
expect_status "an empty Variable → exit 1" 1
expect_says "…with the command for a Variable, not a secret" "gh variable set -R ERPlora/app MSIX_IDENTITY_NAME"

run GITHUB_REPOSITORY=ERPlora/app ANDROID_KEYSTORE_BASE64="   " ANDROID_KEYSTORE_PASSWORD= \
    bash "$SCRIPT" secret:ANDROID_KEYSTORE_BASE64 secret:ANDROID_KEYSTORE_PASSWORD
expect_status "whitespace only counts as missing" 1
expect_says "…and EVERY missing name is reported, not only the first" "ANDROID_KEYSTORE_BASE64"
expect_says "…including the second" "ANDROID_KEYSTORE_PASSWORD"

run GITHUB_REPOSITORY=ERPlora/app SELLER_ID=1 bash "$SCRIPT" SELLER_ID
expect_status "an argument without secret:/var: → usage error (exit 2)" 2

run GITHUB_REPOSITORY=ERPlora/app bash "$SCRIPT" secret:
expect_status "an empty name → usage error (exit 2)" 2

run GITHUB_REPOSITORY=ERPlora/app bash "$SCRIPT"
expect_status "no arguments → usage error (exit 2), never a green that checked nothing" 2

run SELLER_ID= bash "$SCRIPT" secret:SELLER_ID
expect_status "outside Actions (no GITHUB_REPOSITORY) it still fails" 1
expect_says "…and still names the repository of this app" "gh secret set -R ERPlora/app SELLER_ID"

echo
printf 'passed: %d   failed: %d\n\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
