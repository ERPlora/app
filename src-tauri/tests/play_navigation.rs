//! Which pages of erplora.com the Google Play copy is willing to show INSIDE its own window
//! (hub#1915).
//!
//! The app boots on the SaaS (`/shell/`) and shows its sign-in, sign-up and account pages in the
//! webview. Until this guard `on_navigation` returned `true` without looking, so the window could
//! walk to ANY page of erplora.com — the home page with the plan prices, the public marketplace,
//! the panel with Billing. One round of the Android QA found six such doors (hub#1900, hub#1910,
//! saas#2083, saas#2084, saas#2085 and the «Log Out» + cookie banner of saas#2088), each one closed
//! by hand in its template; the next link somebody adds to the web opens the seventh. Paying for
//! digital goods outside Google Play billing, or being steered there, is a rejection reason.
//!
//! So the APP decides: a short list of SaaS pages it follows, and everything else of the SaaS is
//! refused in the Play copy — whatever the templates say. The other copies (direct install,
//! Microsoft Store) keep following every link, exactly as before.
//!
//! What is deliberately NOT policed: pages that are not the SaaS. Hubs are where the app lives, the
//! loopback is development, the bundled page is ours, and foreign hosts keep today's behaviour —
//! the payment pages this guard exists for are reached through the SaaS, and the SaaS is what the
//! list covers. And it governs the WINDOW only: the doors that leave for the system browser
//! (hub#1900, hub#1910 go through `openExternal`) are the web's guard and hub#1918.

use erplora_tauri_lib::{NavigationVerdict, navigation_verdict};

const SAAS: &str = "https://erplora.com";
const PLAY: &str = "play";

fn verdict_in(distribution: &str, raw: &str) -> NavigationVerdict {
    let url: tauri::Url = raw
        .parse()
        .unwrap_or_else(|e| panic!("{raw} is not a URL: {e}"));
    navigation_verdict(distribution, SAAS, &url)
}

fn in_play(raw: &str) -> NavigationVerdict {
    verdict_in(PLAY, raw)
}

fn assert_all(expected: NavigationVerdict, addresses: &[&str], why: &str) {
    let wrong: Vec<String> = addresses
        .iter()
        .filter_map(|raw| {
            let got = in_play(raw);
            (got != expected).then(|| format!("{raw} → {got:?}"))
        })
        .collect();
    assert!(
        wrong.is_empty(),
        "expected {expected:?} in the Play copy ({why}), but:\n  {}",
        wrong.join("\n  ")
    );
}

// ── The positive control: what the guard exists to stop ──────────────────────────────────────────

#[test]
fn the_play_copy_refuses_the_saas_pages_that_sell() {
    // The five the issue names, plus the rest of the public site and the panel. If `on_navigation`
    // goes back to answering `true` without looking, this is the test that goes red.
    assert_all(
        NavigationVerdict::Refuse,
        &[
            "https://erplora.com/pricing/",
            "https://erplora.com/pricing/pro/",
            "https://erplora.com/marketplace/",
            "https://erplora.com/marketplace/modules/pos/",
            "https://erplora.com/dashboard/",
            "https://erplora.com/dashboard/billing/",
            "https://erplora.com/dashboard/hubs/",
            "https://erplora.com/dashboard/marketplace/modules/pos/?hub=h1&utm_source=hub",
            // The panel's Store and Billing where the SaaS serves them since saas#123, and their old
            // addresses at the root, which answer 301 there.
            "https://erplora.com/dashboard/store",
            "https://erplora.com/dashboard/store/plans?business=7f8e1c2a-4b1d-4c3e-9f00-1a2b3c4d5e6f",
            "https://erplora.com/dashboard/store/modules",
            "https://erplora.com/dashboard/billing",
            "https://erplora.com/dashboard/billing/invoices",
            "https://erplora.com/dashboard/billing/subscriptions",
            "https://erplora.com/dashboard/billing/payment",
            "https://erplora.com/store/plans",
            "https://erplora.com/billing/invoices",
            "https://erplora.com/modules/",
            "https://erplora.com/solutions/",
            "https://erplora.com/features/",
            "https://erplora.com/app/",
            "https://erplora.com/home/",
        ],
        "a page of the SaaS that sells, or one tap from it",
    );
}

#[test]
fn the_play_copy_sends_the_saas_home_page_to_the_app_s_own_start() {
    // The web's home page is the one address the SaaS itself sends people to without asking: the
    // «Log Out» POST lands there (`ACCOUNT_LOGOUT_REDIRECT_URL`), and so do the logo and the «←» of
    // the sign-in pages. Refusing it would leave a signed-out person on a stale page; inside the
    // app, "home" is the app's start — which asks for credentials again when there is no session.
    assert_all(
        NavigationVerdict::Home,
        &[
            "https://erplora.com/",
            "https://erplora.com",
            "https://erplora.com/?utm_source=app",
            "https://erplora.com/#pricing",
            "https://www.erplora.com/",
        ],
        "the SaaS's home page",
    );
}

// ── …and what the app still needs to work ───────────────────────────────────────────────────────

#[test]
fn the_play_copy_follows_sign_in_sign_up_and_the_way_into_a_hub() {
    // The SaaS's entry pages as it serves them today (a single-page app since saas-next): the
    // launcher is the sign-in and sign-up, and the steps around it live at the root. None of them
    // shows a price: the launcher's «Create business» card starts on Free without a card, and
    // paying an unpaid invoice is offered on the web only.
    assert_all(
        NavigationVerdict::Allow,
        &[
            // The app's own start: the launcher under the address the app boots on, and «Open».
            "https://erplora.com/shell/",
            "https://erplora.com/shell",
            "https://erplora.com/shell/?choose=1",
            "https://erplora.com/shell/open/7f8e1c2a-4b1d-4c3e-9f00-1a2b3c4d5e6f/",
            // The launcher: where the SaaS signs in (`SPA_LOGIN`) and signs up (`?tab=signup`); its
            // old address with the slash only leads there.
            "https://erplora.com/launch",
            "https://erplora.com/launch?next=/shell/",
            "https://erplora.com/launch?tab=signup&next=/shell/",
            "https://erplora.com/launch/",
            // The old sign-in, which leads to the launcher, and where a refused sign-in or jump
            // lands.
            "https://erplora.com/login",
            "https://erplora.com/login/error?code=authentication_error",
            // Verify the e-mail, reset the password, the second factor and its settings, «Confirm
            // it's you».
            "https://erplora.com/verify-email",
            "https://erplora.com/verify-email?next=/shell/",
            "https://erplora.com/password/reset",
            "https://erplora.com/password/reset/key/1-abc123",
            "https://erplora.com/2fa/authenticate",
            "https://erplora.com/2fa",
            "https://erplora.com/reauthenticate?method=totp&flows=mfa_reauthenticate&next=/shell/",
            // allauth's old addresses of the sign-in and the sign-up, which lead to the launcher.
            "https://erplora.com/accounts/login/?next=/shell/",
            "https://erplora.com/accounts/signup/",
            // «Continue with Google» (a form posted to allauth, which sends the window to Google)
            // and Google's way back.
            "https://erplora.com/_allauth/browser/v1/auth/provider/redirect",
            "https://erplora.com/auth/google/callback/?code=x&state=y",
            // Joining a business: every invitation waiting, or one first.
            "https://erplora.com/invitation",
            "https://erplora.com/invitation/",
            "https://erplora.com/invitations/7f8e1c2a-4b1d-4c3e-9f00-1a2b3c4d5e6f",
            // Google Play requires the privacy policy to be reachable from inside the app, and the
            // cookie banner of these pages links to its settings. Neither sells — and every link
            // they carry to a page that does is judged by this same list.
            "https://erplora.com/legal/privacy/",
            "https://erplora.com/legal/terms/",
            "https://erplora.com/cookies/",
        ],
        "sign-in, sign-up and the way into a hub",
    );
}

#[test]
fn the_entry_pages_the_saas_no_longer_serves_are_not_followed() {
    // The old product's entry pages that saas-next does not serve: with no route they fall into the
    // single-page app's catch-all, or into Django's 404 under the prefixes the app does not own
    // (`/accounts/`, `/auth/`). A page that is not there needs no door — and the next page the SaaS
    // puts at one of those addresses is judged on its own when it exists.
    assert_all(
        NavigationVerdict::Refuse,
        &[
            "https://erplora.com/register/",
            "https://erplora.com/post-login/",
            "https://erplora.com/start/",
            "https://erplora.com/enter/7f8e1c2a-4b1d-4c3e-9f00-1a2b3c4d5e6f/",
            "https://erplora.com/hub/starting/panaderia/",
            "https://erplora.com/accounts/logout/",
            "https://erplora.com/accounts/confirm-email/MQ:1vX:abc/",
            "https://erplora.com/accounts/password/reset/",
            "https://erplora.com/accounts/2fa/authenticate/",
            "https://erplora.com/accounts/reauthenticate/",
            "https://erplora.com/auth/google/login/?next=/shell/",
            "https://erplora.com/auth/hub-bridge/?hub=panaderia",
            "https://erplora.com/auth/handoff/one-time-code/",
            // allauth's other endpoints are its API, never a page of the window.
            "https://erplora.com/_allauth/browser/v1/config",
        ],
        "an entry page the SaaS no longer serves",
    );
}

#[test]
fn the_businesses_are_followed_where_the_saas_serves_them_today() {
    // saas#123 moved the panel under `/dashboard/`: Businesses — pick, open or create one, nothing
    // sold — lives at `/dashboard/businesses`, and its old address at the root answers 301 there,
    // so the old one is followed too (bookmarks, the Hub's links, the mails already sent) and its
    // redirect is not refused halfway.
    assert_all(
        NavigationVerdict::Allow,
        &[
            "https://erplora.com/dashboard/businesses",
            "https://erplora.com/dashboard/businesses/",
            "https://erplora.com/dashboard/businesses?create=1",
            "https://erplora.com/dashboard/businesses/7f8e1c2a-4b1d-4c3e-9f00-1a2b3c4d5e6f/summary",
            "https://erplora.com/dashboard/businesses/7f8e1c2a-4b1d-4c3e-9f00-1a2b3c4d5e6f/members",
            "https://erplora.com/businesses/7f8e1c2a-4b1d-4c3e-9f00-1a2b3c4d5e6f/summary",
        ],
        "Businesses, at its address under the panel and at its old one",
    );
    assert_all(
        NavigationVerdict::Refuse,
        &[
            // On a segment boundary, as every prefix of the list.
            "https://erplora.com/dashboard/businesses-pricing/",
            // Only Businesses: not the panel it is a page of.
            "https://erplora.com/dashboard/",
        ],
        "a page of the panel that is not Businesses",
    );
}

#[test]
fn open_walks_from_businesses_into_the_hub() {
    // «Open» on Businesses (`hubs.business.open`) sends the window to the hub's start of the jump,
    // `/auth/login?next=` on the hub, which binds the jump to this browser with a `state` and
    // bounces to the SaaS's `/hub-login/`. That door mints the one-time code and answers a form that
    // POSTs it to the hub's `/auth/callback`; the hub opens its first page. Every link of the chain,
    // in the order the window walks it: one refused link and «Open» stays on the jump.
    assert_all(
        NavigationVerdict::Allow,
        &[
            "https://erplora.com/dashboard/businesses/",
            "https://erplora.com/shell/open/7f8e1c2a-4b1d-4c3e-9f00-1a2b3c4d5e6f/",
            "https://panaderia.a.erplora.com/auth/login?next=/&shell=1",
            // The door: a member's jump, the Hub sending it back after a refused code, and `pre`.
            "https://erplora.com/hub-login/?hub=panaderia&next=/&state=x",
            "https://erplora.com/hub-login/?hub=panaderia&next=/&error=invalid_code&state=x",
            "https://pre.erplora.com/hub-login/?hub=panaderia&next=/&state=x",
            "https://panaderia.a.erplora.com/auth/callback",
            "https://panaderia.a.erplora.com/",
        ],
        "the jump from «Open» into a hub",
    );
    // Where the door sends a jump it does not finish: the launcher without a session (it comes back
    // to the door once signed in), «Confirm it's you» for the second factor, and the page that says
    // why a jump stopped. None of them sells.
    assert_all(
        NavigationVerdict::Allow,
        &[
            "https://erplora.com/launch?next=/hub-login/%3Fhub%3Dpanaderia%26next%3D/%26state%3Dx",
            "https://erplora.com/reauthenticate?method=totp&flows=mfa_reauthenticate&next=/hub-login/%3Fhub%3Dpanaderia%26state%3Dx",
            "https://erplora.com/login/error?code=invalid_hub",
            "https://erplora.com/login/error?code=not_a_member&hub=panaderia",
            "https://erplora.com/login/error?code=too_many",
        ],
        "where the door sends a jump it does not finish",
    );
    // The staff's «Open in the Hub admin» ends in the SaaS admin when it stops: never a page of the
    // Play copy, which has no way into the admin.
    assert_all(
        NavigationVerdict::Refuse,
        &[
            "https://erplora.com/admin/accounts/hubmembership/",
            "https://erplora.com/admin/accounts/hubs/",
        ],
        "the SaaS admin, where the staff's jump stops",
    );
}

#[test]
fn the_account_page_opens_only_as_the_account_surface() {
    // hub#1900 / saas#2084: `?surface=account` paints the person's account WITHOUT the panel that
    // sells. Play requires an in-app way to delete the account, and it lives there. Since saas#123
    // Account lives at `/dashboard/account/…`; its old addresses — `/dashboard/profile/…` of the old
    // panel and `/account/…`, where it lived before the move — answer 301 there keeping the query,
    // so each is followed WITH the mark too, and only with it.
    assert_all(
        NavigationVerdict::Allow,
        &[
            "https://erplora.com/dashboard/account?surface=account",
            "https://erplora.com/dashboard/account/profile?surface=account",
            "https://erplora.com/dashboard/account/security?surface=account",
            "https://erplora.com/dashboard/account/preferences?surface=account",
            // Django's `request.GET.get` reads the LAST value of a repeated key.
            "https://erplora.com/dashboard/account/profile?surface=panel&surface=account",
            // The old addresses, on their way there.
            "https://erplora.com/dashboard/profile/?surface=account",
            "https://erplora.com/dashboard/profile/sessions/?surface=account",
            "https://erplora.com/account/profile?surface=account",
        ],
        "the account surface",
    );
    assert_all(
        NavigationVerdict::Refuse,
        &[
            // The same page INSIDE the panel, which has Billing a tap away.
            "https://erplora.com/dashboard/account",
            "https://erplora.com/dashboard/account/profile",
            "https://erplora.com/dashboard/account/profile?surface=panel",
            "https://erplora.com/dashboard/account/profile?surface=Account",
            // …and the last value is the one the SaaS reads.
            "https://erplora.com/dashboard/account/profile?surface=account&surface=panel",
            // The old addresses without the mark land in the panel too.
            "https://erplora.com/dashboard/profile/",
            "https://erplora.com/dashboard/profile/sessions/",
            "https://erplora.com/dashboard/profile/?surface=account&surface=panel",
            "https://erplora.com/account/profile",
            // `/account/` is no longer the sign-in (that is the app's start, `/shell/`): its old
            // sign-in address is a page of the panel now, followed only as the account surface.
            "https://erplora.com/account/login/?next=/shell/",
            // The mark opens the account pages, not the rest of the panel.
            "https://erplora.com/dashboard/billing/?surface=account",
            "https://erplora.com/dashboard/billing/invoices?surface=account",
            "https://erplora.com/dashboard/store/plans?surface=account",
            "https://erplora.com/dashboard/?surface=account",
            // On a segment boundary.
            "https://erplora.com/dashboard/accounts/?surface=account",
            "https://erplora.com/accountx/?surface=account",
        ],
        "the account page painted inside the panel",
    );
}

#[test]
fn the_account_is_deleted_from_inside_the_play_copy_and_ends_at_the_app_s_start() {
    // hub#2451: Google Play requires deleting the account from inside the app. «Delete your
    // account» is a card of Account → Profile, shown on every surface; once deleted, the page leaves
    // for the SaaS home page with a full load (`AFTER_DELETION = '/'` of the SaaS), which takes the
    // window to the app's own start. The old panel's «account deleted» page is not served any more:
    // it is a page like any other of the panel.
    assert_eq!(
        in_play("https://erplora.com/dashboard/account/profile?surface=account"),
        NavigationVerdict::Allow,
        "the page that deletes the account"
    );
    assert_eq!(
        in_play("https://erplora.com/"),
        NavigationVerdict::Home,
        "where deleting the account ends"
    );
    assert_eq!(
        in_play("https://erplora.com/dashboard/profile/deleted/"),
        NavigationVerdict::Refuse,
        "the old panel's «account deleted» page"
    );
}

// ── The list cannot be walked around ─────────────────────────────────────────────────────────────

#[test]
fn a_prefix_only_counts_on_a_segment_boundary() {
    assert_all(
        NavigationVerdict::Refuse,
        &[
            "https://erplora.com/shellfish/",
            "https://erplora.com/shell-pricing/",
            "https://erplora.com/accountsx/",
            "https://erplora.com/legalese/",
            "https://erplora.com/authors/",
            "https://erplora.com/launcher/",
            "https://erplora.com/login-pricing/",
            "https://erplora.com/passwords/",
            "https://erplora.com/invitationsx/",
            "https://erplora.com/hub-loginx/",
            "https://erplora.com/hub-login-evil/?hub=panaderia",
            "https://erplora.com/_allauth/browser/v1/auth/provider/redirect-pricing",
            // Django routes are case-sensitive: this is not the app's start, whatever it serves.
            "https://erplora.com/SHELL/",
        ],
        "a path that only LOOKS like an allowed one",
    );
}

#[test]
fn dot_segments_cannot_walk_out_of_an_allowed_page() {
    // The URL parser resolves `..` (also percent-encoded) before the path is looked at, so the
    // judged path is the one the server receives.
    assert_all(
        NavigationVerdict::Refuse,
        &[
            "https://erplora.com/shell/../pricing/",
            "https://erplora.com/shell/%2e%2e/pricing/",
            "https://erplora.com/legal/../dashboard/billing/",
        ],
        "a path that climbs out of an allowed prefix",
    );
}

#[test]
fn every_address_the_saas_answers_on_is_policed() {
    // `www` and `pre` serve the same site — measured on 18/09: `/pricing/` answers 200 on both —
    // and both look like a hub to the hub-domain predicate (`<label>.erplora.com`), so they have to
    // be named. The rest are spellings of the same host.
    assert_all(
        NavigationVerdict::Refuse,
        &[
            "https://www.erplora.com/pricing/",
            "https://pre.erplora.com/pricing/",
            "https://pre.erplora.com/dashboard/billing/",
            "http://erplora.com/pricing/",
            "https://erplora.com:443/pricing/",
            "https://erplora.com:8443/pricing/",
            "https://ERPLORA.com/pricing/",
            "https://erplora.com./pricing/",
            "https://www.erplora.com./dashboard/",
            "https://support@erplora.com/pricing/",
        ],
        "the SaaS under another spelling",
    );
}

#[test]
fn the_configured_saas_is_policed_as_well() {
    // `ERPLORA_SAAS_URL` points a build at another SaaS (a local one in development). Whatever the
    // app boots on is the SaaS, so the list applies to it too — judged by ORIGIN, so a hub served
    // from the same machine on another port is still a hub.
    let local = "http://127.0.0.1:8001";
    let judge = |raw: &str| navigation_verdict(PLAY, local, &raw.parse().expect("url"));
    assert_eq!(
        judge("http://127.0.0.1:8001/pricing/"),
        NavigationVerdict::Refuse
    );
    assert_eq!(
        judge("http://127.0.0.1:8001/shell/"),
        NavigationVerdict::Allow
    );
    assert_eq!(judge("http://127.0.0.1:8001/"), NavigationVerdict::Home);
    assert_eq!(judge("http://127.0.0.1:8787/pos"), NavigationVerdict::Allow);
    // And the production SaaS stays policed whatever the configuration says.
    assert_eq!(
        judge("https://erplora.com/pricing/"),
        NavigationVerdict::Refuse
    );
}

// ── …and the window actually asks ────────────────────────────────────────────────────────────────

/// A top-level function of the shell, from the real source, without its comments: prose is not
/// code, and a comment that names a call must not satisfy the checks below.
fn shell_function(name: &str) -> String {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    source
        .find(&format!("fn {name}"))
        .map(|at| &source[at..])
        .and_then(|rest| rest.find("\n}").map(|end| &rest[..end]))
        .unwrap_or_else(|| panic!("src/lib.rs no longer defines `{name}`"))
        .lines()
        .map(|line| line.find("//").map_or(line, |at| &line[..at]))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The body of the `on_navigation` handler of the main window.
fn main_window_navigation_handler() -> String {
    let window = shell_function("open_main_window");
    window
        .find(".on_navigation(")
        .map(|at| &window[at..])
        .and_then(|rest| rest.find(".build()").map(|end| &rest[..end]))
        .expect("`open_main_window` no longer registers an `on_navigation` handler")
        .to_string()
}

#[test]
fn the_main_window_asks_the_verdict_before_it_follows_a_page() {
    // A correct `navigation_verdict` that nobody consults is `true` without looking all over again —
    // the state this issue was opened against.
    let handler = main_window_navigation_handler();

    let asks = handler
        .find("navigation_verdict(distribution_channel(),")
        .expect("the `on_navigation` handler does not ask `navigation_verdict` for THIS copy (hub#1915)");
    // ONLY `Allow` is followed: `Home` is a refusal too — the SaaS home page carries the plan prices,
    // and the window is sent to the app's start instead of loading it.
    let only_allow = handler[asks..]
        .find("if verdict != NavigationVerdict::Allow {")
        .map(|at| asks + at)
        .expect("the handler must refuse every verdict but `Allow` (hub#1915)");
    let refuses = handler[only_allow..]
        .find("return false")
        .map(|at| only_allow + at)
        .expect("the `on_navigation` handler never refuses a page (hub#1915)");
    // …and a refusal is ANSWERED. Refusing in silence leaves «Log Out» (which lands on the SaaS home
    // page) on a stale page, and a refused link looking like a dead button.
    assert!(
        handler[only_allow..refuses].contains("refuse_navigation("),
        "the handler refuses a page without answering it: no notice, no way back to the start (hub#1915)"
    );

    // A refused page must not be remembered as this till's hub, nor become the page the connectivity
    // guard brings the window back to — the guard navigates by program, which no handler sees. The
    // capture goes through the linked-hub state since hub#2504 (`HubLink::follow`).
    for later in [".follow(nav)", "set_target("] {
        let at = handler
            .find(later)
            .unwrap_or_else(|| panic!("the handler no longer calls `{later}`"));
        assert!(
            refuses < at,
            "`{later}` runs before the verdict refuses the page: a refused page would still be \
             remembered or watched (hub#1915)"
        );
    }
}

#[test]
fn a_refused_page_is_answered_on_the_window_and_off_the_handler() {
    // `answer_refusal` is driven on a mock window by the unit tests; this is the link between the
    // handler and it, which needs a real `AppHandle` and so cannot be driven here.
    let refuse = shell_function("refuse_navigation");
    let off_the_handler = refuse
        .find("async_runtime::spawn(")
        .expect("`refuse_navigation` answers inside the handler: on Android that is the webview client's own callback (hub#1915)");
    assert!(
        refuse[off_the_handler..].contains("answer_refusal(&window, verdict,"),
        "`refuse_navigation` records the refused page but never answers it on the window (hub#1915)"
    );
}

// ── What is not the SaaS keeps today's behaviour ─────────────────────────────────────────────────

#[test]
fn pages_that_are_not_the_saas_are_followed_as_before() {
    assert_all(
        NavigationVerdict::Allow,
        &[
            // The hubs — where the app lives — whatever their aura.
            "https://panaderia.a.erplora.com/",
            "https://panaderia.a.erplora.com/?shell=1",
            "https://panaderia.a.erplora.com/m/sales/pos",
            "https://panaderia.3.erplora.com/pricing/",
            // Development.
            "http://127.0.0.1:8787/",
            "http://localhost:5173/pos",
            // The app's own bundled page (the offline screen).
            "http://tauri.localhost/index.html",
            "tauri://localhost/index.html",
            // Foreign hosts and non-web schemes: not the SaaS, not this guard.
            "https://accounts.google.com/o/oauth2/v2/auth?client_id=x",
            "about:blank",
        ],
        "a page that is not the SaaS",
    );
}

#[test]
fn the_other_copies_follow_every_link_as_before() {
    // Microsoft Store allows paying outside the store for non-game apps, and the direct install has
    // no store at all: both keep the behaviour they had.
    for distribution in ["direct", "msstore"] {
        for raw in [
            "https://erplora.com/",
            "https://erplora.com/pricing/",
            "https://erplora.com/dashboard/billing/",
            "https://erplora.com/dashboard/store/plans",
            "https://erplora.com/dashboard/account/profile",
            "https://www.erplora.com/marketplace/",
            "https://erplora.com/hub-login/?hub=panaderia&next=/&state=x",
        ] {
            assert_eq!(
                verdict_in(distribution, raw),
                NavigationVerdict::Allow,
                "the {distribution} copy must keep following {raw}"
            );
        }
    }
}
