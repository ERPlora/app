#!/usr/bin/env bash
# Runs a command that installs system packages and re-runs it while ANOTHER process holds one of
# apt's locks (ERPlora/infra#313).
#
#   bash scripts/ci/apt-lock-retry.sh sudo apt-get update
#   bash scripts/ci/apt-lock-retry.sh pnpm -F @erplora/web exec playwright install --with-deps chromium
#
# The slots of the shared CI runner share one `/var/lib/apt` and one `/var/lib/dpkg`: when two jobs
# touch apt at the same time, the second one dies at once with
#
#     E: Could not get lock /var/lib/apt/lists/lock. It is held by process 98241 (apt-get)
#
# and the job goes red with every test green. `-o DPkg::Lock::Timeout=N` does not help: it only
# covers the dpkg locks, and `apt-get update` fails on the lists lock whatever it says (measured on
# apt 2.4 and 2.8 — see scripts/tests/apt-lock-retry.test.sh). Hence a retry, and only for that
# failure: anything else (a package that does not exist, no sudo, a network error) keeps its exit
# code and is not retried.
#
# The command must be safe to run twice — `apt-get update`/`install` and `playwright install` are.
#
# Env:
#   APT_LOCK_WAIT_SECONDS    total time to keep retrying (default 600)
#   APT_LOCK_RETRY_INTERVAL  seconds between attempts (default 10)
set -uo pipefail

if [ "$#" -eq 0 ]; then
    echo "usage: apt-lock-retry.sh <command> [args…]" >&2
    exit 2
fi

wait_seconds=${APT_LOCK_WAIT_SECONDS:-600}
interval=${APT_LOCK_RETRY_INTERVAL:-10}
deadline=$(($(date +%s) + wait_seconds))

log=$(mktemp)
trap 'rm -f "$log"' EXIT

attempt=1
while :; do
    # Both streams to the log AND to the job's output: the message that decides is on stderr.
    "$@" 2>&1 | tee "$log"
    rc=${PIPESTATUS[0]}
    if [ "$rc" -eq 0 ]; then
        exit 0
    fi
    # `grep` reads the FILE, not a pipe: `printf … | grep -q` under pipefail is a coin toss.
    if ! grep -qF 'Could not get lock' "$log"; then
        exit "$rc"
    fi
    if [ "$(date +%s)" -ge "$deadline" ]; then
        echo "::error::apt lock still held by another job after ${wait_seconds}s (${attempt} attempts) — giving up (infra#313)" >&2
        exit "$rc"
    fi
    echo "apt lock held by another job on this runner — attempt ${attempt} failed, retrying in ${interval}s (infra#313)" >&2
    sleep "$interval"
    attempt=$((attempt + 1))
done
