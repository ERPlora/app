#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Contract test for the Play reviewer preflight inside the `publish-play` job of
# `.github/workflows/tauri-release.yml` — ERPlora/hub#1888.
#
# WHY. hub#1718 wrote `scripts/ci/play-reviewer-preflight.py`: it proves the account Google reviews
# the app with really signs in and lands on a living hub with modules. Until this test existed it
# only ran when the person publishing remembered step 0 of `GOOGLE-PLAY.md`. The
# `publish-play` job uploads with `changesNotSentForReview: false`, so EVERY `v*` tag sends the app
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
#                                    failure as `continue-on-error`, one line lower (rv-1993).
#   · without `actions/checkout`   → the script does not exist on the runner.
#
# Hermetic: reads the workflow, never runs it. Wired in `.github/workflows/actionlint.yml`.
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

echo "hub#1888 — la guardia del revisor de Play corre dentro de \`${JOB}\`"

code=$(job_code)
if [ -z "$code" ]; then
    bad "tauri-release.yml define el job \`${JOB}\`" "sin el job no hay nada que proteger — ¿se ha renombrado?"
    printf '\nFAIL: %s caso(s) de contrato\n' "$fail"
    exit 1
fi
ok "tauri-release.yml define el job \`${JOB}\`"

step=$(preflight_step)
if [ -n "$step" ]; then
    ok "ejecuta \`${SCRIPT}\`"
else
    bad "ejecuta \`${SCRIPT}\`" \
        "sin el paso, la cuenta de revisión solo se comprueba si alguien se acuerda — y el que avisa es el rechazo de Google"
fi

if grep -qE 'uses:[[:space:]]*actions/checkout@' <<<"$code"; then
    ok "hace checkout del repo (el control vive en él)"
else
    bad "hace checkout del repo" "sin checkout, \`${SCRIPT}\` no existe en el runner y el paso muere sin mirar la cuenta"
fi

preflight_line=$(grep -m1 -nF "$SCRIPT" <<<"$code" | cut -d: -f1 || true)
upload_line=$(grep -m1 -nF "$UPLOAD_ACTION" <<<"$code" | cut -d: -f1 || true)
if [ -z "$upload_line" ]; then
    bad "el job sube con \`${UPLOAD_ACTION}\`" "este contrato mide el orden contra esa acción: si cambió, actualízalo a la nueva"
elif [ -n "$preflight_line" ] && [ "$preflight_line" -lt "$upload_line" ]; then
    ok "la guardia va ANTES de subir (cada tag manda a revisión)"
else
    bad "la guardia va ANTES de subir" \
        "\`changesNotSentForReview: false\`: después del upload la revisión ya está pedida y el rojo llega tarde"
fi

if grep -qE '^[[:space:]]+continue-on-error:[[:space:]]*true' <<<"$code"; then
    bad "no lleva \`continue-on-error: true\`" "el tag saldría verde con la cuenta de revisión rota"
else
    ok "no lleva \`continue-on-error: true\`"
fi

if [ -n "$step" ] && grep -qE '^[[:space:]]+if:' <<<"$step"; then
    bad "el paso de la guardia no lleva \`if:\`" \
        "un \`if:\` convierte la falta de configuración en un salto mudo; el control ya sale rojo \`missing_credentials\`"
else
    ok "el paso de la guardia no se puede saltar con un \`if:\`"
fi

# The `run:` must be the bare script on one line: `|| true`, `; exit 0` or a `run: |` block with
# `set +e` let the script say NO while the step turns green — the same silent failure as
# `continue-on-error`, one line lower. Only its exit code decides, so nothing may sit around it.
if [ -n "$step" ] && grep -qE "^[[:space:]]+run:[[:space:]]*python3[[:space:]]+${SCRIPT}[[:space:]]*$" <<<"$step"; then
    ok "el \`run:\` es solo el script: su código de salida decide"
else
    bad "el \`run:\` es solo el script: su código de salida decide" \
        "con \`|| true\`, \`exit 0\` o \`set +e\` alrededor, el control dice NO y el paso sale verde igual"
fi

if grep -qE 'PLAY_REVIEWER_PASSWORD:[[:space:]]*\$\{\{[[:space:]]*secrets\.PLAY_REVIEWER_PASSWORD[[:space:]]*\}\}' <<<"$step"; then
    ok "la contraseña sale de \`secrets.PLAY_REVIEWER_PASSWORD\`"
else
    bad "la contraseña sale de \`secrets.PLAY_REVIEWER_PASSWORD\`" "un secreto escrito en el workflow es un secreto público"
fi

for var in PLAY_REVIEWER_EMAIL PLAY_REVIEWER_HUB; do
    if grep -qE "${var}:[[:space:]]*\\\$\{\{[[:space:]]*(vars|secrets)\.${var}[[:space:]]*\}\}" <<<"$step"; then
        ok "\`${var}\` sale de la configuración del repo (Variables o secrets)"
    else
        bad "\`${var}\` sale de la configuración del repo" \
            "escrito a pelo seguiría pasando cuando la cuenta de revisión cambie — así sobrevivió un slug muerto en el .env (09/09)"
    fi
done

printf '\n%s ok · %s fallo(s)\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
