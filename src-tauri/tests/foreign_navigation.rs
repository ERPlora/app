//! Which pages the main window shows, and where it sends the rest.
//!
//! The window is the till: the printer, the cash drawer and the card reader answer the linked hub
//! in it, and nothing else. On Android one tap on the YouTube logo of a video player inside a hub
//! page left the till on youtube.com with the hub gone. A page the window does not show therefore
//! opens in the system browser, and the window stays where it was.
//!
//! What the window shows: the hubs, the SaaS, the origins it boots on, the loopback of development,
//! the app's own page and Google's sign-in, which «Continue with Google» walks the window through.

use erplora_tauri_lib::{navigation_verdict, NavigationVerdict, WindowPages, FOREIGN_PAGES_LEAVE};

const SAAS: &str = "https://erplora.com";
const COPIES: [&str; 3] = ["play", "direct", "msstore"];

/// The window of a copy whose `on_navigation` hears the window's own loads only (Android), booted
/// on `entries`.
fn sends_foreign_pages_out(entries: &[&str]) -> WindowPages {
    WindowPages::new(SAAS, entries.iter().map(|e| e.to_string()).collect(), true)
}

fn android() -> WindowPages {
    sends_foreign_pages_out(&[SAAS])
}

fn verdict(window: &WindowPages, distribution: &str, raw: &str) -> NavigationVerdict {
    let url: tauri::Url = raw
        .parse()
        .unwrap_or_else(|e| panic!("{raw} is not a URL: {e}"));
    navigation_verdict(distribution, window, &url)
}

/// Every address gets `expected` in every copy of the app.
fn assert_everywhere(
    window: &WindowPages,
    expected: NavigationVerdict,
    addresses: &[&str],
    why: &str,
) {
    let wrong: Vec<String> = COPIES
        .iter()
        .flat_map(|copy| addresses.iter().map(move |raw| (copy, raw)))
        .filter_map(|(copy, raw)| {
            let got = verdict(window, copy, raw);
            (got != expected).then(|| format!("{copy}: {raw} → {got:?}"))
        })
        .collect();
    assert!(
        wrong.is_empty(),
        "expected {expected:?} ({why}), but:\n  {}",
        wrong.join("\n  ")
    );
}

// ── The positive control: what takes the till away today ─────────────────────────────────────────

#[test]
fn a_foreign_page_opens_in_the_system_browser_in_every_copy() {
    assert_everywhere(
        &android(),
        NavigationVerdict::Browser,
        &[
            // The YouTube logo of an embedded player, and where YouTube sends a phone next.
            "https://www.youtube.com/watch?v=M7lc1UVf-VE",
            "https://m.youtube.com/watch?v=M7lc1UVf-VE",
            "https://www.youtube-nocookie.com/embed/M7lc1UVf-VE",
            // Google, but not its sign-in.
            "https://www.google.com/",
            "https://maps.google.com/?q=Madrid",
            // Any other site, over https or not.
            "https://example.com/",
            "http://example.com/page",
            // A business's own domain: the app opens the business on its platform address, so a
            // page there is its public website, not the till.
            "https://www.peluqueria-ana.es/reservar",
        ],
        "a page the window does not show",
    );
}

#[test]
fn a_look_alike_of_a_page_the_window_shows_is_foreign() {
    assert_everywhere(
        &android(),
        NavigationVerdict::Browser,
        &[
            "https://erplora.com.evil.example/",
            "https://evil-erplora.com/",
            "https://evilerplora.com/shell/",
            "https://panaderia.a.erplora.com.evil.example/",
            "https://accounts.google.com.evil.example/o/oauth2/v2/auth",
            "https://evil.example/accounts.google.com/",
            // The part before `@` is a user name: the host is the one after it.
            "https://erplora.com@evil.example/shell/",
            "https://127.0.0.1.evil.example/",
            "https://localhost.evil.example/",
            "http://tauri.localhost.evil.example/index.html",
        ],
        "only the host decides, and it is matched whole",
    );
}

// ── What the window keeps showing ────────────────────────────────────────────────────────────────

#[test]
fn the_hubs_stay_in_the_window_whatever_their_aura() {
    assert_everywhere(
        &android(),
        NavigationVerdict::Allow,
        &[
            // The linked hub and its siblings: another business is still a hub, and the hardware
            // answers only the linked one anyway.
            "https://panaderia.a.erplora.com/",
            "https://panaderia.a.erplora.com/?shell=1",
            "https://panaderia.a.erplora.com/m/sales/pos",
            "https://otronegocio.3.erplora.com/",
            "http://panaderia.a.erplora.com/",
            "https://panaderia.a.erplora.com./m/sales/pos",
        ],
        "a hub is where the app lives",
    );
}

#[test]
fn the_saas_stays_in_the_window_and_play_still_judges_its_pages() {
    let window = android();
    for raw in [
        "https://erplora.com/",
        "https://erplora.com/shell/",
        "https://www.erplora.com/shell/",
        "https://pre.erplora.com/shell/",
        "https://erplora.com./shell/",
        "https://erplora.com/dashboard/billing/",
    ] {
        assert_eq!(
            verdict(&window, "direct", raw),
            NavigationVerdict::Allow,
            "{raw}"
        );
    }
    assert_eq!(
        verdict(&window, "play", "https://erplora.com/shell/"),
        NavigationVerdict::Allow
    );
    assert_eq!(
        verdict(&window, "play", "https://erplora.com/pricing/"),
        NavigationVerdict::Refuse
    );
    assert_eq!(
        verdict(&window, "play", "https://erplora.com/"),
        NavigationVerdict::Home
    );
}

#[test]
fn a_saas_configured_elsewhere_stays_in_the_window() {
    let base = "https://saas.example.test";
    let window = WindowPages::new(base, vec![base.to_string()], true);
    assert_eq!(
        verdict(&window, "direct", "https://saas.example.test/shell/"),
        NavigationVerdict::Allow
    );
    assert_eq!(
        verdict(&window, "play", "https://saas.example.test/pricing/"),
        NavigationVerdict::Refuse
    );
    assert_eq!(
        verdict(&window, "direct", "https://other.example.test/"),
        NavigationVerdict::Browser
    );
}

#[test]
fn the_origin_the_window_boots_on_stays_in_the_window() {
    // The development override may boot the window on a machine of the local network.
    let window = sends_foreign_pages_out(&[SAAS, "http://192.168.1.20:8787"]);
    assert_everywhere(
        &window,
        NavigationVerdict::Allow,
        &[
            "http://192.168.1.20:8787/",
            "http://192.168.1.20:8787/m/sales/pos",
        ],
        "the window boots there",
    );
    assert_everywhere(
        &window,
        NavigationVerdict::Browser,
        &["http://192.168.1.20:9999/", "https://192.168.1.20:8787/"],
        "the same machine on another port or scheme is another origin",
    );
}

#[test]
fn the_loopback_and_the_app_s_own_page_stay_in_the_window() {
    assert_everywhere(
        &android(),
        NavigationVerdict::Allow,
        &[
            // Development, and the hub the app accepts on loopback in every copy.
            "http://127.0.0.1:8787/",
            "http://localhost:5173/pos",
            "https://localhost:8443/",
            "http://[::1]:8787/",
            // The offline page, served by the app itself.
            "http://tauri.localhost/index.html",
        ],
        "this device and the app's own page",
    );
}

#[test]
fn continue_with_google_takes_the_window_to_google_and_back() {
    assert_everywhere(
        &android(),
        NavigationVerdict::Allow,
        &[
            // The form the SaaS posts, which allauth answers with Google's page.
            "https://erplora.com/_allauth/browser/v1/auth/provider/redirect",
            // Google's sign-in, as the window walks it: the start, the account, the consent.
            "https://accounts.google.com/o/oauth2/v2/auth?client_id=x&redirect_uri=https%3A%2F%2Ferplora.com%2Fauth%2Fgoogle%2Fcallback%2F",
            "https://accounts.google.com/v3/signin/identifier?flowName=GeneralOAuthFlow",
            "https://accounts.google.com/signin/oauth/v3/consent?authuser=0",
            // And the way back.
            "https://erplora.com/auth/google/callback/?code=x&state=y",
        ],
        "«Continue with Google» runs in the window",
    );
    // Google's sign-in is served over https only: anything else is not the page the SaaS sent.
    assert_eq!(
        verdict(
            &android(),
            "direct",
            "http://accounts.google.com/o/oauth2/v2/auth"
        ),
        NavigationVerdict::Browser
    );
}

#[test]
fn the_pages_that_charge_or_check_a_receipt_open_in_the_system_browser() {
    assert_everywhere(
        &android(),
        NavigationVerdict::Browser,
        &[
            // Stripe: the assistant's checkout, an invoice to pay, the customer portal.
            "https://checkout.stripe.com/c/pay/cs_test_x",
            "https://invoice.stripe.com/i/acct_x/test_x",
            "https://billing.stripe.com/p/session/test_x",
            // The tax agency's check of a VeriFactu QR, on its real and its test site.
            "https://www2.agenciatributaria.gob.es/wlpl/TIKE-CONT/ValidarQR?nif=B00000000&numserie=A-1",
            "https://prewww2.aeat.es/wlpl/TIKE-CONT/ValidarQR?nif=B00000000&numserie=A-1",
        ],
        "paying and checking happen in the browser, as `open_external_url` does it",
    );
}

#[test]
fn an_address_that_is_not_a_web_page_is_not_judged() {
    assert_everywhere(
        &android(),
        NavigationVerdict::Allow,
        &[
            "about:blank",
            "about:srcdoc",
            "data:text/html,hello",
            "blob:https://panaderia.a.erplora.com/0b9a3c0e-5d7f-4e8b-9a51-1f2d3c4b5a69",
            "tauri://localhost/index.html",
        ],
        "only web pages can leave for the browser",
    );
}

// ── Where `on_navigation` also hears the frames inside a page ────────────────────────────────────

#[test]
fn where_on_navigation_hears_the_frames_of_a_page_a_foreign_page_loads_as_before() {
    // WKWebView and WebKitGTK hand `on_navigation` the player of a video, a payment field and a
    // captcha that a page loads inside itself, with nothing to tell them from the window's own
    // pages: sending those out would empty every frame and open the browser instead.
    let window = WindowPages::new(SAAS, vec![SAAS.to_string()], false);
    assert_everywhere(
        &window,
        NavigationVerdict::Allow,
        &[
            "https://www.youtube-nocookie.com/embed/M7lc1UVf-VE",
            "https://js.stripe.com/v3/elements-inner-card.html",
            "https://www.google.com/recaptcha/api2/anchor?k=x",
        ],
        "a frame inside a page",
    );
    // The Play list of the SaaS does not depend on it.
    assert_eq!(
        verdict(&window, "play", "https://erplora.com/pricing/"),
        NavigationVerdict::Refuse
    );
}

#[test]
fn only_android_sends_foreign_pages_out() {
    // Android's `on_navigation` hears the window's own loads and none of its frames'. On macOS and
    // Linux it hears the frames too (see the test above); on Windows the Store and direct copies
    // show the Stripe pages of the SaaS panel in the window.
    assert_eq!(
        FOREIGN_PAGES_LEAVE,
        cfg!(target_os = "android"),
        "this system would send the frames of a page or the panel's payments to the browser, \
         or leave the window to any site"
    );
}

// ── …and the window actually sends them ──────────────────────────────────────────────────────────

/// A top-level function of the main window's module, from the real source, without its comments.
fn window_function(name: &str) -> String {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/window.rs");
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    source
        .find(&format!("fn {name}"))
        .map(|at| &source[at..])
        .and_then(|rest| rest.find("\n}").map(|end| &rest[..end]))
        .unwrap_or_else(|| panic!("src/window.rs no longer defines `{name}`"))
        .lines()
        .map(|line| line.find("//").map_or(line, |at| &line[..at]))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_main_window_judges_with_this_system_s_rule_and_the_origins_it_boots_on() {
    let window = window_function("open_main_window");
    assert!(
        window.contains("WindowPages::new(&saas_base, entries.clone(), FOREIGN_PAGES_LEAVE)"),
        "the main window does not judge its pages with the origins it boots on and this system's rule"
    );
}

#[test]
fn a_page_the_window_does_not_show_opens_in_the_system_browser_off_the_handler() {
    let refuse = window_function("refuse_navigation");
    let off_the_handler = refuse
        .find("async_runtime::spawn(")
        .expect("`refuse_navigation` answers inside the handler: on Android that is the webview client's own callback");
    let opens = refuse[off_the_handler..]
        .find("open_in_browser(&app, &target)")
        .expect("a page the window does not show is never handed to the system browser");
    assert!(
        refuse[off_the_handler..off_the_handler + opens].contains("NavigationVerdict::Browser"),
        "the system browser is not kept for the pages the window does not show"
    );
    let open = window_function("open_in_browser");
    assert!(
        open.contains(".open_url(target.as_str(), None::<&str>)"),
        "the page does not go to the system browser as its own app: {open}"
    );
}
