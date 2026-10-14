#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# s3-version-guard.sh — never let ANOTHER run overwrite a version already on S3.
#
#   DOWNLOADS_BUCKET=… VERSION=1.1.31 GITHUB_RUN_ID=… bash scripts/ci/s3-version-guard.sh
#
# Run by `upload-s3` before its first copy. `downloads/app/v<version>/latest.json`
# is written with the metadata `run-id`; if it is already there:
#   · written by this run (a re-run of the same run) → exit 0, the upload resumes
#   · written by another run, or with no run id      → exit 1: re-run THAT run or
#                                                      bump the version
# Not there (404) → exit 0. Any other answer (403, network) → exit 1: «cannot
# tell» is never read as «not there». Missing input → exit 2.
# Tests: scripts/tests/s3-version-guard.test.sh (with a fake `aws`).
# ─────────────────────────────────────────────────────────────────────────────
set -uo pipefail

bucket="${DOWNLOADS_BUCKET:-}"
version="${VERSION:-}"
run_id="${GITHUB_RUN_ID:-}"
if [ -z "$bucket" ] || [ -z "$version" ] || [ -z "$run_id" ]; then
    echo "usage: DOWNLOADS_BUCKET, VERSION and GITHUB_RUN_ID are required" >&2
    exit 2
fi

key="downloads/app/v${version}/latest.json"
if answer=$(aws s3api head-object --bucket "$bucket" --key "$key" \
        --query 'Metadata."run-id"' --output text 2>&1); then
    if [ "$answer" = "$run_id" ]; then
        echo "s3://$bucket/$key was written by this run ($run_id): a re-run, the upload resumes"
        exit 0
    fi
    if [ -z "$answer" ] || [ "$answer" = "None" ]; then
        owner="a run that left no run-id"
    else
        owner="run $answer"
    fi
    echo "::error::v$version is already on S3 (s3://$bucket/$key, written by $owner). Overwriting it would replace what the fleet downloads with a different build of the same version: re-run that run (gh run rerun ${answer:-<run>} --failed) or bump the version in src-tauri/tauri.conf.json"
    exit 1
fi

case "$answer" in
    *"(404)"*|*"Not Found"*)
        echo "v$version is not on S3 yet: first upload"
        exit 0
        ;;
esac
echo "::error::cannot tell whether v$version is already on S3 (s3://$bucket/$key): $answer"
exit 1
