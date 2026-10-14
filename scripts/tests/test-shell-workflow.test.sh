#!/usr/bin/env bash
# Contract of .github/workflows/test-shell.yml — run with:  bash scripts/tests/test-shell-workflow.test.sh
#
# What it pins (hub#2705, ERPlora/pm#655 J3): the installable app is compiled and tested on the
# three desktop systems it ships to, on the PULL REQUEST that touches it. Until then only Linux
# ran here; Windows and macOS were compiled by `tauri-release.yml` alone, on a `vX.Y.Z` tag — so a
# change that broke the Windows or Mac build was found while cutting the release, with the
# installer. The CI was trimmed to one Linux runner when there was one machine for everything; on
# GitHub's runners (public repo, 08/10) that reason is gone.
#
# What must NOT come back with it:
#   · draft PRs spending minutes (measured 29/08: half the runner minutes were runs cancelled by
#     the reviewer's re-push on drafts) — every job keeps the draft filter;
#   · the Kotlin/Android tests three times — they are JVM tests of the Android plugin, the OS of
#     the runner adds nothing, so they stay on Linux only;
#   · one red system hiding the other two — `fail-fast: false`.
#
# Also hub#2689: a push (to `main`) finishes; only a PR's newer head cancels its older run.
#
# Parsed as YAML (PyYAML, as `canonical-mirrors-workflow.test.sh`), so a case names what is
# missing. `--workflow <path>` lets the guard be proven on a mutated copy.
set -uo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
workflow="$repo_root/.github/workflows/test-shell.yml"

while [ $# -gt 0 ]; do
    case "$1" in
        --workflow) workflow="$2"; shift 2 ;;
        *) printf 'usage: %s [--workflow <path>]\n' "$0" >&2; exit 2 ;;
    esac
done

[ -f "$workflow" ] || { printf 'FAIL: no such workflow: %s\n' "$workflow" >&2; exit 1; }

# A YAML parser is REQUIRED, never optional: a guard that degrades to "skipped" when a tool is
# missing is a mute green.
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


SELF = "scripts/tests/test-shell-workflow.test.sh"
DRAFT_IF = "github.event_name != 'pull_request' || !github.event.pull_request.draft"
LINUX_ONLY = "matrix.label == 'linux'"

# ── 1 · The PR trigger, its draft-to-ready type and the trees it watches ──────────────
# YAML 1.1 turns the bare key `on` into the boolean True.
triggers = doc.get("on", doc.get(True)) or {}
pr = triggers.get("pull_request") or {}
push = triggers.get("push") or {}
check("the workflow runs on `pull_request`", isinstance(triggers.get("pull_request"), dict), f"got {pr!r}")
check(
    "`pull_request.types` includes `ready_for_review`",
    "ready_for_review" in (pr.get("types") or []),
    "a draft that becomes ready would never get a run: the draft filter skipped opened/synchronize",
)
for event, block in (("pull_request", pr), ("push", push)):
    paths = block.get("paths") or []
    # `.github/workflows/*.yml`: `release_runner_assumptions.rs` reads EVERY workflow, so a change to
    # any of them can turn this suite red — on 08/10 a guard added to another workflow did, and
    # develop stayed red in the shell with no run saying so.
    # The app is the whole repo (moved out of ERPlora/hub's `apps/tauri/**`): its crate, the
    # bundled page, the workspace manifests and both library crates it compiles against.
    for wanted in (
        "src-tauri/**",
        "shell-dist/**",
        "Cargo.toml",
        "Cargo.lock",
        "crates/peripherals/**",
        "crates/tauri-plugin-erplora-android/**",
        ".github/workflows/test-shell.yml",
        ".github/workflows/*.yml",
        SELF,
    ):
        check(
            f"`{event}.paths` includes `{wanted}`",
            wanted in paths,
            f"paths are {paths} — a change to it alone would run no shell check",
        )

# ── 2 · The test job: three systems, independent, without drafts ──────────────────────
jobs = doc.get("jobs") or {}
job = jobs.get("test") or {}
check("the workflow has the `test` job", bool(job), f"jobs are {list(jobs)}")
check(
    "the `test` job skips draft PRs",
    str(job.get("if", "")).strip() == DRAFT_IF,
    f"if is {job.get('if')!r} — a draft PR would burn three runners per push",
)
strategy = job.get("strategy") or {}
check(
    "`fail-fast: false` — a red system does not cancel the other two",
    strategy.get("fail-fast") is False,
    f"fail-fast is {strategy.get('fail-fast')!r}: a Windows red would hide whether macOS builds",
)
include = (strategy.get("matrix") or {}).get("include") or []
by_label = {str(e.get("label")): e for e in include if isinstance(e, dict)}
check(
    "the matrix has exactly `linux`, `windows` and `macos`",
    sorted(by_label) == ["linux", "macos", "windows"],
    f"labels are {sorted(by_label)}",
)
expected_os = {
    "linux": "${{ vars.CI_RUNNER_LABEL || 'ubuntu-latest' }}",
    "windows": "windows-latest",
    "macos": "macos-latest",
}
for label, os_name in expected_os.items():
    entry = by_label.get(label) or {}
    check(
        f"`{label}` runs on `{os_name}`",
        entry.get("os") == os_name,
        f"got {entry.get('os')!r}",
    )
check(
    "the job runs on `matrix.os`",
    str(job.get("runs-on", "")).strip() == "${{ matrix.os }}",
    f"runs-on is {job.get('runs-on')!r}",
)
check(
    "the job's budget comes from the matrix (`timeout-minutes: matrix.timeout`)",
    str(job.get("timeout-minutes", "")).strip() == "${{ matrix.timeout }}",
    f"timeout-minutes is {job.get('timeout-minutes')!r} — one budget for three systems that compile at different speeds",
)
timeouts = {label: by_label.get(label, {}).get("timeout") for label in expected_os}
check(
    "every system has a numeric budget",
    all(isinstance(t, int) and t > 0 for t in timeouts.values()),
    f"timeouts are {timeouts}",
)
if all(isinstance(t, int) for t in timeouts.values()):
    check(
        "Windows and macOS get at least Linux's budget (they compile slower)",
        timeouts["windows"] >= timeouts["linux"] and timeouts["macos"] >= timeouts["linux"],
        f"timeouts are {timeouts}",
    )

# ── 3 · What runs where ───────────────────────────────────────────────────────────────
steps = job.get("steps") or []
by_name = {str(s.get("name", "")): s for s in steps if isinstance(s, dict)}


def run_of(step):
    return str(step.get("run", ""))


cargo_steps = [s for s in steps if "cargo test" in run_of(s) and "-p erplora-tauri" in run_of(s)]
check("a step runs `cargo test -p erplora-tauri`", len(cargo_steps) == 1, f"found {len(cargo_steps)}")
for s in cargo_steps:
    check(
        "the cargo test step runs on EVERY system (no `if:`)",
        "if" not in s,
        f"if is {s.get('if')!r} — the point of the matrix is that Windows and macOS compile it too",
    )
    check(
        "the cargo test step still covers the Android plugin crate",
        "-p tauri-plugin-erplora-android" in run_of(s),
        run_of(s),
    )
    # In ERPlora/hub this crate was tested by `test-hub.yml`; in this repo nothing else runs it.
    check(
        "the cargo test step covers the hardware crate (erplora-peripherals)",
        "-p erplora-peripherals" in run_of(s),
        run_of(s),
    )

for name in (
    # Bash rules that do not depend on the OS: once is enough, and PowerShell cannot run them.
    "Release gate rules (scripts/release-gate.test.sh)",
    "System deps (webkit2gtk + GTK)",
    "Setup JDK 17",
    "Setup Android SDK",
    "Android platform 36 (sin NDK)",
    "Kotlin unit tests (plugin de permisos + SPP)",
):
    step = by_name.get(name)
    check(f"the step `{name}` exists", step is not None)
    if step is not None:
        check(
            f"`{name}` runs on Linux only",
            str(step.get("if", "")).strip() == LINUX_ONLY,
            f"if is {step.get('if')!r} — webkit2gtk, the Kotlin tests and the OS-independent bash rules are Linux-only work",
        )

# Windows checks out with `core.autocrlf=true`: the tests that read this repo's own sources
# (`include_str!("lib.rs")`, workflows, gradle files) would see CRLF where Linux and macOS see the
# committed LF, and split on "\n" would find nothing. LF is pinned BEFORE the checkout (hub#2705).
names = [str(s.get("name", "")) for s in steps if isinstance(s, dict)]
checkout_at = next((i for i, s in enumerate(steps) if "actions/checkout" in str(s.get("uses", ""))), None)
lf_at = next((i for i, s in enumerate(steps) if "core.autocrlf false" in run_of(s)), None)
check(
    "Windows checks out with LF (`git config --global core.autocrlf false` before the checkout)",
    lf_at is not None and checkout_at is not None and lf_at < checkout_at,
    f"steps are {names}",
)
if lf_at is not None:
    check(
        "the LF step runs on Windows only",
        str(steps[lf_at].get("if", "")).strip() == "matrix.label == 'windows'",
        f"if is {steps[lf_at].get('if')!r}",
    )

# All the test binaries report, not just the first red one: on a matrix, one round trip per red.
for s in cargo_steps:
    check(
        "the cargo test step runs with `--no-fail-fast`",
        "--no-fail-fast" in run_of(s),
        run_of(s),
    )

# Windows runs `run:` under PowerShell by default: a bash body that is not Linux-only must say so.
for s in steps:
    body = run_of(s)
    if not body or str(s.get("if", "")).strip() == LINUX_ONLY:
        continue
    if "set -euo pipefail" in body or "command -v" in body:
        check(
            f"`{s.get('name')}` declares `shell: bash` (it also runs on Windows)",
            s.get("shell") == "bash",
            f"shell is {s.get('shell')!r} — PowerShell would choke on the bash body",
        )

# ── 4 · This contract test RUNS somewhere ─────────────────────────────────────────────
# `actionlint.yml` is not its home: this workflow already fires on every change to itself and to
# this file (section 1), and runs it as a Linux step — a test nobody runs is a shopping list.
contract_steps = [
    s for s in steps
    if any(line.strip() in (f"bash ./{SELF}", f"bash {SELF}") for line in run_of(s).splitlines())
]
check("a step runs this contract test (`bash ./" + SELF + "`)", len(contract_steps) == 1, f"found {len(contract_steps)}")
for s in contract_steps:
    check(
        "the contract step runs on Linux only (once, not three times)",
        str(s.get("if", "")).strip() == LINUX_ONLY,
        f"if is {s.get('if')!r}",
    )

# ── 5 · A push finishes; only a PR's newer head cancels (hub#2689) ────────────────────
concurrency = doc.get("concurrency") or {}
check(
    "a push finishes; only a PR's newer head cancels its older run (hub#2689)",
    str(concurrency.get("cancel-in-progress")).strip() == "${{ github.event_name == 'pull_request' }}",
    f"cancel-in-progress is {concurrency.get('cancel-in-progress')!r}",
)

if failures:
    print(f"FAIL: {len(failures)} contract case(s) on {path}", file=sys.stderr)
    for f in failures:
        print(f"  - {f}", file=sys.stderr)
    sys.exit(1)

print(f"PASS: {passed} test-shell workflow contract cases")
PY
