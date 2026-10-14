#!/usr/bin/env bash
# Contract test for ERPlora/hub#1881 — no workflow may lean on the DEFAULT package list of
# `android-actions/setup-android`, and none may ask for the retired `tools` package.
#
# WHAT HAPPENED. On 2026-09-16 every open PR of the hub went red, including one that only
# touched `.md`. The step `Setup Android SDK` of `test-shell.yml` used the action bare:
#
#     - name: Setup Android SDK
#       uses: android-actions/setup-android@v3
#
# With no `with:`, the action applies its own default — `packages: tools platform-tools`, read
# straight off the `action.yml` of the v3 tag — and after accepting licences it runs
# `sdkmanager tools platform-tools`. Google had just removed the legacy `tools` package from its
# remote repository, so:
#
#     [command]/home/runner/.android/sdk/cmdline-tools/16.0/bin/sdkmanager tools
#     Warning: Failed to find package 'tools'
#     Error: The process '.../sdkmanager' failed with exit code 1
#
# Nothing in this repository ever used `tools`: the very next step installs what the Kotlin tests
# need, explicitly (`sdkmanager --install "platforms;android-36" "build-tools;36.0.0"`). The whole
# fleet was blocked by a package nobody asked for, arriving through a default nobody read.
#
# WHY A TEST AND NOT JUST THE FIX. The fix is two `with:` blocks, and two `with:` blocks are
# memory: the next workflow that needs Android is written by copying one of these files, and the
# copy that loses the `with:` is green everywhere except on the day the release runs. The failure
# mode is the one this repo keeps meeting — `tauri-release.yml` carries the SAME bare step, and it
# fires on a tag, so its red would have landed in the middle of a release instead of in a PR.
#
# WHAT IT CHECKS. Three rules, deliberately narrow, all read off the PARSED YAML — never grepped,
# so a comment discussing the incident can never trip them:
#
#   A · EXPLICIT PACKAGES — every step using `android-actions/setup-android` declares a non-empty
#       `with: packages:`. Inheriting the default is the defect itself, not a style preference.
#   B · NO `tools` — no such `packages:` list carries the retired package as a whole token.
#       `platform-tools` and `cmdline-tools` are legitimate and must not be confused with it,
#       which is why the match is on tokens and not on a substring.
#   C · NO `tools` VIA sdkmanager — the same request in its other shape: a `run:` step that hands
#       a bare `tools` argument to `sdkmanager`. Only the command's own words count; anything past
#       a pipe, a redirection or a `#` is somebody else's text.
#
# The scan is DISCOVERY-BASED — every workflow and every composite action, not a fixed list — so a
# workflow added tomorrow cannot opt out by not being named here. `KNOWN_USES` is not the scan: it
# is the proof the scan still SEES something, the false green hub#1327 already hit one level up.
# The fixtures at the end are the other half of the same proof: they show the detector catching
# the positive, and leaving alone the shapes it must not fire on.
#
# Run:  bash scripts/tests/setup-android-packages.test.sh
set -uo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)

# A YAML parser is REQUIRED, never optional: a guard that degrades to "skipped" when a tool is
# missing is the same mute green this family of tests exists to abolish.
if ! python3 -c 'import yaml' 2>/dev/null; then
    printf 'FAIL: python3 with PyYAML is required to check this contract (apt: python3-yaml)\n' >&2
    exit 1
fi

REPO_ROOT="$repo_root" python3 - <<'PY'
import glob
import os
import re
import shlex
import sys

import yaml

repo_root = os.environ["REPO_ROOT"]

ACTION = "android-actions/setup-android"
RETIRED = "tools"

# The steps that use the action TODAY. Not the scan — the proof that the scan still finds
# something. If one is deliberately retired, delete its line here too.
KNOWN_USES = {
    ".github/workflows/tauri-release.yml": ["Setup Android SDK"],
    ".github/workflows/test-shell.yml": ["Setup Android SDK"],
}

failures = []
passed = 0


def check(name, ok, detail=""):
    global passed
    if ok:
        passed += 1
    else:
        failures.append(f"{name}{': ' + detail if detail else ''}")


def uses_action(step):
    """Does this step invoke setup-android, at whatever ref?"""
    uses = str(step.get("uses") or "")
    return uses.split("@", 1)[0].strip() == ACTION


def packages_findings(step):
    """Rules A and B over one `uses: android-actions/setup-android` step."""
    with_block = step.get("with")
    packages = with_block.get("packages") if isinstance(with_block, dict) else None
    if packages is None or not str(packages).strip():
        return [
            "inherits the action's default `packages: tools platform-tools`, and Google no "
            "longer serves `tools` — declare `with: packages: platform-tools` (the input "
            "REPLACES the default, it does not add to it)"
        ]
    tokens = [t.strip("'\"") for t in str(packages).split()]
    if RETIRED in tokens:
        return [
            f"asks for the retired `{RETIRED}` package (`packages: {packages}`) — it is gone "
            "from Google's remote repository and `sdkmanager` exits 1 on it. `platform-tools` "
            "and `cmdline-tools` are different packages and stay legal"
        ]
    return []


# Only the command's OWN words: a pipe, a redirection or a comment starts somebody else's text,
# and `sdkmanager --list | grep tools` asks for nothing.
SEGMENT_SPLIT = re.compile(r"[|;&]|\n")


def sdkmanager_findings(run):
    """Rule C: a `run:` body handing a bare `tools` argument to sdkmanager."""
    found = []
    for line in str(run).splitlines():
        line = line.split("#", 1)[0]
        for segment in SEGMENT_SPLIT.split(line):
            if "sdkmanager" not in segment:
                continue
            try:
                words = shlex.split(segment)
            except ValueError:
                continue
            if not any(w.endswith("sdkmanager") for w in words):
                continue
            after = words[words.index(next(w for w in words if w.endswith("sdkmanager"))) + 1:]
            if RETIRED in [w for w in after if not w.startswith("-") and ">" not in w]:
                found.append(
                    f"`{segment.strip()}` asks `sdkmanager` for the retired `{RETIRED}` package, "
                    "which Google no longer serves — it exits 1 and takes the job with it"
                )
    return found


files = sorted(
    set(glob.glob(os.path.join(repo_root, ".github/workflows/*.yml")))
    | set(glob.glob(os.path.join(repo_root, ".github/workflows/*.yaml")))
    # The other place a `run:` or a `uses:` can live in this repo (hub#1327 made the same point
    # for the alert-issue guard): a step moved into a composite action would otherwise walk
    # straight out of this guard one level down.
    | set(glob.glob(os.path.join(repo_root, ".github/actions/**/action.yml"), recursive=True))
    | set(glob.glob(os.path.join(repo_root, ".github/actions/**/action.yaml"), recursive=True))
)
check(
    ".github/ holds workflows to scan",
    bool(files),
    "no workflow or composite action matched — the scan would pass without checking anything",
)

discovered = {}

for path in files:
    rel_path = os.path.relpath(path, repo_root)
    try:
        with open(path, encoding="utf-8") as fh:
            doc = yaml.safe_load(fh)
    except yaml.YAMLError as exc:
        check(f"{rel_path} parses as YAML", False, str(exc).replace("\n", " ")[:200])
        continue
    if not isinstance(doc, dict):
        continue

    step_lists = []
    for job_name, job in (doc.get("jobs") or {}).items():
        if isinstance(job, dict):
            step_lists.append((job_name, job.get("steps") or []))
    runs = doc.get("runs")
    if isinstance(runs, dict) and isinstance(runs.get("steps"), list):
        step_lists.append(("runs", runs["steps"]))

    for job_name, steps in step_lists:
        for index, step in enumerate(steps):
            if not isinstance(step, dict):
                continue
            label = str(step.get("name") or step.get("uses") or f"step {index}")
            where = f"{rel_path} · job `{job_name}` · step «{label}»"

            if uses_action(step):
                discovered.setdefault(rel_path, []).append(label)
                problems = packages_findings(step)
                check(
                    f"{where}: declares its Android packages, without `{RETIRED}`",
                    not problems,
                    problems[0] if problems else "",
                )

            if step.get("run"):
                problems = sdkmanager_findings(step["run"])
                check(
                    f"{where}: no `sdkmanager` call asks for `{RETIRED}`",
                    not problems,
                    problems[0] if problems else "",
                )

for rel_path, step_names in sorted(KNOWN_USES.items()):
    for step_name in step_names:
        check(
            f"{rel_path}: the scan still finds its «{step_name}» step",
            step_name in discovered.get(rel_path, []),
            "no step matched — either it was removed without updating KNOWN_USES, or the "
            "discovery itself is broken and this guard is protecting nothing (hub#1327)",
        )

# ── Fixtures · the detector fires on the shape it exists for, and only on it ─────────────
# Same reason `service-container-ports.test.sh` and `ci-prose-matches-triggers.test.sh` carry
# theirs: a detector nobody proves is a detector that can quietly stop detecting, and the whole
# point of hub#1881 is that a silent default went unread for months.
PACKAGE_FIXTURES = [
    (
        "the hub#1881 shape: the action used bare, inheriting `tools platform-tools`",
        {"uses": "android-actions/setup-android@v3"},
        1,
    ),
    (
        "a `with:` that sets something else but still no `packages:`",
        {"uses": "android-actions/setup-android@v3", "with": {"cmdline-tools-version": "12266719"}},
        1,
    ),
    (
        "an empty `packages:`, which falls back to the same default",
        {"uses": "android-actions/setup-android@v3", "with": {"packages": "   "}},
        1,
    ),
    (
        "`packages:` still carrying the retired package",
        {"uses": "android-actions/setup-android@v3", "with": {"packages": "tools platform-tools"}},
        1,
    ),
    (
        "the fix: only the package we actually use",
        {"uses": "android-actions/setup-android@v3", "with": {"packages": "platform-tools"}},
        0,
    ),
    (
        "`platform-tools` and `cmdline-tools` are NOT the retired `tools`",
        {"uses": "android-actions/setup-android@v3",
         "with": {"packages": "platform-tools cmdline-tools;latest"}},
        0,
    ),
    (
        "the release build's extra packages stay legal",
        {"uses": "android-actions/setup-android@v3",
         "with": {"packages": "platform-tools ndk;27.1.12297006"}},
        0,
    ),
]
for label, step, want in PACKAGE_FIXTURES:
    got = len(packages_findings(step))
    check(
        f"packages detector: {label} → {want} finding(s)",
        got == want,
        f"detector found {got}, expected {want} — rule A/B "
        f"{'stops covering' if want else 'starts firing on'} this shape",
    )

SDKMANAGER_FIXTURES = [
    ("the bare install the action performs", "sdkmanager tools platform-tools", 1),
    ("the same request spelled with --install", 'sdkmanager --install "tools"', 1),
    ("what this repo really runs", 'sdkmanager --install "platforms;android-36" "build-tools;36.0.0"', 0),
    ("accepting licences", "yes | sdkmanager --licenses", 0),
    ("a listing piped through grep, which asks for nothing", "sdkmanager --list | grep tools", 0),
    ("the word inside a comment", "sdkmanager --install \"platform-tools\"  # not tools", 0),
]
for label, run, want in SDKMANAGER_FIXTURES:
    got = len(sdkmanager_findings(run))
    check(
        f"sdkmanager detector: {label} → {want} finding(s)",
        got == want,
        f"detector found {got}, expected {want} — rule C "
        f"{'stops covering' if want else 'starts firing on'} this shape",
    )

if failures:
    print(f"FAIL: {len(failures)} contract case(s) on the Android SDK setup (hub#1881)")
    for f in failures:
        print(f"  - {f}")
    sys.exit(1)

print(f"OK: {passed} contract case(s) on the Android SDK setup (hub#1881)")
PY
