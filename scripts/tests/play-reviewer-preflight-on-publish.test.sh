#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Contract test for the Play reviewer preflight inside the `publish-play` job of
# `.github/workflows/tauri-release.yml`.
#
# WHY. `scripts/ci/play-reviewer-preflight.py` proves the account Google reviews the app with
# really signs in and lands on a living hub with modules. Outside the job it only runs when the
# person publishing remembers step 0 of `GOOGLE-PLAY.md`. The `publish-play` job uploads with `changesNotSentForReview: false`, so EVERY `v*` tag sends the app
# to review — with a broken reviewer account, the only one who reports it is Google's rejection,
# a whole round later.
#
# WHAT THE STEP MUST NOT BECOME, and why each one is worse than no step at all:
#
#   · after the upload             → the review was already requested: the red arrives too late.
#   · `continue-on-error: true`    → the tag goes green with the account broken — a silent failure.
#   · guarded by an `if:`          → missing configuration becomes a silent skip; the control
#                                    already answers `missing_credentials` naming the variables,
#                                    and that red is the point (same rule as `FLEET_API_KEY`).
#   · password written in the YAML → a secret in the file is a public secret.
#   · `run: … || true`, `set +e`   → the script says NO and the step stays green: same silent
#                                    failure as `continue-on-error`, one line lower.
#   · without `actions/checkout`   → the script does not exist on the runner.
#
# Hermetic: reads the workflow, never runs it. Run by the `scripts` job of `.github/workflows/test-shell.yml`.
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
workflow="${WORKFLOW:-$repo_root/.github/workflows/tauri-release.yml}"

JOB="publish-play"
SCRIPT="scripts/ci/play-reviewer-preflight.py"
UPLOAD_ACTION="r0adkll/upload-google-play"

pass=0
fail=0

ok() { printf '  \033[32m✓\033[0m %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  \033[31m✗\033[0m %s\n     %s\n' "$1" "$2"; fail=$((fail + 1)); }

# The job's lines, comments dropped: a commented-out step must not satisfy the contract.
job_code() {
    awk -v job="  ${JOB}:" '
        $0 == job {inside=1; next}
        inside && /^  [A-Za-z]/ {exit}
        inside {print}
    ' "$workflow" | grep -v '^[[:space:]]*#' || true
}

# The step (from its `- ` line to the next step) that runs the preflight script.
preflight_step() {
    awk -v script="$SCRIPT" '
        /^      - / { if (found) exit; step = "" }
        { step = step $0 "\n" }
        index($0, script) { found = 1 }
        END { if (found) printf "%s", step }
    ' <<<"$code"
}

echo "the Play reviewer guard runs inside \`${JOB}\`"

code=$(job_code)
if [ -z "$code" ]; then
    bad "tauri-release.yml defines the job \`${JOB}\`" "without the job there is nothing to protect — was it renamed?"
    printf '\nFAIL: %s contract case(s)\n' "$fail"
    exit 1
fi
ok "tauri-release.yml defines the job \`${JOB}\`"

step=$(preflight_step)
if [ -n "$step" ]; then
    ok "runs \`${SCRIPT}\`"
else
    bad "runs \`${SCRIPT}\`" \
        "without the step, the review account is only checked if somebody remembers — and Google's rejection is what tells"
fi

if grep -qE 'uses:[[:space:]]*actions/checkout@' <<<"$code"; then
    ok "checks out the repo (the control lives in it)"
else
    bad "checks out the repo" "without the checkout, \`${SCRIPT}\` does not exist on the runner and the step dies without looking at the account"
fi

preflight_line=$(grep -m1 -nF "$SCRIPT" <<<"$code" | cut -d: -f1 || true)
upload_line=$(grep -m1 -nF "$UPLOAD_ACTION" <<<"$code" | cut -d: -f1 || true)
if [ -z "$upload_line" ]; then
    bad "the job uploads with \`${UPLOAD_ACTION}\`" "this contract measures the order against that action: if it changed, update it to the new one"
elif [ -n "$preflight_line" ] && [ "$preflight_line" -lt "$upload_line" ]; then
    ok "the guard runs BEFORE the upload (every tag sends the app to review)"
else
    bad "the guard runs BEFORE the upload" \
        "\`changesNotSentForReview: false\`: after the upload the review is already requested and the red comes too late"
fi

if grep -qE '^[[:space:]]+continue-on-error:[[:space:]]*true' <<<"$code"; then
    bad "has no \`continue-on-error: true\`" "the tag would go green with the review account broken"
else
    ok "has no \`continue-on-error: true\`"
fi

if [ -n "$step" ] && grep -qE '^[[:space:]]+if:' <<<"$step"; then
    bad "the guard step has no \`if:\`" \
        "an \`if:\` turns missing configuration into a silent skip; the control already goes red with \`missing_credentials\`"
else
    ok "the guard step cannot be skipped with an \`if:\`"
fi

# The `run:` must be the bare script on one line: `|| true`, `; exit 0` or a `run: |` block with
# `set +e` let the script say NO while the step turns green — the same silent failure as
# `continue-on-error`, one line lower. Only its exit code decides, so nothing may sit around it.
if [ -n "$step" ] && grep -qE "^[[:space:]]+run:[[:space:]]*python3[[:space:]]+${SCRIPT}[[:space:]]*$" <<<"$step"; then
    ok "the \`run:\` is the bare script: its exit code decides"
else
    bad "the \`run:\` is the bare script: its exit code decides" \
        "with \`|| true\`, \`exit 0\` or \`set +e\` around it, the control says NO and the step goes green anyway"
fi

if grep -qE 'PLAY_REVIEWER_PASSWORD:[[:space:]]*\$\{\{[[:space:]]*secrets\.PLAY_REVIEWER_PASSWORD[[:space:]]*\}\}' <<<"$step"; then
    ok "the password comes from \`secrets.PLAY_REVIEWER_PASSWORD\`"
else
    bad "the password comes from \`secrets.PLAY_REVIEWER_PASSWORD\`" "a secret written in the workflow is a public secret"
fi

for var in PLAY_REVIEWER_EMAIL PLAY_REVIEWER_HUB; do
    if grep -qE "${var}:[[:space:]]*\\\$\{\{[[:space:]]*(vars|secrets)\.${var}[[:space:]]*\}\}" <<<"$step"; then
        ok "\`${var}\` comes from the repo configuration (Variables or secrets)"
    else
        bad "\`${var}\` comes from the repo configuration" \
            "hardcoded, it would keep passing after the review account changes, the way a dead slug survives in a .env"
    fi
done

printf '\n%s ok · %s failure(s)\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
