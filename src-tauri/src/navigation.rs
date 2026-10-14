//! Which pages of the SaaS the main window follows (hub#1915).
//!
//! The app boots on the SaaS (`/shell/`) and shows its sign-in, sign-up and account pages inside
//! its own window. For as long as `on_navigation` answered `true` without looking, that window could
//! walk to ANY page of erplora.com: the home page with the plan prices, the public marketplace, the
//! panel with Billing. In the Google Play copy each of those is a door to paying outside Play
//! billing — a rejection reason — and one round of the Android QA found six, every one closed by
//! hand in its template. The templates cannot be the only barrier: the next link somebody adds to
//! the web opens the next door.
//!
//! So the APP decides. In the Play copy the SaaS gets a short list of pages the window follows —
//! sign in, sign up, the way into a hub, the account alone — and everything else of the SaaS is
//! refused, whatever the page links to. The list is judged on every navigation, so a page on it may
//! carry links to pages that are not: those are judged when tapped.
//!
//! Only the SaaS is policed. Hubs are where the app lives (and carry their own guard, the
//! `no-purchase-steering` test of the web app), the loopback is development, the bundled page is
//! ours, and foreign hosts keep the behaviour they had: the pages this exists for are reached
//! through the SaaS.
//!
//! This governs the WINDOW. What leaves for the system browser (`open_external_url`) and the
//! panel's in-page htmx navigation never reach `on_navigation`; both are hub#1918.

use tauri::Url;

/// What `on_navigation` does with a page the window is about to load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationVerdict {
    /// Load it.
    Allow,
    /// Do not load it; the window stays where it is and says why.
    Refuse,
    /// Do not load it; the window goes to the app's own start instead (`/shell/`).
    Home,
}

/// The copy of the app that is held to Google Play's payments policy (`distribution_channel`).
const PLAY: &str = "play";

/// Every host the production SaaS answers on, apex included. `www` and `pre` have to be named: both
/// serve the full site (`/pricing/` answers 200 on each, measured 18/09) and both have the shape of
/// a hub (`<label>.erplora.com`), which is what everything else under `erplora.com` is.
const SAAS_HOSTS: &[&str] = &["erplora.com", "www.erplora.com", "pre.erplora.com"];

/// The query mark that paints the account WITHOUT the panel (saas#2084, hub#1900).
const ACCOUNT_SURFACE: (&str, &str) = ("surface", "account");

/// The account pages, which the Play copy opens only as the account surface.
const ACCOUNT_PAGES: &str = "/dashboard/profile/";

/// Where deleting the account from the account surface ends: signed out, so without the mark, and
/// with no way out (saas#2084).
const ACCOUNT_DELETED: &str = "/dashboard/profile/deleted/";

/// The pages of the SaaS the Play copy follows, as path prefixes. None of them sells, and a link
/// they carry to one that does is judged again when tapped.
const PLAY_SAAS_PAGES: &[&str] = &[
    // The app's own start: pick, open or create a hub.
    "/shell/",
    // The unified sign-in / sign-up screen, and its alias.
    "/account/",
    "/register/",
    // allauth: verify the e-mail, reset the password, second factor, reauthenticate, sign out.
    "/accounts/",
    // Google sign-in and the one-time hand-offs into a hub.
    "/auth/",
    // The router after sign-in; the page it redirects to is judged on its own.
    "/post-login/",
    // The first free hub, waking a sleeping one, the walk into a hub, choosing one, joining one,
    // and the page shown while it boots.
    "/start/",
    "/enter/",
    "/businesses/",
    "/invitation/",
    "/hub/starting/",
    // Google Play requires the privacy policy to be reachable from inside the app; the cookie
    // banner of these pages links to its settings.
    "/legal/",
    "/cookies/",
];

/// The verdict for `target` in the `distribution` copy of the app, `saas_base` being the SaaS this
/// build boots on (`ERPLORA_SAAS_URL` or the baked default).
///
/// Every copy but Play follows every link, as before: Microsoft Store allows paying outside the
/// store for non-game apps, and the direct install has no store at all.
pub fn navigation_verdict(distribution: &str, saas_base: &str, target: &Url) -> NavigationVerdict {
    if distribution != PLAY || !is_saas(saas_base, target) {
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

/// The account pages as the account surface, or the page that ends its deletion.
///
/// The mark is read the way the SaaS reads it — Django's `request.GET.get` returns the LAST value of
/// a repeated key — so `?surface=account&surface=panel` is the panel, and is refused.
fn is_account_surface(target: &Url) -> bool {
    let path = target.path();
    if path == ACCOUNT_DELETED {
        return true;
    }
    if !path.starts_with(ACCOUNT_PAGES) {
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
