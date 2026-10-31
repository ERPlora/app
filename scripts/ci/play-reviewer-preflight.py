#!/usr/bin/env python3
"""Does the Google review account really get inside a living business? — ERPlora/hub#1718.

The step a person runs BEFORE sending the app to Google (`GOOGLE-PLAY.md`, step 0), and the
`publish-play` job before every upload. The instructions handed to the reviewer promise that
signing in with the test account shows the app working; on 2026-09-09 that was measured and it was
false — the account landed on the empty «Crea tu negocio gratis» screen, and once a hub existed, on
a hub with zero modules. A reviewer who cannot see the app rejects it, and a rejection costs another
whole round.

It walks the reviewer's own path on the platform live since 2026-10-14 — saas-next at `erplora.com`,
hub-next at `https://<slug>.erplora.com`:

  1. sign-in     allauth.headless, browser client: `GET /_allauth/browser/v1/config` (CSRF cookie)
                 and `POST /_allauth/browser/v1/auth/login` → `meta.is_authenticated` + session;
  2. businesses  `GET /api/hubs/businesses` with that session (`hubs.business.list`): the hub named
                 by PLAY_REVIEWER_HUB is the account's and the SaaS can open it (`can_open`);
  3. hub up      `GET <hub>/readyz` answers `{"status": "ok"}`;
  4. the jump    the launcher's «Open»: `<hub>/auth/login` → the SaaS's `/hub-login/` (one-time code
                 in a form) → `POST <hub>/auth/callback`, which opens the hub's session;
  5. modules     `GET <hub>/api/me` with that session lists the apps the account sees.

Exits 0 only when ALL of it holds, and 1 naming the first thing that does not:

  missing_credentials  the `.env` does not carry the reviewer account   → never a silent skip
  saas_unreachable     the SaaS did not answer
  code_required        the account signs in, but the SaaS asks it for the code it mails
                       (`login_by_code`, `verify_email`): Google's reviewer cannot read that
                       mailbox → mark it as a review account in the SaaS (ERPlora/saas#132)
  login_rejected       the account no longer signs in
  hubs_unreadable      the SaaS would not list the account's businesses
  no_hubs              the account has no business at all               → the 09/09 signup screen
  hub_not_in_account   PLAY_REVIEWER_HUB names a hub that is not one of the account's
  hub_unreachable      that hub's address does not answer
  hub_not_ready        the SaaS cannot open it, or it answers but is not ready
  hub_session_refused  the jump into the hub did not open a session for the account
  hub_detail_withheld  the hub opened one but would not say which apps the account sees
  hub_without_modules  it is up and EMPTY                               → the 09/09 shell

What it writes on the platform: the SaaS session of the login, and, on success, one single-use code
and the hub's session — exactly what the reviewer's own «Open» writes (the hub keeps its copy of the
user on first entry). It never creates an account nor a hub: it posts only to the sign-in, which in
allauth.headless never signs anyone up — the sign-up is its own door, `POST …/auth/signup`, that this
control never calls (saas-next `accounts/doors.py`), whatever `is_open_for_signup` says (app#7).

The slug is never written here: it comes from `PLAY_REVIEWER_HUB`. A hardcoded one would keep
passing after the `.env` is pointed elsewhere (2026-09-09). The password is read and sent, never
printed — not even on the failure paths.

Usage:  python3 scripts/ci/play-reviewer-preflight.py
Contract test: `scripts/tests/play-reviewer-preflight.test.sh` (hermetic, fake SaaS and fake hub).
Runs on Python 3.9+, the python3 a Mac ships (app#12), not only on the runner's 3.12.
"""

from __future__ import annotations

import http.cookiejar
import json
import os
import re
import sys
import urllib.error
import urllib.request
from html.parser import HTMLParser
from urllib.parse import parse_qs, urlencode, urljoin, urlsplit

# 🪤 Measured 2026-09-16: with urllib's default User-Agent, Cloudflare answered 403 «error code:
# 1010» and the control died at the login without looking at anything.
USER_AGENT = "ERPlora-play-reviewer-preflight/1.0 (+https://erplora.com)"

SAAS_URL = os.environ.get("PLAY_REVIEWER_SAAS_URL", "https://erplora.com").rstrip("/")
# `{slug}` is filled from PLAY_REVIEWER_HUB: hub-next serves every hub at its own subdomain.
HUB_URL = os.environ.get("PLAY_REVIEWER_HUB_URL", "https://{slug}.erplora.com")

TIMEOUT = 30

REQUIRED = ("PLAY_REVIEWER_EMAIL", "PLAY_REVIEWER_PASSWORD", "PLAY_REVIEWER_HUB")

# saas-next's addresses (config/urls.py, hubs/api.py, accounts/handoff.py).
CONFIG_PATH = "/_allauth/browser/v1/config"
LOGIN_PATH = "/_allauth/browser/v1/auth/login"
BUSINESSES_PATH = "/api/hubs/businesses"
HUB_LOGIN_PATH = "/hub-login/"
# hub-next's addresses (config/urls.py, base/auth_views.py, base/api.py).
READYZ_PATH = "/readyz"
JUMP_PATH = "/auth/login"
ME_PATH = "/api/me"

# The sign-in steps that wait for the code saas-next mails (`accounts/tests/test_sign_in_code.py`):
# every password sign-in (`login_by_code`), and an address not yet proven (`verify_email`).
MAILED_CODE_FLOWS = ("login_by_code", "verify_email")


def fail(code: str, message: str) -> None:
    """A red that names what to do. A red that does not is one somebody 'fixes' by deleting it."""
    print(f"FAIL {code}: {message}", file=sys.stderr)
    sys.exit(1)


def default_env_file() -> str:
    """The monorepo's root `.env` — the parent of this repo, where `PLAY_REVIEWER_*` lives."""
    repo_root = os.path.dirname(
        os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    )
    return os.path.join(os.path.dirname(repo_root), ".env")


def read_env_file(path: str) -> dict:
    """`KEY=value` pairs, quotes and end-of-line comments stripped. A missing file is not an error
    here: the caller decides, because the variables may perfectly well come from the environment
    instead."""
    values = {}
    try:
        with open(path, encoding="utf-8", errors="replace") as fh:
            for line in fh:
                match = re.match(r"^\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=(.*)$", line)
                if not match:
                    continue
                value = match.group(2).strip()
                quoted = re.fullmatch(r"([\"'])(.*?)\1(?:\s+#.*)?", value)
                if quoted:
                    value = quoted.group(2)
                else:
                    # The root `.env` annotates lines (`KEY=value   # [var]`): a `#` after
                    # whitespace opens a comment, while one glued to the value (`p#ss`) is data.
                    value = re.split(r"\s#", match.group(2), maxsplit=1)[0].strip()
                values[match.group(1)] = value
    except OSError:
        return {}
    return values


def credentials() -> tuple[str, str, str]:
    """The environment wins over the file, so a caller can point this at another account without
    editing the `.env`. Missing configuration is LOUD: naming the variables is what stops the
    next person from working around the control instead of setting it up."""
    env_file = os.environ.get("PLAY_REVIEWER_ENV_FILE") or default_env_file()
    from_file = read_env_file(env_file)
    values = {
        key: (os.environ.get(key) or from_file.get(key) or "").strip()
        for key in REQUIRED
    }
    missing = [key for key in REQUIRED if not values[key]]
    if missing:
        fail(
            "missing_credentials",
            f"{', '.join(missing)} missing. Set them in {env_file} (or export them) with the "
            "account Google reviews the app with and the slug of its hub. Without them this CANNOT "
            "be checked, and sending the app to Google blind is what cost the 09/09 round.",
        )
    return (
        values["PLAY_REVIEWER_EMAIL"],
        values["PLAY_REVIEWER_PASSWORD"],
        values["PLAY_REVIEWER_HUB"],
    )


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    """Redirects are answers to read (where the jump goes, which error it names), never followed."""

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


class Answer:
    def __init__(self, status: int, body: str, headers=None) -> None:
        self.status = status
        self.body = body
        self.headers = headers

    def header(self, name: str) -> str:
        return (self.headers.get(name) if self.headers is not None else None) or ""

    def json(self) -> object:
        try:
            return json.loads(self.body)
        except (ValueError, TypeError):
            return None


class Browser:
    """One site's cookies, as a browser keeps them. The SaaS and the hub get one each, so a cookie
    of `erplora.com` never reaches `<slug>.erplora.com` (both call their session `sessionid`)."""

    def __init__(self) -> None:
        self.jar = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(self.jar), _NoRedirect()
        )

    def cookie(self, name: str) -> str | None:
        return next((c.value for c in self.jar if c.name == name), None)

    def send(
        self,
        url: str,
        *,
        json_body: dict | None = None,
        form: dict | None = None,
        headers: dict | None = None,
        accept: str = "application/json",
    ) -> Answer:
        """An HTTP error or a redirect is an answer, not a crash: the verdict reads its code.
        `status` 0 means nothing answered, and `body` says why."""
        sent = {"User-Agent": USER_AGENT, "Accept": accept, **(headers or {})}
        data = None
        if json_body is not None:
            data = json.dumps(json_body).encode()
            sent["Content-Type"] = "application/json"
        elif form is not None:
            data = urlencode(form).encode()
            sent["Content-Type"] = "application/x-www-form-urlencoded"
        req = urllib.request.Request(url, data=data, headers=sent)
        try:
            with self.opener.open(req, timeout=TIMEOUT) as response:
                body = response.read().decode("utf-8", "replace")
                return Answer(response.status, body, response.headers)
        except urllib.error.HTTPError as exc:
            with exc:
                return Answer(
                    exc.code, exc.read().decode("utf-8", "replace"), exc.headers
                )
        except (urllib.error.URLError, OSError) as exc:
            reason = exc.reason if isinstance(exc, urllib.error.URLError) else exc
            return Answer(0, str(reason))


class _HubLoginForm(HTMLParser):
    """The SaaS's auto-submitted form of the jump (`accounts/hub_login_form.html`)."""

    def __init__(self) -> None:
        super().__init__()
        self.action: str | None = None
        self.fields: dict[str, str] = {}
        self._inside = False

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag == "form" and attrs.get("id") == "hub-login-form":
            self.action, self._inside = attrs.get("action") or "", True
        elif tag == "input" and self._inside and attrs.get("name"):
            self.fields[attrs["name"]] = attrs.get("value") or ""

    def handle_endtag(self, tag):
        if tag == "form":
            self._inside = False


def query_value(url: str, name: str) -> str | None:
    return parse_qs(urlsplit(url).query).get(name, [None])[0]


def _identifier(value: object) -> bool:
    return isinstance(value, str) and re.fullmatch(r"[a-z0-9_]{1,64}", value) is not None


def pending_flows(payload: object) -> list[str]:
    """The ids of the steps allauth.headless says the sign-in still waits for (`data.flows[]` with
    `is_pending`), identifiers only."""
    data = payload.get("data") if isinstance(payload, dict) else None
    flows = data.get("flows") if isinstance(data, dict) else None
    if not isinstance(flows, list):
        return []
    return [
        f.get("id")
        for f in flows
        if isinstance(f, dict) and f.get("is_pending") and _identifier(f.get("id"))
    ]


def allauth_reasons(payload: object) -> str:
    """allauth's error codes and pending steps (`email_password_mismatch`, `mfa_authenticate`…).
    Only identifiers: its messages are never echoed, whatever they may carry."""
    if not isinstance(payload, dict):
        return ""
    errors = payload.get("errors")
    codes = (
        [e.get("code") for e in errors if isinstance(e, dict)]
        if isinstance(errors, list)
        else []
    )
    codes = [c for c in codes if _identifier(c)] + pending_flows(payload)
    return f" ({', '.join(codes)})" if codes else ""


def sign_in(saas: Browser, email: str, password: str) -> None:
    # ── 1. The account signs in ──────────────────────────────────────────────
    config = saas.send(f"{SAAS_URL}{CONFIG_PATH}")
    if config.status == 0:
        fail("saas_unreachable", f"{SAAS_URL} does not answer: {config.body}")
    payload = config.json()
    account = (
        payload.get("data", {}).get("account") if isinstance(payload, dict) else None
    )
    if config.status != 200 or not isinstance(account, dict):
        fail(
            "saas_unreachable",
            f"{SAAS_URL}{CONFIG_PATH} answers HTTP {config.status} without allauth's configuration: "
            "the SaaS's sign-in is not there.",
        )
    # `is_open_for_signup` is not read: the sign-in below never signs anyone up (the sign-up is
    # `auth/signup`, never called here), so an open sign-up is no reason to leave the account
    # unproven (app#7).
    csrf = saas.cookie("csrftoken")
    if not csrf:
        fail(
            "login_rejected",
            f"{SAAS_URL}{CONFIG_PATH} set no `csrftoken` cookie: without it the login is a 403",
        )

    answer = saas.send(
        f"{SAAS_URL}{LOGIN_PATH}",
        json_body={"email": email, "password": password},
        headers={"X-CSRFToken": csrf, "Origin": SAAS_URL},
    )
    if answer.status == 0:
        fail(
            "saas_unreachable",
            f"{SAAS_URL} stopped answering at the login: {answer.body}",
        )
    payload = answer.json()
    # The password is accepted and the SaaS mails a code to finish: the reviewer stops right there.
    mailed_code = [f for f in pending_flows(payload) if f in MAILED_CODE_FLOWS]
    if answer.status == 401 and mailed_code:
        fail(
            "code_required",
            f"the review account exists and its password is accepted, but {SAAS_URL} asks it for "
            f"the code it mails ({', '.join(mailed_code)}). Google's reviewer cannot read that "
            "mailbox, so the review would stop at the sign-in. The account has to be exempt from "
            "the mailed code as a review account in the SaaS (ERPlora/saas#132).",
        )
    meta = payload.get("meta") if isinstance(payload, dict) else None
    if (
        answer.status != 200
        or not isinstance(meta, dict)
        or meta.get("is_authenticated") is not True
    ):
        fail(
            "login_rejected",
            f"the review account does not sign in at {SAAS_URL} (HTTP {answer.status}"
            f"{', not authenticated' if answer.status == 200 else ''}){allauth_reasons(payload)}. "
            "Google uses exactly these credentials: if they do not get in, the reviewer does not either.",
        )


def find_business(saas: Browser, wanted_slug: str) -> dict:
    # ── 2. …and has a business, the one the `.env` names, which the SaaS can open ─
    answer = saas.send(f"{SAAS_URL}{BUSINESSES_PATH}?{urlencode({'limit': 100})}")
    if answer.status == 0:
        fail(
            "saas_unreachable",
            f"{SAAS_URL} stopped answering at the businesses: {answer.body}",
        )
    if answer.status != 200:
        fail(
            "hubs_unreadable",
            f"the SaaS does not list the account's businesses (HTTP {answer.status})",
        )
    payload = answer.json()
    rows = payload.get("rows") if isinstance(payload, dict) else None
    if not isinstance(rows, list):
        fail("hubs_unreadable", "the SaaS's answer carries no `rows` list")
    rows = [r for r in rows if isinstance(r, dict) and r.get("slug")]
    if not rows:
        fail(
            "no_hubs",
            "the review account has NO business: whoever signs in sees the sign-up of a business, "
            "which is exactly what the reviewer saw on 09/09.",
        )
    row = next((r for r in rows if r["slug"] == wanted_slug), None)
    if row is None:
        fail(
            "hub_not_in_account",
            f"PLAY_REVIEWER_HUB={wanted_slug} is not a business of the review account. Its own are: "
            f"{', '.join(sorted(r['slug'] for r in rows))}. Set the variable to the right one "
            "(sending the reviewer to an address that is not theirs is a guaranteed 404).",
        )
    if row.get("can_open") is not True:
        fail(
            "hub_not_ready",
            f"the SaaS cannot open {wanted_slug}: status `{row.get('status')}`, reason "
            f"`{row.get('open_blocked_reason')}`. The reviewer's «Open» would not work.",
        )
    return row


def check_hub_up(hub: Browser, hub_url: str) -> None:
    # ── 3. …that is actually up ──────────────────────────────────────────────
    # hub-next's /readyz is the service's (database + migrations), not one hub's: which hub exists
    # and may open is the SaaS's answer above, and what is inside is step 5.
    answer = hub.send(f"{hub_url}{READYZ_PATH}")
    if answer.status == 0:
        fail(
            "hub_unreachable", f"{hub_url}{READYZ_PATH} does not answer: {answer.body}"
        )
    if answer.status != 200:
        fail(
            "hub_not_ready",
            f"{hub_url}{READYZ_PATH} answers HTTP {answer.status}: the reviewer would not see the app",
        )
    ready = answer.json()
    if not isinstance(ready, dict) or ready.get("status") != "ok":
        status = ready.get("status") if isinstance(ready, dict) else None
        fail(
            "hub_not_ready",
            f"{hub_url}{READYZ_PATH} answers 200 but declares `{status}`. The 200 is not the "
            "verdict: the `status` inside is.",
        )


def enter_hub(saas: Browser, hub: Browser, hub_url: str, slug: str) -> None:
    # ── 4. …and lets the account in, through the reviewer's own «Open» ───────
    # The hub has no login of its own: it sends to the SaaS with a `state` in its cookie, the SaaS
    # answers a form with a single-use code, and the hub's callback opens the session.
    start = hub.send(f"{hub_url}{JUMP_PATH}?{urlencode({'next': '/'})}")
    if start.status == 0:
        fail("hub_unreachable", f"{hub_url}{JUMP_PATH} does not answer: {start.body}")
    target = urljoin(f"{hub_url}{JUMP_PATH}", start.header("Location"))
    if start.status not in (301, 302, 303, 307, 308) or not target.startswith(
        f"{SAAS_URL}{HUB_LOGIN_PATH}"
    ):
        fail(
            "hub_session_refused",
            f"{hub_url}{JUMP_PATH} does not send to {SAAS_URL}{HUB_LOGIN_PATH} (HTTP {start.status}): "
            "the hub would not start the reviewer's sign-in.",
        )

    page = saas.send(target, accept="text/html")
    if page.status == 0:
        fail(
            "saas_unreachable", f"{SAAS_URL} stopped answering at the jump: {page.body}"
        )
    if page.status != 200:
        refused = urljoin(target, page.header("Location"))
        reason = (
            query_value(refused, "code")
            or urlsplit(refused).path
            or f"HTTP {page.status}"
        )
        fail(
            "hub_session_refused",
            f"the SaaS refuses the review account's jump into {slug} (`{reason}`): the reviewer's "
            "«Open» would end there.",
        )
    form = _HubLoginForm()
    form.feed(page.body)
    action = urljoin(target, form.action or "")
    if (
        not form.action
        or not form.fields.get("code")
        or not action.startswith(f"{hub_url}/")
    ):
        fail(
            "hub_session_refused",
            f"the SaaS's jump page carries no form with a code for {hub_url}: nothing was posted "
            "(a single-use code only goes to the hub it is for).",
        )

    callback = hub.send(action, form=form.fields, accept="text/html")
    if callback.status == 0:
        fail("hub_unreachable", f"{action} does not answer: {callback.body}")
    landed = urljoin(action, callback.header("Location"))
    if callback.status not in (301, 302, 303, 307, 308) or not landed.startswith(
        f"{hub_url}/"
    ):
        reason = query_value(landed, "error") if callback.header("Location") else None
        fail(
            "hub_session_refused",
            f"the hub {slug} does not open a session for the review account (HTTP {callback.status}"
            f"{f', `{reason}`' if reason else ''}): if it does not let this account in, it does not "
            "let the reviewer in either.",
        )


def count_modules(hub: Browser, hub_url: str, slug: str) -> list[str]:
    # ── 5. …and is not an empty shell. This is the 09/09 failure ─────────────
    answer = hub.send(f"{hub_url}{ME_PATH}")
    if answer.status == 0:
        fail("hub_unreachable", f"{hub_url}{ME_PATH} does not answer: {answer.body}")
    me = answer.json()
    if answer.status != 200 or not isinstance(me, dict):
        fail(
            "hub_not_ready",
            f"{hub_url}{ME_PATH} answers HTTP {answer.status} without a JSON document",
        )
    if not me.get("user"):
        fail(
            "hub_session_refused",
            f"the jump into {slug} ended, but {hub_url}{ME_PATH} still sees nobody: the session "
            "did not stick, and the reviewer would be sent back to the sign-in.",
        )
    apps = me.get("apps")
    if not isinstance(apps, list):
        fail(
            "hub_detail_withheld",
            f"{hub_url}{ME_PATH} does not say which apps the review account sees (no `apps` list): "
            "what the reviewer will find cannot be checked.",
        )
    labels = [a.get("label") for a in apps if isinstance(a, dict) and a.get("label")]
    if not labels:
        fail(
            "hub_without_modules",
            f"the hub {slug} is alive but the review account sees NO module in it: the reviewer "
            "gets in and finds «Aquí aparecerán tus apps», no POS, no calendar, no till. Apply a "
            "sector template before sending.",
        )
    return labels


def main() -> None:
    email, password, wanted_slug = credentials()
    saas, hub = Browser(), Browser()
    sign_in(saas, email, password)
    row = find_business(saas, wanted_slug)
    hub_url = HUB_URL.replace("{slug}", wanted_slug).rstrip("/")
    check_hub_up(hub, hub_url)
    enter_hub(saas, hub, hub_url, wanted_slug)
    labels = count_modules(hub, hub_url, wanted_slug)
    print(
        f"OK: the review account signs in, its business `{wanted_slug}` is `{row.get('status')}` and "
        f"opens at {hub_url}, and inside it sees {len(labels)} modules: {', '.join(labels)}."
    )


if __name__ == "__main__":
    main()
