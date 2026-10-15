#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Behaviour test for `scripts/ci/play-reviewer-preflight.py` — the control that answers, BEFORE
# the app is sent to Google, whether the review account really gets inside a living business
# (ERPlora/hub#1718).
#
# WHY THE CONTROL EXISTS. The instructions handed to Google promise that signing in with the test
# account shows the app working. On 2026-09-09 that was measured and it was FALSE: the account
# reached the empty «Crea tu negocio gratis» screen, and then — once a hub existed — an EMPTY hub,
# «Aquí aparecerán tus apps», zero modules. A reviewer who cannot see the app rejects it, and a
# rejection costs another whole round.
#
# WHICH PLATFORM. Since 2026-10-14 `erplora.com` is saas-next and the hubs are hub-next at
# `https://<slug>.erplora.com`. The fake servers below speak THEIR contract, read from their code:
#
#   · sign-in    allauth.headless, browser client: GET `/_allauth/browser/v1/config` hands out the
#                `csrftoken` cookie; POST `/_allauth/browser/v1/auth/login` wants it back as
#                `X-CSRFToken` with a same-origin `Origin`, and answers `meta.is_authenticated` plus
#                the `sessionid` cookie (saas-next `config/urls.py`, allauth `headless/internal`).
#   · businesses GET `/api/hubs/businesses` with that session (`hubs.business.list`, saas-next
#                `hubs/api.py`): `rows[]` with `slug`, `status`, `can_open`, `open_blocked_reason`.
#   · hub up     GET `<hub>/readyz` → `{"status": "ok"}` (hub-next `base/health.py`).
#   · the jump   GET `<hub>/auth/login` → 302 to the SaaS `/hub-login/?…&state=` with the
#                `hub_login_state` cookie; the SaaS answers a form with a one-time `code`; POST
#                `<hub>/auth/callback` opens the hub's session (hub-next `base/auth_views.py`,
#                saas-next `accounts/handoff.py`). The same door the reviewer's «Open» goes through.
#   · modules    GET `<hub>/api/me` with that session → `apps[]` (hub-next `base/queries.py`).
#
# WHAT THE CONTROL MUST NOT BECOME, and why each one is worse than having no control at all:
#
#   · skipping when unconfigured  → the check quietly stops happening and the submission goes out
#                                   unproven. Missing configuration is a LOUD red naming the
#                                   variables to set.
#   · a hardcoded slug            → it keeps passing after the `.env` points somewhere else, which
#                                   is how a stale `PLAY_REVIEWER_HUB` survived while its address
#                                   answered 404 (09/09).
#   · a stub that says yes to all → the fake SaaS CHECKS email + password, the CSRF pair and the
#                                   Origin on the login, and demands the session on the list; the
#                                   fake hub demands the state cookie on the callback and its own
#                                   session on `/api/me`, and refuses any cookie of the SaaS. With a
#                                   stub that accepted anything, «send no password» or «list without
#                                   the session» survived as mutants (rv-1885).
#   · trusting «the hub exists»   → a hub with zero modules is the shell Google saw: the modules
#                                   the account sees inside are what separates «a hub» from «an app».
#   · trusting «status 200»       → the verdict is the body, not the HTTP code.
#   · creating the account        → saas-next's unified sign-in (ADR-0154) signs UP an unknown
#                                   email posted to the login. While the SaaS says signup is open,
#                                   the control never posts the credentials.
#   · printing the password       → it is never printed, not even on the failure paths.
#
# Every case is hermetic: fake servers on 127.0.0.1, no network, no real credentials. The real run
# is a person's step before submitting (`GOOGLE-PLAY.md`, step 0) and the `publish-play` job's.
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

# A password shaped like a real one, so «the output never leaks it» is a real assertion. Fake: it
# only ever reaches the stub on 127.0.0.1.
FAKE_EMAIL='reviewer@example.invalid'
FAKE_PASSWORD='s3cr3t-not-a-real-one-8f2c'
# The account's hub in every scenario, and a slug that is NOT the account's. Both fake on purpose:
# the production slug lives ONLY in the root `.env` (PLAY_REVIEWER_HUB).
FAKE_SLUG='negocio-de-prueba'
GHOST_SLUG='hub-que-no-es-suyo'

echo "hub#1718 — the guard before sending the app to Google Play (saas-next + hub-next)"

if [ ! -f "$script" ]; then
    bad "scripts/ci/play-reviewer-preflight.py exists" \
        "without it the app can be sent to Google with the review account broken and nobody knows until the rejection"
    printf '\n%d passed, %d failed\n' "$pass" "$fail"
    exit 1
fi
ok "scripts/ci/play-reviewer-preflight.py exists"

# ── The fake SaaS + fake hub ─────────────────────────────────────────────────
# One server plays both, routed by path: `/_allauth/…`, `/api/hubs/…` and `/hub-login/` are the
# SaaS; `/hub/<slug>/…` is that hub. Each login POST is logged (with its User-Agent), so «the
# credentials were never sent» and the Cloudflare trap have a test.
cat > "$tmp/stub.py" <<'STUB'
import html, json, sys
from http.cookies import SimpleCookie
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import parse_qs, urlencode, urlsplit

scenario = json.load(open(sys.argv[1]))
port_file, login_log = sys.argv[2], sys.argv[3]
expected_email, expected_password = sys.argv[4], sys.argv[5]
CSRF = "csrf-of-the-saas"
SAAS_SESSION = "saas-session"
HUB_SESSION = "hub-session"
CODE = "one-time-code"
STATE = "state-of-this-browser"
HUBS = scenario.get("hubs", [])
LIVE = scenario.get("readyz", {})


class H(BaseHTTPRequestHandler):
    def origin(self):
        return f"http://127.0.0.1:{self.server.server_address[1]}"

    def cookies(self):
        jar = SimpleCookie(self.headers.get("Cookie", ""))
        return {name: morsel.value for name, morsel in jar.items()}

    def _send(self, code, body=b"", *, headers=(), content_type="application/json"):
        if isinstance(body, (dict, list)):
            body = json.dumps(body)
        raw = body.encode() if isinstance(body, str) else body
        self.send_response(code)
        self.send_header("content-type", content_type)
        for name, value in headers:
            self.send_header(name, value)
        self.send_header("content-length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def _redirect(self, location, *headers):
        self._send(302, b"", headers=[("Location", location), *headers], content_type="text/html")

    def _hub_path(self, path):
        """`/hub/<slug>/rest` → (`slug`, `/rest`)."""
        slug, _, rest = path[len("/hub/"):].partition("/")
        return slug, "/" + rest

    def _saas_cookie_leaked(self):
        """A browser never sends `erplora.com`'s host-only cookies to `<slug>.erplora.com`; Python's
        cookiejar does (measured 2026-10-15), and both sites call their session `sessionid`. Here
        both live on 127.0.0.1, so the fake hub refuses any cookie of the SaaS outright."""
        cookies = self.cookies()
        if "csrftoken" in cookies or cookies.get("sessionid") == SAAS_SESSION:
            self._send(400, {"error": {"code": "saas_cookie_leaked"}})
            return True
        return False

    def do_GET(self):
        url = urlsplit(self.path)
        query = {k: v[0] for k, v in parse_qs(url.query).items()}
        if url.path == "/_allauth/browser/v1/config":
            # allauth's browser views call get_token(): this is where the CSRF cookie comes from.
            return self._send(
                200,
                {"status": 200, "data": {"account": {
                    "login_methods": ["email"],
                    "is_open_for_signup": scenario.get("signup_open", False),
                }}},
                headers=[("Set-Cookie", f"csrftoken={CSRF}; Path=/; SameSite=Lax")],
            )
        if url.path == "/api/hubs/businesses":
            # Session-only door, like ninja's django_auth: without the cookie, the 401 envelope.
            if self.cookies().get("sessionid") != SAAS_SESSION:
                return self._send(401, {"error": {"code": "unauthorized", "message": "Unauthorized"}})
            blocked = scenario.get("blocked", {})
            rows = [
                {
                    "id": str(i), "slug": s, "name": s, "host": f"{s}.erplora.com",
                    "status": "preparing" if s in blocked else "active",
                    "role": "hub_owner",
                    "can_open": s not in blocked,
                    "open_blocked_reason": blocked.get(s),
                }
                for i, s in enumerate(HUBS)
            ]
            return self._send(
                scenario.get("hubs_status", 200),
                {"rows": rows, "total": len(rows), "limit": 100, "offset": 0,
                 "invitations": [], "ownership_offers": []},
            )
        if url.path == "/hub-login/":
            slug = query.get("hub", "")
            if self.cookies().get("sessionid") != SAAS_SESSION:
                return self._redirect(f"/login?{urlencode({'next': self.path})}")
            error = scenario.get("hub_login_error")
            if error or slug not in HUBS:
                return self._redirect(f"/login/error?{urlencode({'code': error or 'not_a_member', 'hub': slug})}")
            action = scenario.get("form_action") or f"{self.origin()}/hub/{slug}/auth/callback"
            fields = {"code": CODE, "next": query.get("next", "/"), "state": query.get("state", "")}
            inputs = "".join(
                f'<input type="hidden" name="{n}" value="{html.escape(v)}">' for n, v in fields.items()
            )
            page = (
                '<!DOCTYPE html><html><body><form id="hub-login-form" method="post" '
                f'action="{html.escape(action)}">{inputs}<p>Opening your business…</p></form>'
                '<script>document.getElementById("hub-login-form").submit();</script></body></html>'
            )
            return self._send(200, page, content_type="text/html; charset=utf-8")
        if url.path.startswith("/hub/"):
            if self._saas_cookie_leaked():
                return
            slug, rest = self._hub_path(url.path)
            spec = LIVE.get(slug)
            if spec is None:
                return self._send(404, {"error": {"code": "not_found"}})
            if rest == "/readyz":
                return self._send(spec.get("code", 200), spec.get("body", {"status": "ok"}))
            if rest == "/auth/login":
                # Like hub-next's login_redirect: the state goes in a cookie for the callback only.
                target = f"{self.origin()}/hub-login/?{urlencode({'hub': slug, 'next': query.get('next', '/'), 'state': STATE})}"
                return self._redirect(
                    target, ("Set-Cookie", f"hub_login_state={STATE}; Path=/hub/{slug}/auth/callback; HttpOnly")
                )
            if rest == "/api/me":
                signed_in = self.cookies().get("sessionid") == HUB_SESSION and not scenario.get("me_anonymous")
                if not signed_in:
                    return self._send(200, {"user": None, "scope": {"kind": "host", "key": slug},
                                            "permissions": [], "menu": [], "apps": []})
                body = {"user": {"id": "1", "label": "Reviewer"}, "scope": {"kind": "host", "key": slug},
                        "menu": [], "language": "es"}
                if not scenario.get("me_without_apps"):
                    body["apps"] = [{"label": a, "state": "on"} for a in scenario.get("apps", ["sales", "inventory"])]
                return self._send(200, body)
        self._send(404, {"detail": "nope"})

    def do_POST(self):
        url = urlsplit(self.path)
        raw = self.rfile.read(int(self.headers.get("content-length", 0) or 0))
        if url.path == "/_allauth/browser/v1/auth/login":
            with open(login_log, "a") as fh:
                fh.write(self.headers.get("User-Agent", "") + "\n")
            # Django's CsrfViewMiddleware: the cookie and the header must match, and an unsafe
            # request carries a same-origin Origin.
            cookie = self.cookies().get("csrftoken")
            if not cookie or cookie != self.headers.get("X-CSRFToken") or self.headers.get("Origin") != self.origin():
                return self._send(403, "<h1>Forbidden (CSRF)</h1>", content_type="text/html")
            if "login" in scenario:
                return self._send(scenario["login"]["code"], scenario["login"]["body"])
            try:
                sent = json.loads(raw or b"{}")
            except ValueError:
                sent = {}
            if sent.get("email") != expected_email or sent.get("password") != expected_password:
                return self._send(400, {"status": 400, "errors": [{
                    "message": "The email address and/or password you specified are not correct.",
                    "code": "email_password_mismatch", "param": "password"}]})
            if scenario.get("login_unauthenticated"):
                return self._send(200, {"status": 200, "data": {}, "meta": {"is_authenticated": False}})
            return self._send(
                200,
                {"status": 200, "data": {"user": {"email": expected_email}}, "meta": {"is_authenticated": True}},
                headers=[("Set-Cookie", f"sessionid={SAAS_SESSION}; Path=/; HttpOnly; SameSite=Lax")],
            )
        if url.path.startswith("/hub/"):
            if self._saas_cookie_leaked():
                return
            slug, rest = self._hub_path(url.path)
            if rest == "/auth/callback" and slug in LIVE:
                form = {k: v[0] for k, v in parse_qs(raw.decode()).items()}
                # hub-next redeems a code only with THIS browser's state, and only a good code.
                if self.cookies().get("hub_login_state") != form.get("state") or form.get("code") != CODE:
                    return self._send(403, "<p>Sign in again</p>", content_type="text/html")
                error = scenario.get("callback_error")
                if error:
                    return self._redirect(f"{self.origin()}/hub-login/?{urlencode({'hub': slug, 'error': error})}")
                # `next` is a path of the hub (hub-next's safe_next): here, under its prefix.
                return self._redirect(
                    f"/hub/{slug}{form.get('next', '/')}",
                    ("Set-Cookie", f"sessionid={HUB_SESSION}; Path=/; HttpOnly; SameSite=Lax"),
                )
        self._send(404, {"detail": "nope"})

    def log_message(self, *a):
        pass


srv = HTTPServer(("127.0.0.1", 0), H)
with open(port_file, "w") as fh:
    fh.write(str(srv.server_address[1]))
srv.serve_forever()
STUB

start_stub() { # $1 = scenario json
    [ -n "$stub_pid" ] && kill "$stub_pid" 2>/dev/null
    stub_pid=""
    rm -f "$tmp/port" "$tmp/logins"
    printf '%s' "$1" > "$tmp/scenario.json"
    python3 "$tmp/stub.py" "$tmp/scenario.json" "$tmp/port" "$tmp/logins" "$FAKE_EMAIL" "$FAKE_PASSWORD" &
    stub_pid=$!
    for _ in $(seq 1 100); do
        [ -s "$tmp/port" ] && break
        sleep 0.05
    done
    if [ ! -s "$tmp/port" ]; then
        printf 'FAIL: the fake server did not start\n' >&2
        exit 1
    fi
    port=$(cat "$tmp/port")
}

# A hub that is exactly what a reviewer must find: in the account, open, up, with modules.
healthy() { # $1 = extra scenario keys (with a trailing comma) or ""
    printf '{%s"hubs": ["%s"], "readyz": {"%s": {"code": 200, "body": {"status": "ok"}}}}' "${1:-}" "$FAKE_SLUG" "$FAKE_SLUG"
}

# run_preflight <env-file> [hub-url-template] [saas-url] → stdout+stderr in $out, exit code in $rc.
# The env file is what the control reads when the variables are not exported: a scratch file keeps
# the real root `.env` (and the real credentials) out of the test.
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

no_password_in_output() { # $1 = case name
    if [[ "$out" == *"$FAKE_PASSWORD"* ]]; then
        bad "$1 — the password does NOT appear in the output" \
            "the control printed the credential: it would end up in a log, a PR or the scrollback"
    fi
}

# expect_fail <case> <env-file> <expected code> [hub-url-template] [saas-url]
expect_fail() {
    local name=$1 envfile=$2 want=$3 hub_url=${4:-} saas_url=${5:-}
    run_preflight "$envfile" "$hub_url" "$saas_url"
    if [ "$rc" -eq 0 ]; then
        bad "$name" "the control came out GREEN (exit 0): it would let the app go to Google with this broken"
    elif [[ "$out" != *"$want"* ]]; then
        bad "$name" "it fails, but not with \`$want\`: $(printf '%s' "$out" | tr '\n' ' ' | cut -c1-240)"
    else
        ok "$name"
    fi
    no_password_in_output "$name"
}

# expect_said <case> <text>: the last red names what happened (a red that does not is «fixed» by
# deleting the check that caught it, or approved by the next check by accident — mutants did both).
expect_said() {
    if [[ "$out" == *"$2"* ]]; then
        ok "$1"
    else
        bad "$1" "the message does not say \`$2\`: $(printf '%s' "$out" | tr '\n' ' ' | cut -c1-240)"
    fi
}

# ── 1. The good case: the account signs in and lands inside a living business ─
# Also the proof that the control SENDS what it claims: the fake SaaS refuses the login without
# the exact email + password, the CSRF pair and the Origin; the list without the session; the fake
# hub refuses the callback without this browser's state, and `/api/me` without the hub's session.
start_stub "$(healthy)"
write_env "$tmp/env.ok" "$FAKE_SLUG"
run_preflight "$tmp/env.ok"
if [ "$rc" -eq 0 ]; then
    ok "account with its open hub, /readyz ok and 2 modules inside → GREEN"
else
    bad "account with its open hub, /readyz ok and 2 modules inside → GREEN" \
        "the control went red on the good case (exit $rc): $(printf '%s' "$out" | tr '\n' ' ' | cut -c1-240)"
fi
expect_said "the green names the modules the reviewer will see" "inventory"
if [[ "$out" == *"$FAKE_PASSWORD"* ]]; then
    bad "the good case does not print the password" "the control printed the credential"
else
    ok "the good case does not print the password"
fi

# 🪤 Measured 2026-09-16: with urllib's default User-Agent, Cloudflare answered 403 «error code:
# 1010» and the control died at the login. It sends its own.
ua=$(head -n1 "$tmp/logins" 2>/dev/null)
if [ -z "$ua" ]; then
    bad "the login sends its own User-Agent" "the fake server saw no login request"
elif [[ "$ua" == Python-urllib* ]]; then
    bad "the login sends its own User-Agent" "it sends \`$ua\`: a WAF cuts it and the control never reaches the hub"
else
    ok "the login sends its own User-Agent (a WAF cuts urllib's)"
fi

# ── 2. THE TEST OF THE TEST: the control must CATCH THE POSITIVE ─────────────
# Every case below is a real failure Google would see, and the control must come out RED.

# 2a. A slug that is not the account's — the 09/09 case: a `.env` pointing at a dead hub.
start_stub "$(healthy)"
write_env "$tmp/env.ghost" "$GHOST_SLUG"
expect_fail "🔴 a slug that is not the account's → RED" "$tmp/env.ghost" "hub_not_in_account"
expect_said "…and it names the slug that IS the account's" "$FAKE_SLUG"

# 2b. The business exists but the SaaS will not open it (being prepared, suspended, deleting).
start_stub "$(healthy "\"blocked\": {\"$FAKE_SLUG\": \"preparing\"},")"
write_env "$tmp/env.blocked" "$FAKE_SLUG"
expect_fail "🔴 the SaaS cannot open the business (still preparing) → RED" "$tmp/env.blocked" "hub_not_ready"
expect_said "…and it says why (\`preparing\`)" "preparing"

# 2c. The hub service is down — a 502 from the runtime.
start_stub "{\"hubs\": [\"$FAKE_SLUG\"], \"readyz\": {\"$FAKE_SLUG\": {\"code\": 502, \"body\": {}}}}"
write_env "$tmp/env.down" "$FAKE_SLUG"
expect_fail "🔴 the hub answers 502 → RED" "$tmp/env.down" "hub_not_ready"
expect_said "…and the red says 502 (not approved by the next check by accident)" "502"

# 2d. Nothing answers for that hub at that address.
start_stub "{\"hubs\": [\"$FAKE_SLUG\"], \"readyz\": {}}"
write_env "$tmp/env.404" "$FAKE_SLUG"
expect_fail "🔴 the hub's address answers 404 → RED" "$tmp/env.404" "hub_not_ready"

# 2e. The dead address: nobody listening. The real shape of the 09/09 dead hub.
start_stub "$(healthy)"
write_env "$tmp/env.dead" "$FAKE_SLUG"
expect_fail "🔴 the hub's address does not answer (nobody listening) → RED" \
    "$tmp/env.dead" "hub_unreachable" 'http://127.0.0.1:1/{slug}'

# 2f. 200, but the runtime declares itself unavailable. The HTTP code is not the verdict.
start_stub "{\"hubs\": [\"$FAKE_SLUG\"], \"readyz\": {\"$FAKE_SLUG\": {\"code\": 200, \"body\": {\"status\": \"unavailable\", \"reason\": \"migrations\"}}}}"
write_env "$tmp/env.degraded" "$FAKE_SLUG"
expect_fail "🔴 200 but \`status: unavailable\` → RED (the 200 is not the verdict)" "$tmp/env.degraded" "hub_not_ready"

# 2g. 🔴 THE 09/09 FAILURE: the hub is alive and EMPTY. «Aquí aparecerán tus apps».
start_stub "$(healthy '"apps": [],')"
write_env "$tmp/env.empty" "$FAKE_SLUG"
expect_fail "🔴 hub alive with ZERO modules inside → RED (the shell Google saw on 09/09)" \
    "$tmp/env.empty" "hub_without_modules"

# 2h. The account has no business at all — where the issue started.
start_stub '{"hubs": [], "readyz": {}}'
write_env "$tmp/env.nohubs" "$FAKE_SLUG"
expect_fail "🔴 the account has no hub → RED (the signup screen the reviewer saw)" \
    "$tmp/env.nohubs" "no_hubs"

# 2i. The credentials stopped working: allauth answers 400 `email_password_mismatch`.
start_stub '{"hubs": [], "readyz": {}}'
cat > "$tmp/env.badcreds" <<EOF
PLAY_REVIEWER_EMAIL=$FAKE_EMAIL
PLAY_REVIEWER_PASSWORD=not-the-password-anymore
PLAY_REVIEWER_HUB=$FAKE_SLUG
EOF
expect_fail "🔴 the SaaS rejects the login → RED" "$tmp/env.badcreds" "login_rejected"
expect_said "…and the red says 400" "400"
expect_said "…and names allauth's reason" "email_password_mismatch"

# 2j. The login waits for a step the reviewer cannot take (a second factor): 401 with a flow.
start_stub "$(healthy '"login": {"code": 401, "body": {"status": 401, "data": {"flows": [{"id": "mfa_authenticate", "is_pending": true}]}, "meta": {"is_authenticated": false}}},')"
write_env "$tmp/env.mfa" "$FAKE_SLUG"
expect_fail "🔴 the login asks for a second factor → RED" "$tmp/env.mfa" "login_rejected"
expect_said "…and names the pending step" "mfa_authenticate"

# 2k. 200 to the login, but no authenticated session: nothing to go on with.
start_stub "$(healthy '"login_unauthenticated": true,')"
write_env "$tmp/env.noauth" "$FAKE_SLUG"
expect_fail "🔴 login 200 without \`is_authenticated\` → RED" "$tmp/env.noauth" "login_rejected"

# 2l. The SaaS does not answer at all. Without this case «SaaS down = green» survived (rv-1885).
start_stub "$(healthy)"
write_env "$tmp/env.saasdown" "$FAKE_SLUG"
expect_fail "🔴 the SaaS does not answer (nobody listening) → RED" "$tmp/env.saasdown" "saas_unreachable" "" "http://127.0.0.1:1"

# 2m. The SaaS will not list the account's businesses.
start_stub "$(healthy '"hubs_status": 500,')"
write_env "$tmp/env.nolist" "$FAKE_SLUG"
expect_fail "🔴 the SaaS answers 500 to the list of businesses → RED" "$tmp/env.nolist" "hubs_unreadable"

# 2n. 🔴 NEVER CREATE THE ACCOUNT. saas-next's unified sign-in (ADR-0154) signs UP an email it does
# not know when it is posted to the login. While the SaaS says signup is open, the control stops
# BEFORE sending the credentials: a missing review account must be a red, never a new account.
start_stub "$(healthy '"signup_open": true,')"
write_env "$tmp/env.signup" "$FAKE_SLUG"
expect_fail "🔴 the SaaS has signup open → RED before posting the credentials" "$tmp/env.signup" "signup_open"
if [ -s "$tmp/logins" ]; then
    bad "…and the credentials were never sent" \
        "the control POSTed the login with signup open: an unknown email would have CREATED an account"
else
    ok "…and the credentials were never sent (no login POST reached the SaaS)"
fi

# 2o. The SaaS refuses the jump into the hub (the account is no member of it any more).
start_stub "$(healthy '"hub_login_error": "not_a_member",')"
write_env "$tmp/env.notmember" "$FAKE_SLUG"
expect_fail "🔴 the SaaS refuses the jump into the hub → RED" "$tmp/env.notmember" "hub_session_refused"
expect_said "…and names the SaaS's reason" "not_a_member"

# 2p. The hub refuses the code (it could not redeem it at the SaaS).
start_stub "$(healthy '"callback_error": "invalid_code",')"
write_env "$tmp/env.badcode" "$FAKE_SLUG"
expect_fail "🔴 the hub refuses the one-time code → RED" "$tmp/env.badcode" "hub_session_refused"
expect_said "…and names the hub's reason" "invalid_code"

# 2q. The SaaS's form would post the one-time code somewhere that is not the hub: never sent.
start_stub "$(healthy '"form_action": "http://127.0.0.1:1/elsewhere",')"
write_env "$tmp/env.elsewhere" "$FAKE_SLUG"
expect_fail "🔴 the jump's form points outside the hub → RED, the code is not posted" \
    "$tmp/env.elsewhere" "hub_session_refused"

# 2r. The jump ends but the hub does not know the session: it cannot say what the reviewer sees.
start_stub "$(healthy '"me_anonymous": true,')"
write_env "$tmp/env.anon" "$FAKE_SLUG"
expect_fail "🔴 after the jump the hub still sees nobody → RED" "$tmp/env.anon" "hub_session_refused"

# 2s. The hub answers `/api/me` without the list of apps: nothing to count → RED with its own code.
start_stub "$(healthy '"me_without_apps": true,')"
write_env "$tmp/env.noapps" "$FAKE_SLUG"
expect_fail "🔴 \`/api/me\` without \`apps\` → RED (cannot check what the reviewer sees)" \
    "$tmp/env.noapps" "hub_detail_withheld"

# ── 3. No escape valve: missing configuration is RED, never a silent skip ───
start_stub "$(healthy)"
: > "$tmp/env.empty-file"
expect_fail "🔴 no credentials → RED, not a green skip" "$tmp/env.empty-file" "missing_credentials"
missing_out=$out
for var in PLAY_REVIEWER_EMAIL PLAY_REVIEWER_PASSWORD PLAY_REVIEWER_HUB; do
    if [[ "$missing_out" == *"$var"* ]]; then
        ok "the configuration red names \`$var\`"
    else
        bad "the configuration red names \`$var\`" \
            "without the variable's name nobody knows what to set, and the control gets skipped"
    fi
done

cat > "$tmp/env.half" <<EOF
PLAY_REVIEWER_EMAIL=reviewer@example.invalid
PLAY_REVIEWER_PASSWORD=$FAKE_PASSWORD
EOF
expect_fail "🔴 only PLAY_REVIEWER_HUB missing → RED" "$tmp/env.half" "missing_credentials"

# ── 4. The slug comes from the `.env`, the hosts are the new platform's ─────
# A hardcoded slug or tenant host (`<slug>.erplora.com`, the old `<slug>.a.erplora.com`) would keep
# passing with the `.env` pointing elsewhere. The template `{slug}.erplora.com` does not match:
# no label in front of it.
if grep -qE "$FAKE_SLUG|[a-z0-9]([a-z0-9-]*[a-z0-9])?\.erplora\.com" "$script"; then
    bad "no slug is written in the code" \
        "a slug or a tenant host appears in the control: with the \`.env\` pointing at another hub it would stay green"
else
    ok "no slug is written in the code (it comes from PLAY_REVIEWER_HUB)"
fi

# Since 2026-10-14 the hubs are hub-next at `https://<slug>.erplora.com`; the Hetzner
# `<slug>.a.erplora.com` has no DNS. The default must be the address the reviewer's app opens.
if grep -qF '"https://{slug}.erplora.com"' "$script" && ! grep -qF '.a.erplora.com' "$script"; then
    ok "the default hub address is https://{slug}.erplora.com (hub-next), not the old .a.erplora.com"
else
    bad "the default hub address is https://{slug}.erplora.com" \
        "the control still points at the old Hetzner hosts, which have no DNS since 2026-10-14"
fi

# ── 5. It runs on the runner's Python, not only on the newest one ───────────
# `publish-play` and the `scripts` job run the system python3 of `ubuntu-latest` (3.12). A local
# formatter targeting 3.14 rewrote `except (A, B):` into PEP 758's `except A, B:` (2026-10-15),
# a SyntaxError before 3.14 that would turn every release red without checking anything.
if grep -nE '^[[:space:]]*except[[:space:]]+[^(:][^:]*,[^:]*:' "$script"; then
    bad "every multi-type \`except\` is parenthesized (Python < 3.14 parses it)" \
        "an unparenthesized \`except A, B:\` is 3.14-only: the runner's python3 would not even start the control"
else
    ok "every multi-type \`except\` is parenthesized (Python < 3.14 parses it)"
fi

# ── 6. The `.env` is read the way dotenv reads it ────────────────────────────
# The real root `.env` annotates its lines: `PLAY_REVIEWER_HUB=<slug>   # [var]`. Read raw, the
# comment became part of the value: run locally, the control looked for the hub `<slug>   # [var]`
# and signed in as `<email>   # [var]` (2026-10-15; CI never saw it, the values come exported).
# A `#` glued to the value, or inside quotes, is data: a password may contain one.
start_stub "$(healthy)"
{
    printf '# Google Play review account\n'
    printf 'PLAY_REVIEWER_EMAIL=%s   # [var]\n' "$FAKE_EMAIL"
    printf 'PLAY_REVIEWER_PASSWORD=%s\t# [secret]\n' "$FAKE_PASSWORD"
    printf 'PLAY_REVIEWER_HUB=%s   # [var]\n' "$FAKE_SLUG"
} > "$tmp/env.comments"
run_preflight "$tmp/env.comments"
if [ "$rc" -eq 0 ]; then
    ok "end-of-line comments in the \`.env\` are not part of the values → GREEN"
else
    bad "end-of-line comments in the \`.env\` are not part of the values → GREEN" \
        "the comment travelled with the value (exit $rc): $(printf '%s' "$out" | tr '\n' ' ' | cut -c1-240)"
fi
no_password_in_output "end-of-line comments"

# check_password_kept <case> <password> <how the .env line writes it>: the fake SaaS only lets in
# this exact password, so a reader that cut it at the `#` comes out red.
check_password_kept() {
    local name=$1 saved=$FAKE_PASSWORD
    FAKE_PASSWORD=$2
    start_stub "$(healthy)"
    {
        printf 'PLAY_REVIEWER_EMAIL=%s\n' "$FAKE_EMAIL"
        printf 'PLAY_REVIEWER_PASSWORD=%s\n' "$3"
        printf 'PLAY_REVIEWER_HUB=%s\n' "$FAKE_SLUG"
    } > "$tmp/env.password"
    run_preflight "$tmp/env.password"
    if [ "$rc" -eq 0 ]; then
        ok "$name"
    else
        bad "$name" "the password reached the SaaS cut or changed (exit $rc): $(printf '%s' "$out" | tr '\n' ' ' | cut -c1-240)"
    fi
    no_password_in_output "$name"
    FAKE_PASSWORD=$saved
}
check_password_kept "a \`#\` glued to the value is part of it (p#ss) → GREEN" \
    's3cr3t#not-a-real-one-8f2c' 's3cr3t#not-a-real-one-8f2c   # [secret]'
check_password_kept "a \` # \` inside double quotes is part of the value → GREEN" \
    's3cr3t # not-a-real-one-8f2c' '"s3cr3t # not-a-real-one-8f2c"   # [secret]'
check_password_kept "a \` # \` inside single quotes is part of the value → GREEN" \
    's3cr3t # not-a-real-one-8f2c' "'s3cr3t # not-a-real-one-8f2c'"

# An empty value that only carries its note is still missing: read raw it became the slug `# set me`
# and the red blamed the account (`hub_not_in_account`) instead of naming the variable to set.
start_stub "$(healthy)"
{
    printf 'PLAY_REVIEWER_EMAIL=%s\n' "$FAKE_EMAIL"
    printf 'PLAY_REVIEWER_PASSWORD=%s\n' "$FAKE_PASSWORD"
    printf 'PLAY_REVIEWER_HUB=   # set me\n'
} > "$tmp/env.empty-commented"
expect_fail "🔴 an empty PLAY_REVIEWER_HUB with only a comment → missing, RED" \
    "$tmp/env.empty-commented" "missing_credentials"

printf '\n'
if [ "$fail" -gt 0 ]; then
    printf 'FAIL: %d case(s) (%d ok)\n' "$fail" "$pass"
    exit 1
fi
printf 'OK: %d case(s)\n' "$pass"
