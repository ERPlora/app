#!/usr/bin/env bash
# Contract test for ERPlora/infra#313 — a job that installs system packages on the SHARED CI
# runner must wait out the apt lock another job holds, instead of dying on it.
#
# WHAT HAPPENED. On 2026-09-26 the Playwright job of hub#2225 went red with every test green: its
# step `Instalar el navegador de Playwright` runs `playwright install --with-deps chromium`, which
# shells out to `sudo apt-get update && sudo apt-get install …`, and another job on the same
# machine was running `apt-get` at that moment:
#
#     E: Could not get lock /var/lib/apt/lists/lock. It is held by process 98241 (apt-get)
#     Installation process exited with code: 100
#
# Re-running the same commit went green. The slots of `ci-runner-1` share one `/var/lib/apt` and
# one `/var/lib/dpkg`, so any two jobs that touch apt at the same time can knock each other over.
#
# WHY A RETRY AND NOT `-o DPkg::Lock::Timeout=N`. That option only makes apt wait for the DPKG
# locks. The lock in the incident is the LISTS lock that `apt-get update` takes, and apt fails on
# it at once whatever the timeout says — measured on 2026-09-27 against apt 2.4.14 (ubuntu:22.04)
# and 2.8.3 (ubuntu:24.04), holding the lock with fcntl from another process: `apt-get -o
# DPkg::Lock::Timeout=60 update` exits 100 in 8 ms. The same goes for a system-wide
# `/etc/apt/apt.conf.d` on the runner, which is why the fix lives here and not in ERPlora/infra.
# Playwright's `--with-deps` also gives no way to pass apt options. So every such step goes
# through `scripts/ci/apt-lock-retry.sh`, which re-runs the command while the failure is a held
# lock, and gives up on anything else at once.
#
# WHAT IT CHECKS.
#   Part 1 — the wrapper itself, against a fake command (no apt, no sudo):
#     1 · a command that fails on a held lock is re-run until it succeeds, and the job is green;
#     2 · any other failure is NOT retried and its exit code comes through untouched;
#     3 · «Could not open lock file» (not root) is not a held lock: no retry;
#     4 · the wait is bounded: past APT_LOCK_WAIT_SECONDS it stops with the command's exit code;
#     5 · the command's output stays visible in the log;
#     6 · a call without a command is a usage error, not a silent green.
#   Part 2 — the workflows, read off the PARSED YAML (discovery-based, every workflow and composite
#     action): every `run:` line that calls `apt-get`/`apt` update|install|upgrade, or Playwright
#     with `--with-deps` / `install-deps`, goes through the wrapper — EVERY such call on the line,
#     each one inside its own command (a wrapper before `&&`/`;`/`||` guards only the call before
#     it; hub#2267, hub#2268). `KNOWN` is the proof the scan
#     still SEES the steps that exist today; the fixtures prove the detector catches the positive
#     and leaves the harmless shapes alone.
#
# Run:  bash scripts/tests/apt-lock-retry.test.sh
set -uo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
wrapper="$repo_root/scripts/ci/apt-lock-retry.sh"

failures=0
passed=0
pass() { passed=$((passed + 1)); printf 'ok   %s\n' "$1"; }
fail() { failures=$((failures + 1)); printf 'FAIL %s\n' "$1" >&2; [ -n "${2:-}" ] && printf '     %s\n' "$2" >&2; return 0; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# A fake apt: fails with the message in $FAKE_MESSAGE (exit $FAKE_RC) for the first $FAKE_FAILS
# calls, then succeeds. Every call is counted in $work/calls.
cat >"$work/fake-apt" <<'EOF'
#!/usr/bin/env bash
count_file="$FAKE_DIR/calls"
n=$(( $(cat "$count_file" 2>/dev/null || echo 0) + 1 ))
echo "$n" >"$count_file"
echo "fake-apt attempt $n"
if [ "$n" -le "${FAKE_FAILS:-0}" ]; then
    echo "$FAKE_MESSAGE" >&2
    exit "${FAKE_RC:-100}"
fi
echo "fake-apt done"
EOF
chmod +x "$work/fake-apt"

HELD='E: Could not get lock /var/lib/apt/lists/lock. It is held by process 98241 (apt-get)'

# run_wrapper <fails> <rc> <message> [env…] — runs the wrapper under a hard cap so a wrapper that
# never gives up turns this suite red instead of hanging it (the infra#310 lesson: every script
# with a wait loop needs a ceiling in its test). The cap is `perl alarm` + `exec`, not `timeout`:
# macOS has no `timeout`, and SIGALRM kills the wrapper with rc 142.
run_wrapper() {
    local fails=$1 rc=$2 message=$3
    shift 3
    rm -f "$work/calls"
    env FAKE_DIR="$work" FAKE_FAILS="$fails" FAKE_RC="$rc" FAKE_MESSAGE="$message" \
        APT_LOCK_RETRY_INTERVAL=0 "$@" \
        perl -e 'alarm shift; exec @ARGV' 30 bash "$wrapper" "$work/fake-apt" >"$work/out" 2>&1
    echo $? >"$work/rc"
}
calls() { cat "$work/calls" 2>/dev/null || echo 0; }
out_has() { grep -qF -- "$1" "$work/out"; }

if [ ! -f "$wrapper" ]; then
    fail "scripts/ci/apt-lock-retry.sh exists" "missing — every Part 1 case below fails with it"
fi

# 1 · held lock twice, then success → green after three calls.
run_wrapper 2 100 "$HELD" APT_LOCK_WAIT_SECONDS=60
if [ "$(cat "$work/rc")" = 0 ] && [ "$(calls)" = 3 ]; then
    pass "1 · a held apt lock is waited out and the step ends green"
else
    fail "1 · a held apt lock is waited out and the step ends green" \
        "rc=$(cat "$work/rc") calls=$(calls) (want rc=0 calls=3)"
fi

# 1b · the dpkg frontend lock (apt-get install) is the same case.
run_wrapper 1 100 'E: Could not get lock /var/lib/dpkg/lock-frontend. It is held by process 4242 (apt-get)' APT_LOCK_WAIT_SECONDS=60
if [ "$(cat "$work/rc")" = 0 ] && [ "$(calls)" = 2 ]; then
    pass "1b · the dpkg frontend lock is waited out too"
else
    fail "1b · the dpkg frontend lock is waited out too" "rc=$(cat "$work/rc") calls=$(calls) (want rc=0 calls=2)"
fi

# 2 · a real failure (package not found) → exactly one call, its exit code untouched.
run_wrapper 5 100 'E: Unable to locate package libdoesnotexist' APT_LOCK_WAIT_SECONDS=60
if [ "$(cat "$work/rc")" = 100 ] && [ "$(calls)" = 1 ]; then
    pass "2 · any other failure is not retried and keeps its exit code"
else
    fail "2 · any other failure is not retried and keeps its exit code" \
        "rc=$(cat "$work/rc") calls=$(calls) (want rc=100 calls=1)"
fi

# 2b · the exit code is the command's, whatever it is (not a blanket 1).
run_wrapper 5 7 'boom' APT_LOCK_WAIT_SECONDS=60
if [ "$(cat "$work/rc")" = 7 ] && [ "$(calls)" = 1 ]; then
    pass "2b · the command's own exit code comes through"
else
    fail "2b · the command's own exit code comes through" "rc=$(cat "$work/rc") calls=$(calls) (want rc=7 calls=1)"
fi

# 3 · not root: apt cannot even OPEN the lock file. Waiting would never fix it.
run_wrapper 5 100 'E: Could not open lock file /var/lib/apt/lists/lock - open (13: Permission denied)' APT_LOCK_WAIT_SECONDS=60
if [ "$(cat "$work/rc")" = 100 ] && [ "$(calls)" = 1 ]; then
    pass "3 · a permission error on the lock file is not mistaken for a held lock"
else
    fail "3 · a permission error on the lock file is not mistaken for a held lock" \
        "rc=$(cat "$work/rc") calls=$(calls) (want rc=100 calls=1)"
fi

# 4 · the lock never frees: the wrapper stops once the budget is spent, with apt's exit code.
run_wrapper 1000 100 "$HELD" APT_LOCK_WAIT_SECONDS=0
if [ "$(cat "$work/rc")" = 100 ] && [ "$(calls)" -ge 1 ] && [ "$(calls)" -le 2 ]; then
    pass "4 · the wait is bounded by APT_LOCK_WAIT_SECONDS"
else
    fail "4 · the wait is bounded by APT_LOCK_WAIT_SECONDS" \
        "rc=$(cat "$work/rc") calls=$(calls) (want rc=100 after 1-2 calls; 142 = never gave up)"
fi

# 5 · the command's own output reaches the log, both streams.
run_wrapper 1 100 "$HELD" APT_LOCK_WAIT_SECONDS=60
if out_has 'fake-apt done' && out_has 'Could not get lock /var/lib/apt/lists/lock'; then
    pass "5 · the command's output stays visible"
else
    fail "5 · the command's output stays visible" "log: $(tr '\n' '|' <"$work/out")"
fi

# 6 · no command → usage error.
perl -e 'alarm shift; exec @ARGV' 30 bash "$wrapper" >"$work/out" 2>&1
rc=$?
if [ "$rc" = 2 ]; then
    pass "6 · no command is a usage error (exit 2)"
else
    fail "6 · no command is a usage error (exit 2)" "rc=$rc"
fi

# ── Part 2 — the workflows ────────────────────────────────────────────────────────────────────
# A YAML parser is REQUIRED, never optional: a guard that degrades to "skipped" when a tool is
# missing is the same mute green this family of tests exists to abolish.
if ! python3 -c 'import yaml' 2>/dev/null; then
    fail "python3 with PyYAML is available" "required to check the workflows (apt: python3-yaml)"
else
    REPO_ROOT="$repo_root" python3 - <<'PY'
import glob
import os
import re
import sys

import yaml

repo_root = os.environ["REPO_ROOT"]
WRAPPER = "scripts/ci/apt-lock-retry.sh"

# The steps that touch apt TODAY. Not the scan — the proof the scan still finds something. If one
# is deliberately retired, delete its line here too.
# (`test-web.yml` and `visual-baselines.yml` stayed in ERPlora/hub when the app moved here.)
KNOWN = {
    ".github/workflows/test-shell.yml": ["System deps (webkit2gtk + GTK)"],
    ".github/workflows/tauri-release.yml": ["Install Linux build deps"],
}

APT = re.compile(r"(?:^|[\s;&|(])(?:sudo\s+(?:-\S+\s+)*)?apt(?:-get)?\s+(?:-\S+(?:\s+[^\s-]\S*)?\s+)*(?:update|install|upgrade|dist-upgrade|full-upgrade)\b")
PLAYWRIGHT_DEPS = re.compile(r"\bplaywright\s+(?:install-deps\b|install\b[^\n]*--with-deps\b)")

failures = []
passed = 0


def check(name, ok, detail=""):
    global passed
    if ok:
        passed += 1
    else:
        failures.append(f"{name}{': ' + detail if detail else ''}")


def logical_lines(run):
    """The shell lines of a `run:` body: comments dropped, `\\`-continuations joined."""
    lines = []
    pending = ""
    for raw in str(run).splitlines():
        stripped = raw.strip()
        if not pending and stripped.startswith("#"):
            continue
        if stripped.endswith("\\"):
            pending += stripped[:-1] + " "
            continue
        lines.append(pending + stripped)
        pending = ""
    if pending:
        lines.append(pending)
    return lines


SEPARATOR = re.compile(r"&&|\|\||[;|&]")


def guarded(line, match):
    """The wrapper prefixes the command this apt call belongs to (text since the last `&&`/`;`/`|`)."""
    return WRAPPER in SEPARATOR.split(line[: match.start() + 1])[-1]


def unguarded(run):
    """Every line with an apt call that does not go through the wrapper — ALL calls, not the first."""
    found = []
    for line in logical_lines(run):
        matches = [*APT.finditer(line), *PLAYWRIGHT_DEPS.finditer(line)]
        if any(not guarded(line, match) for match in matches):
            found.append(line)
    return found


def touches_apt(run):
    return any(APT.search(l) or PLAYWRIGHT_DEPS.search(l) for l in logical_lines(run))


# ── The detector against fixtures: it must catch the positive and leave the rest alone. ──
FIXTURES = [
    ("sudo apt-get update", True),
    ("sudo apt-get install -y --no-install-recommends \\\n  libgtk-3-dev pkg-config", True),
    ("pnpm -F @erplora/web exec playwright install --with-deps chromium", True),
    ("uv run playwright install-deps chromium", True),
    ("sudo apt install -y jq", True),
    ("sudo -E apt-get -o DPkg::Lock::Timeout=600 update", True),
    # The wrapper only guards the command it prefixes: later on the line, or before a second apt
    # call chained with `&&`/`;`/`||`, it does not count (hub#2267, hub#2268).
    (f"sudo apt-get update && bash {WRAPPER} true", True),
    (f"bash {WRAPPER} sudo apt-get update && sudo apt-get install -y jq", True),
    (f"bash {WRAPPER} sudo apt-get update; sudo apt-get install -y jq", True),
    # No space after the separator: the apt match STARTS on `;`, so the command boundary has to
    # include that character or the previous command's wrapper leaks into this one.
    (f"bash {WRAPPER} sudo apt-get update;sudo apt-get install -y jq", True),
    (f"bash {WRAPPER} sudo apt-get update || sudo apt-get install -y jq", True),
    (f"bash {WRAPPER} sudo apt-get update && pnpm exec playwright install --with-deps chromium", True),
    (f"bash {WRAPPER} sudo apt-get update && bash {WRAPPER} sudo apt-get install -y jq", False),
    (f"bash {WRAPPER} sudo apt-get update", False),
    (f"bash {WRAPPER} sudo apt-get install -y \\\n  libgtk-3-dev", False),
    (f"bash {WRAPPER} pnpm -F @erplora/web exec playwright install --with-deps chromium", False),
    ("pnpm -F @erplora/web exec playwright install chromium", False),
    ("# a comment about sudo apt-get update", False),
    ("echo 'see apt-get docs' && apt-cache policy libgtk-3-0", False),
]
for body, should_flag in FIXTURES:
    flagged = bool(unguarded(body))
    check(
        f"detector on fixture `{body.splitlines()[0]}`",
        flagged == should_flag,
        f"flagged={flagged}, want {should_flag}",
    )

files = sorted(
    set(glob.glob(os.path.join(repo_root, ".github/workflows/*.yml")))
    | set(glob.glob(os.path.join(repo_root, ".github/workflows/*.yaml")))
    | set(glob.glob(os.path.join(repo_root, ".github/actions/**/action.yml"), recursive=True))
    | set(glob.glob(os.path.join(repo_root, ".github/actions/**/action.yaml"), recursive=True))
)
check(".github/ holds workflows to scan", bool(files), "nothing matched — the scan would pass blind")

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
            if not isinstance(step, dict) or not step.get("run"):
                continue
            label = str(step.get("name") or f"step {index}")
            if not touches_apt(step["run"]):
                continue
            discovered.setdefault(rel_path, []).append(label)
            bad = unguarded(step["run"])
            check(
                f"{rel_path} · job `{job_name}` · step «{label}» waits out the apt lock",
                not bad,
                f"`{bad[0] if bad else ''}` runs apt without `bash {WRAPPER}` — on the shared "
                "runner another job's apt lock turns it red (infra#313)",
            )

for rel_path, labels in KNOWN.items():
    for label in labels:
        check(
            f"the scan still sees {rel_path} · «{label}»",
            label in discovered.get(rel_path, []),
            "not found — renamed, moved or retired? update KNOWN, or the detector went blind",
        )

for f in failures:
    print(f"FAIL {f}", file=sys.stderr)
print(f"workflows: {passed} passed, {len(failures)} failed")
sys.exit(1 if failures else 0)
PY
    if [ $? -eq 0 ]; then pass "2 · every apt step in .github/ goes through the wrapper"; else fail "2 · every apt step in .github/ goes through the wrapper"; fi
fi

printf '\n%d passed, %d failed\n' "$passed" "$failures"
[ "$failures" -eq 0 ]
