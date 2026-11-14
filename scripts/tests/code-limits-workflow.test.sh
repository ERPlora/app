#!/usr/bin/env bash
# Contract of .github/workflows/code-limits.yml — run with:  bash scripts/tests/code-limits-workflow.test.sh
#
# What it pins: the size of every code file and the function-limit excuses are checked on EVERY
# pull request and every push to `main`, whatever the change touches. `test-shell.yml` only runs
# when certain trees change, so a big file added anywhere else (another folder, a new crate) would
# go unchecked there.
#
# And it stays cheap: one Linux job with the whole history (the checks compare their exception
# lists with the merge-base of HEAD and `main`) and no cargo build.
#
# Parsed as YAML (PyYAML), so a case names what is missing. `--workflow <path>` lets the guard be
# proven on a mutated copy.
set -uo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
workflow="$repo_root/.github/workflows/code-limits.yml"

while [ $# -gt 0 ]; do
    case "$1" in
        --workflow) workflow="$2"; shift 2 ;;
        *) printf 'usage: %s [--workflow <path>]\n' "$0" >&2; exit 2 ;;
    esac
done

[ -f "$workflow" ] || { printf 'FAIL: no such workflow: %s\n' "$workflow" >&2; exit 1; }

# A YAML parser is REQUIRED, never optional: a guard that degrades to "skipped" is a mute green.
if ! python3 -c 'import yaml' 2>/dev/null; then
    printf 'FAIL: python3 with PyYAML is required to check the workflow contract (apt: python3-yaml)\n' >&2
    exit 1
fi

WORKFLOW="$workflow" python3 - <<'PY'
import os
import sys

import yaml

path = os.environ["WORKFLOW"]
with open(path, encoding="utf-8") as fh:
    doc = yaml.safe_load(fh)

failures = []
passed = 0


def check(name, ok, detail=""):
    global passed
    if ok:
        passed += 1
    else:
        failures.append(f"{name}{': ' + detail if detail else ''}")


def run_of(step):
    return str(step.get("run", ""))


DRAFT_IF = "github.event_name != 'pull_request' || !github.event.pull_request.draft"
FILTERS = ("paths", "paths-ignore", "branches", "branches-ignore")

# ── 1 · Every pull request and every push to main, no path filter ──────────────────────────
# YAML 1.1 turns the bare key `on` into the boolean True.
triggers = doc.get("on", doc.get(True)) or {}
pr = triggers.get("pull_request")
push = triggers.get("push")
check("the workflow runs on `pull_request`", isinstance(pr, dict), f"triggers are {triggers!r}")
check("the workflow runs on `push`", isinstance(push, dict), f"triggers are {triggers!r}")
pr = pr if isinstance(pr, dict) else {}
push = push if isinstance(push, dict) else {}
for name in FILTERS:
    check(
        f"`pull_request` has no `{name}` filter: every pull request is checked",
        name not in pr,
        f"pull_request is {pr!r}",
    )
check(
    "`pull_request.types` includes `ready_for_review` (a draft made ready gets its run)",
    "ready_for_review" in (pr.get("types") or []),
    f"types are {pr.get('types')!r}",
)
check("`push` runs on `main`", push.get("branches") == ["main"], f"push is {push!r}")
for name in ("paths", "paths-ignore"):
    check(f"`push` has no `{name}` filter: every push to main is checked", name not in push, f"push is {push!r}")

# ── 2 · One cheap Linux job that runs both checks over the whole history ─────────────────────
jobs = doc.get("jobs") or {}
check("the workflow has exactly one job", len(jobs) == 1, f"jobs are {list(jobs)}")
job = next(iter(jobs.values()), {}) or {}
check("the job runs on the Linux runner", str(job.get("runs-on", "")).strip() == "ubuntu-latest", f"runs-on is {job.get('runs-on')!r}")
check("the job skips draft pull requests", str(job.get("if", "")).strip() == DRAFT_IF, f"if is {job.get('if')!r}")
timeout = job.get("timeout-minutes")
check("the job has a short timeout (≤ 10 minutes)", isinstance(timeout, int) and timeout <= 10, f"timeout-minutes is {timeout!r}")

steps = [s for s in (job.get("steps") or []) if isinstance(s, dict)]
checkout_at = next((i for i, s in enumerate(steps) if "actions/checkout" in str(s.get("uses", ""))), None)
check("the job checks out the repository", checkout_at is not None, f"steps are {steps!r}")
if checkout_at is not None:
    depth = str((steps[checkout_at].get("with") or {}).get("fetch-depth"))
    check(
        "the checkout brings the whole history (`fetch-depth: 0`): the checks need the merge-base with main",
        depth == "0",
        f"fetch-depth is {depth!r}",
    )
for script in ("scripts/ci/file-size.sh", "scripts/ci/function-limits.sh"):
    at = next((i for i, s in enumerate(steps) if script in run_of(s)), None)
    check(f"a step runs `{script}`", at is not None, f"steps are {[run_of(s) for s in steps]!r}")
    if at is not None and checkout_at is not None:
        check(f"`{script}` runs after the checkout", checkout_at < at)
        check(f"`{script}` runs without `if:` (never skipped)", "if" not in steps[at], f"if is {steps[at].get('if')!r}")
        check(f"`{script}` is not allowed to fail", not steps[at].get("continue-on-error"), repr(steps[at]))

heavy = [s for s in steps if "cargo" in run_of(s) or "rust" in str(s.get("uses", ""))]
check("no cargo build, toolchain or cargo cache: the job stays seconds long", not heavy, f"steps {heavy!r}")

if failures:
    print(f"FAIL: {len(failures)} contract case(s) on {path}", file=sys.stderr)
    for f in failures:
        print(f"  - {f}", file=sys.stderr)
    sys.exit(1)

print(f"PASS: {passed} code-limits workflow contract cases")
PY
