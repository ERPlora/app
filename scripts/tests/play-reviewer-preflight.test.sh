#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Behaviour test for `scripts/ci/play-reviewer-preflight.py` — the control that answers, BEFORE
# the app is sent to Google, whether the review account really gets inside a living business
# (ERPlora/hub#1718).
#
# WHY THE CONTROL EXISTS. The instructions handed to Google promise that signing in with the test
# account shows the app working. On 2026-09-09 that was measured and it was FALSE: the account
# reached the empty «Crea tu negocio gratis» screen, and then — once a hub existed — an EMPTY hub,
# «Aquí aparecerán tus apps», zero modules installed. A reviewer who cannot see the app rejects it,
# and a rejection costs another whole round. Nothing in the repo asked the question: `git grep
# PLAY_REVIEWER origin/develop` returned zero results, so the app could be sent again with the
# review account broken and nobody would know until Google answered.
#
# WHAT THE CONTROL MUST NOT BECOME, and why each one is worse than having no control at all:
#
#   · skipping when unconfigured  → the exact bug one level up: the check quietly stops happening
#                                   and the submission goes out unproven. Missing configuration is
#                                   a LOUD red naming the variables to set — the same criterion
#                                   `FLEET_API_KEY` gets in `build-hub.yml` (canary-on-publish).
#   · a hardcoded slug            → it would keep passing after the `.env` is pointed somewhere
#                                   else, which is precisely how a stale `PLAY_REVIEWER_HUB`
#                                   survived in the `.env` while its address answered 404 (09/09).
#   · a stub that says yes to all → the fake SaaS CHECKS email + password on the login and
#                                   demands the Bearer on `/api/v1/hubs/`, like the real one. With a
#                                   stub that accepted anything, «send no password» and «list the
#                                   hubs without the session» survived as mutants (rv-1885).
#   · trusting «the hub exists»   → creating the hub was not enough on 09/09. A hub with zero
#                                   modules is the shell Google saw: `checks.modules.registered`
#                                   is what separates «there is a hub» from «there is an app».
#   · trusting «status 200»       → the SaaS answers 200 for a hub whose runtime is degraded. The
#                                   verdict is `status: UP` inside the body, not the HTTP code.
#   · printing the password       → the credential reaches a CI log, a PR or a terminal scrollback.
#                                   It is never printed, not even on the failure paths.
#
# Every case here is hermetic: a fake SaaS and a fake hub on 127.0.0.1, no network, no credentials.
# The real run needs the reviewer account and is a person's step before submitting, documented in
# `GOOGLE-PLAY.md`.
#
# Run:  bash scripts/tests/play-reviewer-preflight.test.sh
# Dependency-free: bash + python3's stdlib.
# ─────────────────────────────────────────────────────────────────────────────
set -uo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
script="$repo_root/scripts/ci/play-reviewer-preflight.py"
tmp=$(mktemp -d)
stub_pid=""
cleanup() {
    [ -n "$stub_pid" ] && kill "$stub_pid" 2>/dev/null
    rm -rf "$tmp"
}
trap cleanup EXIT

pass=0
fail=0
ok()  { printf '  \033[32m✓\033[0m %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  \033[31m✗\033[0m %s\n     %s\n' "$1" "$2"; fail=$((fail + 1)); }

# A password shaped like a real one, so «the output never leaks it» is a real assertion and not a
# match on an empty string. It is fake: it only ever reaches the stub on 127.0.0.1.
FAKE_EMAIL='reviewer@example.invalid'
FAKE_PASSWORD='s3cr3t-not-a-real-one-8f2c'
# The account's hub in every scenario, and a slug that is NOT the account's. Both fake on purpose:
# the production slug lives ONLY in the root `.env` (PLAY_REVIEWER_HUB) — not here, not in the
# control, not in the doc. A test that carried it would be one more place to keep in sync.
FAKE_SLUG='negocio-de-prueba'
GHOST_SLUG='hub-que-no-es-suyo'

echo "hub#1718 — la guardia previa al envío a Google Play"

if [ ! -f "$script" ]; then
    bad "scripts/ci/play-reviewer-preflight.py existe" \
        "sin él, la app se puede volver a enviar a Google con la cuenta de revisión rota y nadie se entera hasta el rechazo"
    printf '\n%d passed, %d failed\n' "$pass" "$fail"
    exit 1
fi
ok "scripts/ci/play-reviewer-preflight.py existe"

# ── The fake SaaS + fake hub ─────────────────────────────────────────────────
# One server plays both planes, routed by path: `/api/v1/…` is the SaaS, `/hub/<slug>/readyz` is
# that hub's runtime. It records the User-Agent of the login so the Cloudflare trap has a test.
cat > "$tmp/stub.py" <<'STUB'
import json, sys
from http.server import BaseHTTPRequestHandler, HTTPServer

scenario = json.load(open(sys.argv[1]))
port_file, ua_log = sys.argv[2], sys.argv[3]
expected_email, expected_password = sys.argv[4], sys.argv[5]
TOKEN = "tok"
HUB_SESSION = "hub-session"


class H(BaseHTTPRequestHandler):
    def _send(self, code, body):
        raw = body if isinstance(body, bytes) else json.dumps(body).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def do_POST(self):
        if self.path == "/api/v1/auth/login/":
            with open(ua_log, "a") as fh:
                fh.write(self.headers.get("User-Agent", "") + "\n")
            raw = self.rfile.read(int(self.headers.get("content-length", 0) or 0))
            try:
                sent = json.loads(raw or b"{}")
            except ValueError:
                sent = {}
            code = scenario.get("login_status", 200)
            if code != 200:
                return self._send(code, {"detail": "no"})
            # The real SaaS checks the credentials; so does this one. A control that stopped
            # sending the password (or sent it under another name) must not pass the good case.
            if sent.get("email") != expected_email or sent.get("password") != expected_password:
                return self._send(401, {"detail": "bad credentials"})
            if scenario.get("login_without_access"):
                return self._send(200, {"user": {"email": expected_email, "id": "1"}})
            return self._send(200, {"access": TOKEN, "refresh": "r", "user": {"email": expected_email, "id": "1"}})
        if self.path.startswith("/hub/") and self.path.endswith("/api/auth/cloud"):
            # The hub's own door for a SaaS account (hub#2549): the SaaS `access` goes in as the
            # Bearer and a hub session comes back as `token`, like at the real runtime.
            slug = self.path[len("/hub/"):-len("/api/auth/cloud")]
            if slug not in scenario.get("readyz", {}):
                return self._send(404, {"detail": "no such hub"})
            if self.headers.get("Authorization") != f"Bearer {TOKEN}":
                return self._send(401, {"ok": False, "code": "cloud_token_invalid"})
            code = scenario.get("cloud_session_status", 200)
            if code != 200:
                return self._send(code, {"ok": False, "code": "cloud_login_refused"})
            return self._send(200, {"ok": True, "token": HUB_SESSION})
        self._send(404, {"detail": "nope"})

    def do_GET(self):
        if self.path == "/api/v1/hubs/":
            # JWT-only door, like the real one: without the session token the list is a 401.
            if self.headers.get("Authorization") != f"Bearer {TOKEN}":
                return self._send(401, {"detail": "Authentication credentials were not provided."})
            return self._send(
                scenario.get("hubs_status", 200),
                {"hubs": [{"id": "1", "slug": s, "name": s} for s in scenario.get("hubs", [])]},
            )
        if self.path.startswith("/hub/") and self.path.endswith("/readyz"):
            slug = self.path[len("/hub/"):-len("/readyz")]
            spec = scenario.get("readyz", {}).get(slug)
            if spec is None:
                return self._send(404, {"detail": "no such hub"})
            body = spec.get("body", {})
            # Like the real runtime since hub#2549: the inside of each check (the module count
            # among it) only reaches an owner/admin session; anybody else reads the statuses.
            admin = scenario.get("session_is_admin", True)
            if not (admin and self.headers.get("X-Hub-Session") == HUB_SESSION) and isinstance(body.get("checks"), dict):
                body = dict(body, checks={
                    name: {"status": check.get("status")} if isinstance(check, dict) else check
                    for name, check in body["checks"].items()
                })
            return self._send(spec.get("code", 200), body)
        self._send(404, {"detail": "nope"})

    def log_message(self, *a):
        pass


srv = HTTPServer(("127.0.0.1", 0), H)
with open(port_file, "w") as fh:
    fh.write(str(srv.server_address[1]))
srv.serve_forever()
STUB

# A hub that is exactly what a reviewer must find: up, and with its modules registered.
healthy_body='{"status":"UP","version":"1.1.23","checks":{"database":{"status":"UP"},"migrations":{"applied":112,"status":"UP"},"modules":{"expected":14,"failed":[],"missing":[],"registered":14,"status":"UP"}}}'

start_stub() { # $1 = scenario json
    [ -n "$stub_pid" ] && kill "$stub_pid" 2>/dev/null
    stub_pid=""
    rm -f "$tmp/port" "$tmp/ua"
    printf '%s' "$1" > "$tmp/scenario.json"
    python3 "$tmp/stub.py" "$tmp/scenario.json" "$tmp/port" "$tmp/ua" "$FAKE_EMAIL" "$FAKE_PASSWORD" &
    stub_pid=$!
    for _ in $(seq 1 100); do
        [ -s "$tmp/port" ] && break
        sleep 0.05
    done
    if [ ! -s "$tmp/port" ]; then
        printf 'FAIL: el servidor falso no arrancó\n' >&2
        exit 1
    fi
    port=$(cat "$tmp/port")
}

# run_preflight <env-file> [hub-url-template] [saas-url] → captures stdout+stderr in $out and the
# exit code in $rc. The env file is what the control reads when the variables are not already
# exported: pointing it at a scratch file is what keeps the real root `.env` (and the real
# credentials) out of the test.
run_preflight() {
    out=$(env -u PLAY_REVIEWER_EMAIL -u PLAY_REVIEWER_PASSWORD -u PLAY_REVIEWER_HUB \
        PLAY_REVIEWER_ENV_FILE="$1" \
        PLAY_REVIEWER_SAAS_URL="${3:-http://127.0.0.1:$port}" \
        PLAY_REVIEWER_HUB_URL="${2:-http://127.0.0.1:$port/hub/{slug\}}" \
        python3 "$script" 2>&1)
    rc=$?
}

write_env() { # $1=file, $2=slug  (the credentials live only in this scratch file)
    cat > "$1" <<EOF
PLAY_REVIEWER_EMAIL=$FAKE_EMAIL
PLAY_REVIEWER_PASSWORD=$FAKE_PASSWORD
PLAY_REVIEWER_HUB=$2
EOF
}

# expect_fail <case> <env-file> <expected code> [hub-url-template] [saas-url]
expect_fail() {
    local name=$1 envfile=$2 want=$3 hub_url=${4:-} saas_url=${5:-}
    run_preflight "$envfile" "$hub_url" "$saas_url"
    if [ "$rc" -eq 0 ]; then
        bad "$name" "el control salió en VERDE (exit 0): dejaría enviar la app a Google con esto roto"
    elif [[ "$out" != *"$want"* ]]; then
        bad "$name" "falla, pero no con el código \`$want\`: $(printf '%s' "$out" | tr '\n' ' ' | cut -c1-200)"
    else
        ok "$name"
    fi
    if [[ "$out" == *"$FAKE_PASSWORD"* ]]; then
        bad "$name — la contraseña NO aparece en la salida" \
            "el control imprimió la credencial: acabaría en un log, en una PR o en el scrollback"
    fi
}

# ── 1. El caso bueno: la cuenta entra y cae dentro de un negocio vivo ────────
# Y es también la prueba de que el control MANDA lo que dice: el servidor falso rechaza el login
# si no llegan email y contraseña exactos, y la lista de hubs si no llega el Bearer de la sesión.
start_stub "{\"hubs\": [\"$FAKE_SLUG\"], \"readyz\": {\"$FAKE_SLUG\": {\"code\": 200, \"body\": $healthy_body}}}"
write_env "$tmp/env.ok" "$FAKE_SLUG"
run_preflight "$tmp/env.ok"
if [ "$rc" -eq 0 ]; then
    ok "cuenta con su hub, /readyz UP y 14 módulos → VERDE"
else
    bad "cuenta con su hub, /readyz UP y 14 módulos → VERDE" \
        "el control salió en rojo sobre el caso bueno (exit $rc): $(printf '%s' "$out" | tr '\n' ' ' | cut -c1-200)"
fi
if [[ "$out" == *"$FAKE_PASSWORD"* ]]; then
    bad "el caso bueno no imprime la contraseña" "el control imprimió la credencial"
else
    ok "el caso bueno no imprime la contraseña"
fi

# 🪤 Trampa medida el 16/09: con el User-Agent por defecto de urllib, Cloudflare contesta 403
# «error code: 1010» y el control moriría en el login sin llegar a mirar nada. Manda uno normal.
ua=$(cat "$tmp/ua" 2>/dev/null)
if [ -z "$ua" ]; then
    bad "el login manda un User-Agent propio" "el servidor falso no vio ninguna petición de login"
elif [[ "$ua" == Python-urllib* ]]; then
    bad "el login manda un User-Agent propio" \
        "manda \`$ua\`: Cloudflare lo corta con 403 «error code: 1010» y el control nunca llega al hub"
else
    ok "el login manda un User-Agent propio (Cloudflare corta el de urllib con un 1010)"
fi

# ── 2. EL TEST DEL TEST: el control tiene que CAZAR EL POSITIVO ──────────────
# Un control que solo sabe decir que sí no es un control. Cada caso de aquí abajo es un fallo real
# que Google vería, y el control tiene que salir en ROJO en todos.

# 2a. Un slug que no es de la cuenta — el caso real del 09/09: un `.env` apuntando a un hub que
# respondía 404.
start_stub "{\"hubs\": [\"$FAKE_SLUG\"], \"readyz\": {\"$FAKE_SLUG\": {\"code\": 200, \"body\": $healthy_body}}}"
write_env "$tmp/env.ghost" "$GHOST_SLUG"
expect_fail "🔴 un slug que no es de la cuenta → ROJO" "$tmp/env.ghost" "hub_not_in_account"
# Y lo dice con el slug que SÍ lo es: si no, quien lo lea no sabe qué poner en el `.env`.
if [[ "$out" == *"$FAKE_SLUG"* ]]; then
    ok "al fallar, nombra el slug que SÍ es de la cuenta"
else
    bad "al fallar, nombra el slug que SÍ es de la cuenta" \
        "no dice cuál es el bueno, así que no se sabe qué poner en PLAY_REVIEWER_HUB: $(printf '%s' "$out" | tr '\n' ' ' | cut -c1-200)"
fi

# 2b. El hub caído — un 502 del runtime.
start_stub "{\"hubs\": [\"$FAKE_SLUG\"], \"readyz\": {\"$FAKE_SLUG\": {\"code\": 502, \"body\": {}}}}"
write_env "$tmp/env.down" "$FAKE_SLUG"
expect_fail "🔴 el hub contesta 502 → ROJO" "$tmp/env.down" "hub_not_ready"
# Y el mensaje dice 502, no «200». Sin esto el caso lo aprueba igual el siguiente control —el
# `status` de dentro—, así que borrar la comprobación del código HTTP se quedaba en verde: lo
# cazó un mutante. Quien lea el rojo tiene que saber que el runtime ni contestó.
if [[ "$out" == *502* ]]; then
    ok "el rojo del 502 dice 502 (no lo aprueba de rebote el control del \`status\`)"
else
    bad "el rojo del 502 dice 502" \
        "el control no miró el código HTTP: lo cazó de rebote otro check y el mensaje miente sobre lo que pasó — $(printf '%s' "$out" | tr '\n' ' ' | cut -c1-160)"
fi

# 2c. El hub que no contesta nada — nada escuchando en esa dirección.
start_stub "{\"hubs\": [\"$FAKE_SLUG\"], \"readyz\": {}}"
write_env "$tmp/env.404" "$FAKE_SLUG"
expect_fail "🔴 el hub no existe en esa dirección (404) → ROJO" "$tmp/env.404" "hub_not_ready"

# 2c-bis. La dirección muerta: nada escuchando al otro lado. Es la forma real del hub muerto del
# `.env` del 09/09, y la que deja el control colgado si no trata el error de conexión.
start_stub "{\"hubs\": [\"$FAKE_SLUG\"], \"readyz\": {\"$FAKE_SLUG\": {\"code\": 200, \"body\": $healthy_body}}}"
write_env "$tmp/env.dead" "$FAKE_SLUG"
expect_fail "🔴 la dirección del hub no responde (nadie escuchando) → ROJO" \
    "$tmp/env.dead" "hub_unreachable" 'http://127.0.0.1:1/{slug}'

# 2d. Responde 200, pero el runtime se declara caído. El código HTTP no es el veredicto.
start_stub "{\"hubs\": [\"$FAKE_SLUG\"], \"readyz\": {\"$FAKE_SLUG\": {\"code\": 200, \"body\": {\"status\": \"DOWN\", \"checks\": {\"modules\": {\"registered\": 14}}}}}}"
write_env "$tmp/env.degraded" "$FAKE_SLUG"
expect_fail "🔴 200 pero \`status: DOWN\` → ROJO (el 200 no es el veredicto)" "$tmp/env.degraded" "hub_not_ready"

# 2e. 🔴 EL FALLO DEL 09/09: el hub está vivo y VACÍO. «Aquí aparecerán tus apps».
start_stub "{\"hubs\": [\"$FAKE_SLUG\"], \"readyz\": {\"$FAKE_SLUG\": {\"code\": 200, \"body\": {\"status\": \"UP\", \"version\": \"1.1.23\", \"checks\": {\"modules\": {\"registered\": 0, \"expected\": 0, \"status\": \"UP\"}}}}}}"
write_env "$tmp/env.empty" "$FAKE_SLUG"
expect_fail "🔴 hub vivo pero con CERO módulos → ROJO (es el cascarón que vio Google el 09/09)" \
    "$tmp/env.empty" "hub_without_modules"

# 2f. La cuenta se quedó sin ningún hub — el punto de partida de la issue.
start_stub '{"hubs": [], "readyz": {}}'
write_env "$tmp/env.nohubs" "$FAKE_SLUG"
expect_fail "🔴 la cuenta no tiene ningún hub → ROJO (la pantalla de alta que vio el revisor)" \
    "$tmp/env.nohubs" "no_hubs"

# 2g. Las credenciales dejaron de valer.
start_stub '{"login_status": 401, "hubs": [], "readyz": {}}'
write_env "$tmp/env.badcreds" "$FAKE_SLUG"
expect_fail "🔴 el SaaS rechaza el login → ROJO" "$tmp/env.badcreds" "login_rejected"
# Mismo motivo que en el 502: sin nombrar el 401, borrar la comprobación del código dejaba el caso
# en verde porque lo recogía el «200 sin access» de después. También lo cazó un mutante.
if [[ "$out" == *401* ]]; then
    ok "el rojo del login dice 401 (no lo aprueba de rebote el control del \`access\`)"
else
    bad "el rojo del login dice 401" \
        "el control no miró el código HTTP del login — $(printf '%s' "$out" | tr '\n' ' ' | cut -c1-160)"
fi

# 2g-bis. El SaaS contesta 200 al login pero sin sesión (`access`): no hay con qué seguir.
start_stub "{\"login_without_access\": true, \"hubs\": [\"$FAKE_SLUG\"], \"readyz\": {\"$FAKE_SLUG\": {\"code\": 200, \"body\": $healthy_body}}}"
write_env "$tmp/env.noaccess" "$FAKE_SLUG"
expect_fail "🔴 login 200 pero sin \`access\` → ROJO" "$tmp/env.noaccess" "login_rejected"

# 2h. El SaaS no contesta nada — nadie escuchando. Sin este caso «SaaS caído = verde» sobrevivía
# como mutante (rv-1885): el control no podía comprobar nada y aun así dejaba enviar.
start_stub "{\"hubs\": [\"$FAKE_SLUG\"], \"readyz\": {\"$FAKE_SLUG\": {\"code\": 200, \"body\": $healthy_body}}}"
write_env "$tmp/env.saasdown" "$FAKE_SLUG"
expect_fail "🔴 el SaaS no responde (nadie escuchando) → ROJO" "$tmp/env.saasdown" "saas_unreachable" "" "http://127.0.0.1:1"

# 2i. 🔴 hub#2549: el hub ya no cuenta sus módulos a quien no ha entrado. El control entra en el
# hub con la propia cuenta (la misma puerta que usa el revisor); si el hub no le abre sesión, no
# puede contar nada, y eso es ROJO, nunca «cero módulos» ni un verde.
start_stub "{\"cloud_session_status\": 403, \"hubs\": [\"$FAKE_SLUG\"], \"readyz\": {\"$FAKE_SLUG\": {\"code\": 200, \"body\": $healthy_body}}}"
write_env "$tmp/env.nosession" "$FAKE_SLUG"
expect_fail "🔴 el hub no abre sesión a la cuenta de revisión → ROJO" "$tmp/env.nosession" "hub_session_refused"

# 2j. La sesión se abre pero la cuenta no administra el hub: /readyz le da solo los estados y el
# recuento de módulos no llega. No se puede comprobar → ROJO con su código, no «sin módulos».
start_stub "{\"session_is_admin\": false, \"hubs\": [\"$FAKE_SLUG\"], \"readyz\": {\"$FAKE_SLUG\": {\"code\": 200, \"body\": $healthy_body}}}"
write_env "$tmp/env.notadmin" "$FAKE_SLUG"
expect_fail "🔴 la cuenta no administra su hub (sin detalle en /readyz) → ROJO" "$tmp/env.notadmin" "hub_detail_withheld"

# ── 3. Sin válvula: faltar configuración es ROJO, nunca un skip silencioso ───
start_stub "{\"hubs\": [\"$FAKE_SLUG\"], \"readyz\": {\"$FAKE_SLUG\": {\"code\": 200, \"body\": $healthy_body}}}"
: > "$tmp/env.empty-file"
expect_fail "🔴 sin credenciales → ROJO, no un skip en verde" "$tmp/env.empty-file" "missing_credentials"
# Y dice CÓMO ponerlas: un rojo que no dice qué falta se «arregla» borrando el control.
missing_out=$out
for var in PLAY_REVIEWER_EMAIL PLAY_REVIEWER_PASSWORD PLAY_REVIEWER_HUB; do
    if [[ "$missing_out" == *"$var"* ]]; then
        ok "el rojo por configuración nombra \`$var\`"
    else
        bad "el rojo por configuración nombra \`$var\`" \
            "sin el nombre de la variable, quien lo vea no sabe qué poner y acaba saltándose el control"
    fi
done

# Media configuración es igual de roja que ninguna: un `.env` al que le falta el slug no puede
# comprobar nada, y es justo el estado en el que se «arregla» quitando la comprobación.
cat > "$tmp/env.half" <<EOF
PLAY_REVIEWER_EMAIL=reviewer@example.invalid
PLAY_REVIEWER_PASSWORD=$FAKE_PASSWORD
EOF
expect_fail "🔴 falta solo PLAY_REVIEWER_HUB → ROJO" "$tmp/env.half" "missing_credentials"

# ── 4. El slug sale del `.env`, no del código ───────────────────────────────
# Si el control llevara el slug escrito dentro, seguiría en verde con el `.env` apuntando a otro
# sitio — que es exactamente cómo el hub muerto del `.env` sobrevivió apuntando a un 404. Se mira
# el slug de este test y cualquier host de tenant escrito a pelo (`<slug>.a.erplora.com`); la
# plantilla `{slug}.a.erplora.com` del control no casa porque no lleva un slug delante.
if grep -qE "$FAKE_SLUG|[a-z0-9]([a-z0-9-]*[a-z0-9])?\.a\.erplora\.com" "$script"; then
    bad "el slug no está escrito en el código" \
        "un slug o un host de tenant aparece en el propio control: con el \`.env\` apuntando a otro hub seguiría dando verde"
else
    ok "el slug no está escrito en el código (sale de PLAY_REVIEWER_HUB)"
fi

printf '\n'
if [ "$fail" -gt 0 ]; then
    printf 'FAIL: %d caso(s) (%d ok)\n' "$fail" "$pass"
    exit 1
fi
printf 'OK: %d caso(s)\n' "$pass"
