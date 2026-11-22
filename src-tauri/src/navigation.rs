//! Which pages the main window follows.
//!
//! The window is the till: the printer, the cash drawer and the card reader answer the linked hub in
//! it. It shows the hubs, the SaaS, the origins it boots on, this device, the app's own page and, on
//! the computer, Google's sign-in; on Android any other page opens in the system browser instead, and
//! the window stays where it was ([`FOREIGN_PAGES_LEAVE`]). Without it, one tap on the YouTube logo of
//! a video player inside a hub page took the whole till to youtube.com. On Android Google's sign-in
//! goes to the device's browser too, through the app's own sign-in (`app_sign_in.rs`).
//!
//! The app boots on the SaaS (`/shell/`) and shows its sign-in, sign-up and account pages inside
//! its own window. If `on_navigation` answered `true` without looking, that window could walk to
//! ANY page of erplora.com: the home page with the plan prices, the public marketplace, the panel
//! with Billing. In the Google Play copy each of those is a door to paying outside Play billing — a
//! rejection reason. The templates cannot be the only barrier: the next link somebody adds to the
//! web opens the next door.
//!
//! So the APP decides. In the Play copy the SaaS gets a short list of pages the window follows —
//! sign in, sign up, the way into a hub, the account alone — and everything else of the SaaS is
//! refused, whatever the page links to. The list is judged on every navigation, so a page on it may
//! carry links to pages that are not: those are judged when tapped.
//!
//! The list polices the SaaS only. Hubs are where the app lives (and carry their own guard, the
//! `no-purchase-steering` test of the web app), the loopback is development, the bundled page is
//! ours, and a foreign page is sent out before the list is asked: the pages the list exists for are
//! reached through the SaaS.
//!
//! This governs the WINDOW. What leaves for the system browser (`open_external_url`) and the
//! panel's in-page navigation (its own router, without a full load) never reach `on_navigation`,
//! so neither is judged here.

use tauri::Url;

use crate::app_sign_in::{BROWSER_PATH, FINISH_PATH, START_PATH};
use crate::links::{is_hub_domain, is_loopback_host};

/// What `on_navigation` does with a page the window is about to load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationVerdict {
    /// Load it.
    Allow,
    /// Do not load it; the window stays where it is and says why.
    Refuse,
    /// Do not load it; the window goes to the app's own start instead (`/shell/`).
    Home,
    /// Do not load it; it opens in the system browser and the window stays where it is.
    Browser,
    /// Google's page on Android: do not load it; the app starts its own sign-in in the device's
    /// browser instead (`app_sign_in.rs`).
    AppSignIn,
    /// The browser half of that sign-in: it opens in a tab of the device's browser over the app.
    SignInTab,
}

/// Whether this system sends a page the window does not show to the system browser: only Android,
/// where `on_navigation` hears the window's own loads and none of its frames'. WKWebView and
/// WebKitGTK hand it the video player, the payment field and the captcha a page loads inside
/// itself, with nothing to tell them apart from the window's pages, so judging there would empty
/// every frame. WebView2 hears the window alone too, but the Store and direct copies show the Stripe
/// pages of the SaaS panel (an invoice to pay, the customer portal) in the window.
pub const FOREIGN_PAGES_LEAVE: bool = cfg!(target_os = "android");

/// The app's own page (the offline screen) where Tauri serves it over `http`: Windows and Android.
const BUNDLED_PAGE_HOST: &str = "tauri.localhost";

/// Google's sign-in, which «Continue with Google» walks the window through on the computer
/// (`PLAY_SAAS_PAGES`); on Android it starts the app's sign-in in the browser ([`sign_in_verdict`]).
const GOOGLE_SIGN_IN_HOST: &str = "accounts.google.com";

/// The pages the main window shows, fixed when it opens.
#[derive(Debug, Clone)]
pub struct WindowPages {
    /// The SaaS this build boots on (`ERPLORA_SAAS_URL` or the baked default).
    saas_base: String,
    /// The origins the window boots on: the SaaS, and the development override.
    entries: Vec<String>,
    /// [`FOREIGN_PAGES_LEAVE`] of the system the window runs on.
    foreign_pages_leave: bool,
}

impl WindowPages {
    pub fn new(saas_base: &str, entries: Vec<String>, foreign_pages_leave: bool) -> Self {
        Self {
            saas_base: saas_base.to_string(),
            entries,
            foreign_pages_leave,
        }
    }

    /// Does `target` leave for the system browser? Only a web page can: any other address is the
    /// webview's own business (`about:`, `data:`, `blob:`, the `tauri:` page).
    fn sends_out(&self, target: &Url) -> bool {
        self.foreign_pages_leave
            && matches!(target.scheme(), "http" | "https")
            && !self.shows(target)
    }

    /// Is `target` a page the window shows? Hubs and this device by host, whatever the scheme or
    /// port, like the SaaS; the origins the window boots on whole. Google's sign-in is not one: it
    /// never reaches here on Android, the only system where a page leaves ([`sign_in_verdict`]).
    fn shows(&self, target: &Url) -> bool {
        if is_saas(&self.saas_base, target)
            || self.entries.contains(&target.origin().ascii_serialization())
        {
            return true;
        }
        let Some(host) = target.host_str() else {
            return false;
        };
        let host = host.strip_suffix('.').unwrap_or(host);
        is_hub_domain(host) || is_loopback_host(host) || host == BUNDLED_PAGE_HOST
    }
}

/// The copy of the app that is held to Google Play's payments policy (`distribution_channel`).
const PLAY: &str = "play";

/// Every host the production SaaS answers on, apex included. `www` and `pre` have to be named: both
/// serve the full site (`/pricing/` answers 200 on each) and both have the shape of
/// a hub (`<label>.erplora.com`), which is what everything else under `erplora.com` is.
const SAAS_HOSTS: &[&str] = &["erplora.com", "www.erplora.com", "pre.erplora.com"];

/// The query mark that paints the account WITHOUT the panel.
const ACCOUNT_SURFACE: (&str, &str) = ("surface", "account");

/// The account pages, which the Play copy opens only as the account surface, as path prefixes:
/// Account under the panel, and the old addresses the SaaS sends there for good keeping the query —
/// the old panel's profile and `/account/`.
/// Deleting the account ends on the SaaS home page, which is the app's start (`Home`).
const ACCOUNT_PAGES: &[&str] = &["/dashboard/account/", "/dashboard/profile/", "/account/"];

/// The pages of the SaaS the Play copy follows, as path prefixes: the ones it serves today, an old
/// address only while the SaaS still redirects it. None of them sells, and a link they carry to one
/// that does is judged again when tapped.
const PLAY_SAAS_PAGES: &[&str] = &[
    // The app's own start (the launcher under the address the app boots on) and its «Open».
    "/shell/",
    // The launcher: where the SaaS signs in and signs up (`SPA_LOGIN`); `/launch/` leads there.
    "/launch/",
    // The old sign-in, which leads to the launcher, and where a refused sign-in or jump lands.
    "/login/",
    // Verify the e-mail, reset the password, the second factor and its settings, «Confirm it's you».
    "/verify-email/",
    "/password/",
    "/2fa/",
    "/reauthenticate/",
    // allauth's old addresses of the sign-in and the sign-up, which lead to the launcher; the rest
    // of its old pages are gone (404).
    "/accounts/login/",
    "/accounts/signup/",
    // «Continue with Google»: the form posted to allauth, which sends the window to Google, and
    // Google's way back.
    "/_allauth/browser/v1/auth/provider/redirect/",
    "/auth/google/callback/",
    // Choosing a business (Businesses of the panel, and its old address), and joining
    // one: every invitation waiting, or one first.
    "/dashboard/businesses/",
    "/businesses/",
    "/invitation/",
    "/invitations/",
    // The jump into a hub: «Open» starts at the hub's `/auth/login`, which bounces here; this door
    // posts a one-time code to the hub's callback, or sends the jump to the launcher, «Confirm it's
    // you» or the page that says why it stopped.
    "/hub-login/",
    // Google Play requires the privacy policy to be reachable from inside the app; the cookie
    // banner of these pages links to its settings.
    "/legal/",
    "/cookies/",
];

/// The verdict for `target` in the `distribution` copy of the app, whose main window shows `window`.
///
/// A page the window does not show leaves for the system browser in every copy, where this system
/// sends such pages out ([`FOREIGN_PAGES_LEAVE`]). Of the SaaS, every copy but Play follows every
/// link: Microsoft Store allows paying outside the store for non-game apps, and the direct install
/// has no store at all.
pub fn navigation_verdict(distribution: &str, window: &WindowPages, target: &Url) -> NavigationVerdict {
    if let Some(verdict) = sign_in_verdict(window, target) {
        return verdict;
    }
    if window.sends_out(target) {
        return NavigationVerdict::Browser;
    }
    if distribution != PLAY || !is_saas(&window.saas_base, target) {
        return NavigationVerdict::Allow;
    }
    let path = target.path();
    if path == "/" {
        // The SaaS sends people to its home page without asking — the «Log Out» POST lands there,
        // and so do the logo and the «←» of the sign-in pages. Inside the app, home is the app's
        // start, which asks for credentials again when the session is gone.
        return NavigationVerdict::Home;
    }
    if PLAY_SAAS_PAGES.iter().any(|prefix| under(path, prefix)) || is_account_surface(target) {
        NavigationVerdict::Allow
    } else {
        NavigationVerdict::Refuse
    }
}

/// The pages of «Continue with Google» the app answers itself. The start and the finish of its sign-in
/// are refused to every page, in every copy: the app loads them on its own (`AppSignIn::admits`), and a
/// link to the finish could open someone else's session in this window. On Android, Google's page
/// starts that sign-in and its browser half opens in a tab of the device's browser.
fn sign_in_verdict(window: &WindowPages, target: &Url) -> Option<NavigationVerdict> {
    let saas = is_saas(&window.saas_base, target);
    if saas && (under(target.path(), START_PATH) || under(target.path(), FINISH_PATH)) {
        return Some(NavigationVerdict::Refuse);
    }
    if !window.foreign_pages_leave {
        return None;
    }
    if target.scheme() == "https" && target.host_str() == Some(GOOGLE_SIGN_IN_HOST) {
        return Some(NavigationVerdict::AppSignIn);
    }
    (saas && under(target.path(), BROWSER_PATH)).then_some(NavigationVerdict::SignInTab)
}

/// Is `target` a page of the SaaS? The production hosts by NAME, whatever the scheme or port — a
/// different spelling is still the same site — and the configured one by ORIGIN, so that a hub
/// served from the same development machine on another port is still a hub.
fn is_saas(saas_base: &str, target: &Url) -> bool {
    let Some(host) = target.host_str() else {
        return false;
    };
    // A fully-qualified `erplora.com.` resolves to the same site, and Django accepts it: it strips
    // the trailing dot before validating the host.
    let host = host.strip_suffix('.').unwrap_or(host);
    if SAAS_HOSTS.contains(&host) {
        return true;
    }
    match saas_base.parse::<Url>() {
        Ok(base) => base.origin() == target.origin(),
        Err(_) => false,
    }
}

/// Is `path` the page `prefix` names or one below it? On a segment boundary: `/shellfish/` is not
/// `/shell/`. The bare form without the trailing slash counts too — Django answers it with a
/// redirect to the slashed one.
fn under(path: &str, prefix: &str) -> bool {
    path.starts_with(prefix) || path == prefix.trim_end_matches('/')
}

/// The account pages as the account surface.
///
/// The mark is read the way the SaaS reads it — Django's `request.GET.get` returns the LAST value of
/// a repeated key — so `?surface=account&surface=panel` is the panel, and is refused.
fn is_account_surface(target: &Url) -> bool {
    let path = target.path();
    if !ACCOUNT_PAGES.iter().any(|prefix| under(path, prefix)) {
        return false;
    }
    let (key, value) = ACCOUNT_SURFACE;
    target
        .query_pairs()
        .filter(|(k, _)| k == key)
        .last()
        .is_some_and(|(_, v)| v == value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(raw: &str) -> Url {
        raw.parse().expect("test URL")
    }

    #[test]
    fn a_prefix_matches_itself_below_and_bare_but_never_a_longer_name() {
        assert!(under("/shell/", "/shell/"));
        assert!(under("/shell/create/", "/shell/"));
        assert!(under("/shell", "/shell/"));
        assert!(!under("/shellfish/", "/shell/"));
        assert!(!under("/shel", "/shell/"));
    }

    #[test]
    fn an_unparseable_saas_base_leaves_the_production_hosts_policed() {
        let base = "not a url";
        assert!(is_saas(base, &url("https://erplora.com/pricing/")));
        assert!(!is_saas(base, &url("https://panaderia.a.erplora.com/")));
    }

    #[test]
    fn a_page_that_is_not_served_from_a_saas_host_is_not_the_saas() {
        assert!(!is_saas("https://erplora.com", &url("about:blank")));
        assert!(!is_saas(
            "https://erplora.com",
            &url("tauri://localhost/index.html")
        ));
    }
}
