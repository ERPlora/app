#!/usr/bin/env bash
# Contract of .github/workflows/tauri-release.yml — run with:  bash scripts/tests/tauri-release-workflow.test.sh
#
# What it pins (ERPlora/app, 2026-10-14): a push to `main` builds, tests and publishes the app —
# Microsoft Store, Google Play, S3 (installers + `latest.json`) and a GitHub Release — but ONLY when
# the version in `src-tauri/tauri.conf.json` is new. The version is the single source of truth:
#
#   · `version` job (scripts/release-version.sh): tag `v<version>` exists → every other job skips;
#     missing and greater than the latest `v*` tag → release; lower → red.
#   · nothing builds before the tests (`test-shell.yml`, called) are green;
#   · S3 by OIDC with the repo Variables `AWS_DOWNLOADS_ROLE_ARN`, `DOWNLOADS_BUCKET`, `AWS_REGION`;
#     without them a visible warning, never a silent skip and never Hetzner's static keys;
#   · the store jobs fail LOUDLY when a secret is missing, naming it (scripts/ci/require-config.sh);
#   · the tag `v<version>` and the GitHub Release are created LAST, after the release gate.
#
# Parsed as YAML (PyYAML), so a case names what is missing. `--workflow <path>` lets the guard be
# proven on a mutated copy.
set -uo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
workflow="$repo_root/.github/workflows/tauri-release.yml"

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
import re
import sys

import yaml

path = os.environ["WORKFLOW"]
with open(path, encoding="utf-8") as fh:
    text = fh.read()
doc = yaml.safe_load(text)
# The workflow without its comment lines: a `#` line explaining a rule is not the rule.
code = "\n".join(line for line in text.splitlines() if not line.lstrip().startswith("#"))

failures = []
passed = 0


def check(name, ok, detail=""):
    global passed
    if ok:
        passed += 1
    else:
        failures.append(f"{name}{': ' + detail if detail else ''}")


jobs = doc.get("jobs") or {}
INSTALLERS = (
    "erplora-app-setup.exe",
    "erplora-app.msi",
    "erplora-app.AppImage",
    "erplora-app.deb",
    "erplora-app.dmg",
    "erplora-app.apk",
)
BUILD = "needs.version.outputs.build == 'true'"
RELEASE = "needs.version.outputs.release == 'true'"


def job(job_id):
    return jobs.get(job_id) or {}


def needs(job_id):
    value = job(job_id).get("needs") or []
    return [value] if isinstance(value, str) else list(value)


def cond(obj):
    return str(obj.get("if", "") or "")


def steps(job_id):
    return [s for s in (job(job_id).get("steps") or []) if isinstance(s, dict)]


def run_of(step):
    return str(step.get("run", "") or "")


def index_of(job_id, predicate):
    return next((i for i, s in enumerate(steps(job_id)) if predicate(s)), None)


def uses(step, action):
    return str(step.get("uses", "")).startswith(action)


# ── 1 · Triggers: every push to main, plus a manual dispatch ─────────────────────────────────
# YAML 1.1 turns the bare key `on` into the boolean True.
triggers = doc.get("on", doc.get(True)) or {}
push = triggers.get("push") or {}
check("the workflow runs on `push`", isinstance(triggers.get("push"), dict), f"triggers are {triggers!r}")
check("`push.branches` is exactly [main]", push.get("branches") == ["main"], f"got {push.get('branches')!r}")
check(
    "`push` has no `tags` filter (the tag is created BY the release, not its trigger)",
    "tags" not in push,
    f"push is {push!r}",
)
dispatch = triggers.get("workflow_dispatch")
check("`workflow_dispatch` is kept", isinstance(dispatch, dict), f"got {dispatch!r}")
publish_input = ((dispatch or {}).get("inputs") or {}).get("publish") or {}
check(
    "the dispatch keeps its `publish` boolean input, default true",
    publish_input.get("type") == "boolean" and publish_input.get("default") is True,
    f"publish input is {publish_input!r}",
)

# A newer push must never cancel a release halfway (after Play, before the tag): it waits.
concurrency = doc.get("concurrency") or {}
check(
    "`cancel-in-progress: false` — a push never cancels a release in flight",
    concurrency.get("cancel-in-progress") is False,
    f"cancel-in-progress is {concurrency.get('cancel-in-progress')!r}",
)
top_permissions = doc.get("permissions") or {}
check(
    "top-level permissions are `contents: read` only",
    top_permissions == {"contents": "read"},
    f"permissions are {top_permissions!r}",
)

# ── 2 · The version gate decides; nothing else reads a ref ───────────────────────────────────
version = job("version")
check("the workflow has the `version` job", bool(version), f"jobs are {list(jobs)}")
gate_step = index_of("version", lambda s: "scripts/release-version.sh" in run_of(s))
check("the `version` job runs scripts/release-version.sh", gate_step is not None)
if gate_step is not None:
    step = steps("version")[gate_step]
    check("the version gate step has an `id` (its outputs feed the job)", bool(step.get("id")))
    check(
        "the version gate reads the tags of the repository (`git ls-remote --tags`)",
        "git ls-remote --tags" in run_of(step),
        run_of(step),
    )
outputs = version.get("outputs") or {}
for key in ("version", "tag", "build", "release"):
    check(f"the `version` job exposes `{key}`", key in outputs, f"outputs are {list(outputs)}")

# (`refs/tags/` alone is legitimate: the version job parses `git ls-remote --tags` with it.)
for marker in ("refs/tags/v", "github.ref_name", "startsWith(github.ref"):
    check(
        f"no job reads the version from the ref (`{marker}`)",
        marker not in code,
        "the version comes only from tauri.conf.json, through the `version` job",
    )
check(
    "nothing rewrites tauri.conf.json during the build",
    "writeFileSync" not in code,
    "the file IS the version; a build that rewrites it ships a number nobody committed",
)

# ── 3 · Tests first: nothing builds on red ───────────────────────────────────────────────────
test = job("test")
check(
    "the `test` job calls the shell test workflow",
    str(test.get("uses", "")) == "./.github/workflows/test-shell.yml",
    f"uses is {test.get('uses')!r}",
)
check("the `test` job needs `version`", "version" in needs("test"), f"needs are {needs('test')}")
check("the `test` job only runs when something is built", BUILD in cond(test), f"if is {cond(test)!r}")
for job_id in ("build", "build-android"):
    check(f"`{job_id}` needs `test` (no build on red)", "test" in needs(job_id), f"needs are {needs(job_id)}")
    check(f"`{job_id}` needs `version`", "version" in needs(job_id), f"needs are {needs(job_id)}")
    check(f"`{job_id}` only runs when the version gate says build", BUILD in cond(job(job_id)), f"if is {cond(job(job_id))!r}")

# ── 4 · Publishing: only a release, only after BOTH builds ───────────────────────────────────
for job_id in ("publish-store", "publish-play", "upload-s3"):
    check(f"the workflow has `{job_id}`", bool(job(job_id)), f"jobs are {list(jobs)}")
    for wanted in ("version", "build", "build-android"):
        check(f"`{job_id}` needs `{wanted}`", wanted in needs(job_id), f"needs are {needs(job_id)}")
    check(f"`{job_id}` only runs on a release", RELEASE in cond(job(job_id)), f"if is {cond(job(job_id))!r}")

pack = index_of("build", lambda s: "pack-msix.ps1" in run_of(s))
check("the Windows leg packs the MSIX", pack is not None)
if pack is not None:
    check(
        "the MSIX takes the version from the `version` job",
        "needs.version.outputs.version" in run_of(steps("build")[pack]),
        run_of(steps("build")[pack]),
    )

# ── 5 · S3 by OIDC, never Hetzner ────────────────────────────────────────────────────────────
check("no Hetzner secret is left (`HETZNER_S3_*`)", "HETZNER_S3_" not in text)
check("no Hetzner endpoint in the code", "hetzner" not in code.lower() and "your-objectstorage" not in code)
check("no custom S3 endpoint (`--endpoint-url`)", "--endpoint-url" not in code)

s3 = job("upload-s3")
check(
    "`upload-s3` has `id-token: write` (OIDC) and `contents: read`",
    (s3.get("permissions") or {}) == {"contents": "read", "id-token": "write"},
    f"permissions are {s3.get('permissions')!r}",
)
others_with_oidc = [j for j, body in jobs.items() if j != "upload-s3" and "id-token" in ((body or {}).get("permissions") or {})]
check("no other job gets `id-token`", not others_with_oidc, f"also on {others_with_oidc}")

VARS_SET = ("vars.AWS_DOWNLOADS_ROLE_ARN != ''", "vars.DOWNLOADS_BUCKET != ''")
creds = index_of("upload-s3", lambda s: uses(s, "aws-actions/configure-aws-credentials@v4"))
check("`upload-s3` takes credentials with aws-actions/configure-aws-credentials@v4", creds is not None)
if creds is not None:
    step = steps("upload-s3")[creds]
    with_ = step.get("with") or {}
    check(
        "the role is the Variable `AWS_DOWNLOADS_ROLE_ARN`",
        str(with_.get("role-to-assume", "")).replace(" ", "") == "${{vars.AWS_DOWNLOADS_ROLE_ARN}}",
        f"role-to-assume is {with_.get('role-to-assume')!r}",
    )
    check(
        "the region is the Variable `AWS_REGION`",
        str(with_.get("aws-region", "")).replace(" ", "") == "${{vars.AWS_REGION}}",
        f"aws-region is {with_.get('aws-region')!r}",
    )
    check(
        "no static keys are handed to the credentials action",
        not any(k in with_ for k in ("aws-access-key-id", "aws-secret-access-key")),
        f"with is {with_!r}",
    )
    check("the credentials step runs only with both Variables set", all(v in cond(step) for v in VARS_SET), f"if is {cond(step)!r}")

upload = index_of("upload-s3", lambda s: "aws s3 cp" in run_of(s))
check("`upload-s3` has the step that uploads", upload is not None)
if upload is not None:
    step = steps("upload-s3")[upload]
    body = run_of(step)
    check("the upload runs only with both Variables set", all(v in cond(step) for v in VARS_SET), f"if is {cond(step)!r}")
    check("the upload comes after the credentials", creds is not None and creds < upload)
    check("the bucket is the Variable `DOWNLOADS_BUCKET`", "vars.DOWNLOADS_BUCKET" in str(step.get("env", "")) + body + str(s3.get("env", "")))
    check("installers land on `downloads/app/v<version>/` (the old layout)", "/downloads/app/v${VERSION}/" in body, body)
    check("the manifest lands on `downloads/app/latest/latest.json` (the key the SaaS reads)", "/downloads/app/latest/latest.json" in body, body)
    for name in INSTALLERS:
        check(f"the upload ships `{name}`", name in body)
    cps = [line for line in body.splitlines() if "aws s3 cp" in line]
    check(
        "`latest/latest.json` is the LAST object written (never announces a missing installer)",
        bool(cps) and "/downloads/app/latest/latest.json" in cps[-1],
        f"last copy is {cps[-1] if cps else None!r}",
    )

skipped = index_of(
    "upload-s3",
    lambda s: "vars.AWS_DOWNLOADS_ROLE_ARN == ''" in cond(s) and "vars.DOWNLOADS_BUCKET == ''" in cond(s),
)
check("`upload-s3` has a step for the missing Variables", skipped is not None)
if skipped is not None:
    step = steps("upload-s3")[skipped]
    body = run_of(step)
    check("…which is an Actions warning", "::warning::" in body, body)
    check("…written to the job summary", "GITHUB_STEP_SUMMARY" in body, body)
    check(
        "…saying exactly what is missing",
        "S3 upload skipped: AWS_DOWNLOADS_ROLE_ARN/DOWNLOADS_BUCKET not set" in body,
        body,
    )
    check("…and reporting the channel as skipped to the gate", "result=skipped" in body, body)
check("`upload-s3` reports what it did (`outputs.result`)", "result" in (s3.get("outputs") or {}), f"outputs are {s3.get('outputs')!r}")

# ── 6 · Missing secrets are red, named ───────────────────────────────────────────────────────
def require_step(job_id, names):
    i = index_of(job_id, lambda s: "scripts/ci/require-config.sh" in run_of(s) and all(n in run_of(s) for n in names))
    check(f"`{job_id}` checks {', '.join(n.split(':')[1] for n in names)} with require-config.sh", i is not None)
    if i is None:
        return None
    step = steps(job_id)[i]
    check(f"`{job_id}`: the check cannot be skipped (no `if:`)", "if" not in step, f"if is {step.get('if')!r}")
    check(f"`{job_id}`: the check cannot be ignored (no `continue-on-error`)", not step.get("continue-on-error"))
    env = step.get("env") or {}
    for n in names:
        kind, name = n.split(":")
        source = "secrets" if kind == "secret" else "vars"
        check(
            f"`{job_id}`: {name} reaches the check from `{source}.{name}`",
            str(env.get(name, "")).replace(" ", "") == "${{" + f"{source}.{name}" + "}}",
            f"env is {env!r}",
        )
    checkout = index_of(job_id, lambda s: uses(s, "actions/checkout@"))
    check(f"`{job_id}` checks out the repo before the check (the script lives in it)", checkout is not None and checkout < i)
    return i


store_check = require_step(
    "publish-store",
    [
        "secret:AZURE_AD_TENANT_ID",
        "secret:SELLER_ID",
        "secret:AZURE_AD_APPLICATION_CLIENT_ID",
        "secret:AZURE_AD_APPLICATION_SECRET",
    ],
)
msstore = index_of("publish-store", lambda s: "msstore reconfigure" in run_of(s))
check("`publish-store` checks the secrets BEFORE `msstore reconfigure`", store_check is not None and msstore is not None and store_check < msstore)

play_check = require_step("publish-play", ["secret:PLAY_SERVICE_ACCOUNT_JSON"])
play_upload = index_of("publish-play", lambda s: uses(s, "r0adkll/upload-google-play"))
check("`publish-play` checks the secret BEFORE uploading", play_check is not None and play_upload is not None and play_check < play_upload)

keystore = require_step("build-android", ["secret:ANDROID_KEYSTORE_BASE64", "secret:ANDROID_KEYSTORE_PASSWORD"])
if keystore is not None:
    check(
        "the keystore step writes no secret into its script text",
        "secrets." not in run_of(steps("build-android")[keystore]),
        "a `${{ secrets.X }}` inside `run:` is pasted into the script before bash parses it",
    )
check("no job or step may `continue-on-error`", "continue-on-error" not in code)

# ── 7 · The gate sees the S3 channel as it really went ───────────────────────────────────────
gate = job("release-gate")
check("the release gate runs only on a release", RELEASE in cond(gate) and "always()" in cond(gate), f"if is {cond(gate)!r}")
for wanted in ("version", "test", "build", "build-android", "publish-store", "publish-play", "upload-s3"):
    check(f"the release gate needs `{wanted}`", wanted in needs("release-gate"), f"needs are {needs('release-gate')}")
gate_env = ""
for s in steps("release-gate"):
    gate_env += str((s.get("env") or {}).get("CHANNELS", "")) + "\n" + str((s.get("env") or {}).get("BUILDS", ""))
s3_line = next((l for l in gate_env.splitlines() if l.strip().startswith("s3|")), "")
check("the gate's s3 channel reads what the upload REALLY did (`outputs.result`)", "needs.upload-s3.outputs.result" in s3_line, s3_line)
check(
    "the gate's s3 channel is gated by the two Variables (so it can be declared pending)",
    "AWS_DOWNLOADS_ROLE_ARN" in s3_line and "DOWNLOADS_BUCKET" in s3_line,
    s3_line,
)
check("the gate also reports the tests", "needs.test.result" in gate_env, gate_env)

# ── 8 · The tag and the GitHub Release come last ─────────────────────────────────────────────
rel = job("github-release")
check("the workflow has the `github-release` job", bool(rel), f"jobs are {list(jobs)}")
check("`github-release` needs the release gate", "release-gate" in needs("github-release"), f"needs are {needs('github-release')}")
check("`github-release` needs `version`", "version" in needs("github-release"), f"needs are {needs('github-release')}")
check(
    "`github-release` runs only on a release whose gate is green",
    RELEASE in cond(rel) and "needs.release-gate.result == 'success'" in cond(rel),
    f"if is {cond(rel)!r}",
)
# The implicit `success()` is also false when an ANCESTOR was skipped — a store channel declared
# pending is one — so without `always()` no version would be tagged while a channel is pending.
check("`github-release` evaluates with `always()` (pending channels upstream are skipped jobs)", "always()" in cond(rel), f"if is {cond(rel)!r}")
check(
    "`github-release` is the only job with `contents: write`",
    (rel.get("permissions") or {}) == {"contents": "write"}
    and not [j for j, b in jobs.items() if j != "github-release" and ((b or {}).get("permissions") or {}).get("contents") == "write"],
    f"permissions are {rel.get('permissions')!r}",
)
create = index_of("github-release", lambda s: "gh release create" in run_of(s))
check("`github-release` creates the release with `gh release create`", create is not None)
if create is not None:
    step = steps("github-release")[create]
    body = run_of(step)
    env = step.get("env") or {}
    check("…for the tag of the `version` job", "needs.version.outputs.tag" in str(env), f"env is {env!r}")
    check("…on the commit that was built (`--target`)", "--target" in body, body)
    for name in INSTALLERS:
        check(f"…attaching `{name}`", name in body)
download = index_of("github-release", lambda s: uses(s, "actions/download-artifact@"))
check("`github-release` downloads the built installers first", download is not None and create is not None and download < create)

if failures:
    print(f"FAIL: {len(failures)} contract case(s) on {path}", file=sys.stderr)
    for f in failures:
        print(f"  - {f}", file=sys.stderr)
    sys.exit(1)

print(f"PASS: {passed} tauri-release workflow contract cases")
PY
