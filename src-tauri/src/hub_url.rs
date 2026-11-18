//! Shell state: capturing and persisting hub_url.

use std::path::Path;

use crate::links::{is_hub_domain, is_loopback_host};
use crate::{ShellError, DEFAULT_SAAS_URL, ENV_SAAS_URL};

/// Persisted origin of the hub PWA captured through `?shell=1`.
const HUB_URL_FILE: &str = "hub.url";

/// Normalizes the SaaS base URL: no surrounding spaces and no trailing `/`.
pub(crate) fn normalize_base(base: &str) -> String {
    base.trim().trim_end_matches('/').to_string()
}

/// URL of the SaaS onboarding that the first start loads.
pub(crate) fn onboarding_url(base: &str) -> String {
    format!("{}/shell/", normalize_base(base))
}

/// SaaS base URL: the [`ENV_SAAS_URL`] environment variable or the baked-in default.
pub(crate) fn saas_base_url() -> String {
    normalize_base(
        &std::env::var(ENV_SAAS_URL)
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_SAAS_URL.to_string()),
    )
}

/// The ORIGIN this installation may enter "app mode" on, or `None` if it may not.
///
/// The trust boundary of `hub.url`, and the reason it is a boundary at all: whatever ends up in
/// that file is what the window opens FULL-SCREEN, chrome-less and titled "ERPlora" on every cold
/// start from then on. So the destination is checked, not merely parsed — a hub of ours, or the
/// loopback of this machine, and nothing else.
///
/// It is deliberately the SAME rule as `links::hub_url_for_host` (what a deep link may open) and
/// as `capabilities/*.json` (what may drive the hardware); `tests/remote_acl.rs`
/// asserts the capture and the capabilities agree origin by origin. They have to: remembering an
/// origin the capabilities refuse produces a till that looks fine and cannot print, because every
/// `invoke` from it — down to `device_context` — is rejected by Tauri's ACL.
///
/// `https` everywhere, plus plain `http` on loopback for development. There is no opaque-origin
/// case left to reject: only `http`/`https` get here, and both are special schemes whose origin is
/// always a tuple.
pub(crate) fn trusted_hub_origin(url: &tauri::Url) -> Option<String> {
    // The `None` branch is unreachable: the scheme gate below only lets `http`/`https` through, and
    // neither parses without a host. The `?` stays so the function is total on its own terms
    // instead of depending on the order of the two checks.
    let host = url.host_str()?;
    let allowed = match url.scheme() {
        "https" => is_hub_domain(host) || is_loopback_host(host),
        // A deployed hub is always https; plain http is the development exemption.
        "http" => is_loopback_host(host),
        _ => false,
    };
    if !allowed {
        return None;
    }
    Some(url.origin().ascii_serialization())
}

/// Capture contract: when the navigation carries the `?shell=1` marker **and** the destination is
/// one of ours ([`trusted_hub_origin`]), returns the ORIGIN to persist as `hub_url`.
///
/// The marker says "remember me", not "trust me": it rides on the URL being navigated to, and the
/// window still loads many pages that are not a hub ([`navigation_verdict`]): the SaaS, Google's
/// sign-in and, on the desktop, any foreign page. Without the second filter one link is enough to
/// leave the till booting on somebody else's page, for good.
///
/// [`navigation_verdict`]: crate::navigation_verdict
pub fn shell_capture_origin(url: &tauri::Url) -> Option<String> {
    let has_marker = url.query_pairs().any(|(k, v)| k == "shell" && v == "1");
    if !has_marker {
        return None;
    }
    let captured = trusted_hub_origin(url);
    if captured.is_none() {
        log::warn!(
            "shell: {} asks to be remembered and is not one of our hubs; not captured",
            url.origin().ascii_serialization()
        );
    }
    captured
}

/// Reads the persisted `hub.url`, reduced to its ORIGIN and passed through [`trusted_hub_origin`].
///
/// It is revalidated on READ, not only on write, because guarding the capture only protects the
/// tills that have not captured anything yet: one that already ran a build without the filter
/// carries the foreign origin in the file, and an update that trusts it again does not fix exactly
/// the case that matters. It is also a text file in the user's data directory: nothing stops
/// anyone from editing it.
pub(crate) fn load_hub_url(cache_dir: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(cache_dir.join(HUB_URL_FILE)).ok()?;
    // No `is_empty` guard: an empty (or blank) file does not parse as a URL, so the `?` below is
    // the same rejection written once instead of twice.
    //
    // The WHATWG parser strips leading and trailing C0 and space itself, so the `trim` changes no
    // outcome today. It stays because `hub.url` is OUR file format: normalizing it here says so,
    // instead of leaning on a parsing detail of a dependency to make our own writes readable.
    let trimmed = raw.trim();
    let parsed: tauri::Url = trimmed.parse().ok()?;
    let trusted = trusted_hub_origin(&parsed);
    if trusted.is_none() {
        log::warn!(
            "shell: the remembered hub ({trimmed}) is not one of ours; back to the onboarding"
        );
    }
    trusted
}

/// Persists the captured origin as `hub.url` (creating `cache_dir` if it does not exist).
pub(crate) fn persist_hub_url(cache_dir: &Path, origin: &str) -> Result<(), ShellError> {
    std::fs::create_dir_all(cache_dir).map_err(|e| ShellError::Io(e.to_string()))?;
    std::fs::write(cache_dir.join(HUB_URL_FILE), origin).map_err(|e| ShellError::Io(e.to_string()))
}

/// Forgets the captured hub (best effort: no error if there was none).
pub(crate) fn clear_hub_url(cache_dir: &Path) {
    let _ = std::fs::remove_file(cache_dir.join(HUB_URL_FILE));
}

/// What the remembered hub answers when it is asked at start-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HubProbe {
    /// It answered with this HTTP status.
    Status(u16),
    /// It could not be reached: no network, DNS down, timeout.
    Unreachable,
}

/// Must the remembered `hub.url` be forgotten?
///
/// **Only** if the hub no longer exists. The decision is asymmetric on purpose: forgetting too
/// eagerly deletes the user's hub and sends them through the onboarding again, while forgetting
/// too little only leaves an ugly screen that fixes itself as soon as the hub is back. When in
/// doubt, it is kept.
///
/// It exists because [`forget_hub`] **does not reach this case**: the frontend calls it on a 410
/// from the server, and if the hub was deleted the PWA never loads at all — the 404 is served by
/// the edge. Without this the app opens on «404 page not found» for good, with no way out through
/// the UI. Seen on a Mac with a hub that a production purge had removed.
///
/// [`forget_hub`]: crate::window::forget_hub
pub(crate) fn should_forget_hub(probe: HubProbe) -> bool {
    match probe {
        // The hub is not there. 410 is also the server's explicit `hub_not_found` contract.
        HubProbe::Status(404) | HubProbe::Status(410) => true,
        // Everything else (alive, redirecting to the login, unauthenticated, down or unreachable)
        // is a hub that DOES exist.
        _ => false,
    }
}

/// The window's start URL, by precedence: the start-up deep link → the development override
/// ([`ENV_SHELL_URL`]) → the persisted `hub.url` (app mode) → the SaaS onboarding. Pure so that it
/// can be tested.
///
/// The deep link goes **first** because it is what the user is asking for NOW: the override is
/// static configuration and the persisted hub is last time's. If either of them won, pressing
/// «Open Terminal» on hub B from a till that remembers hub A would open A — the wrong business —
/// without a single error to give it away.
///
/// [`ENV_SHELL_URL`]: crate::ENV_SHELL_URL
pub(crate) fn initial_url_for(
    deep_link: Option<&str>,
    override_url: Option<&str>,
    persisted: Option<&str>,
    saas_base: &str,
) -> String {
    if let Some(target) = deep_link {
        return target.to_string();
    }
    if let Some(dev) = override_url {
        return dev.to_string();
    }
    if let Some(origin) = persisted {
        return format!("{}/", normalize_base(origin));
    }
    onboarding_url(saas_base)
}

/// Which pages may choose a hub (the entries of [`hub_link::HubLink`]) and which hub the window
/// starts linked to, by the precedence of [`initial_url_for`].
///
/// The development override is an entry (it may boot at a local SaaS that chooses the hub) and is
/// linked only when it is a hub of ours that is not the SaaS itself (a local PWA).
///
/// [`hub_link::HubLink`]: crate::hub_link::HubLink
pub(crate) fn boot_link(
    override_url: Option<&str>,
    persisted: Option<&str>,
    saas_base: &str,
) -> (Vec<String>, Option<String>) {
    let origin = |raw: &str| raw.parse::<tauri::Url>().ok().map(|u| u.origin().ascii_serialization());
    let hub = |raw: &str| raw.parse::<tauri::Url>().ok().as_ref().and_then(trusted_hub_origin);
    let saas = origin(saas_base);
    let mut entries: Vec<String> = saas.iter().cloned().collect();
    if let Some(dev) = override_url.and_then(origin) {
        if !entries.contains(&dev) {
            entries.push(dev);
        }
    }
    let linked = match override_url {
        Some(dev) => hub(dev).filter(|dev| Some(dev) != saas.as_ref()),
        None => persisted.map(str::to_string),
    };
    (entries, linked)
}

/// What a cold start does with the link it was launched by.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ColdStartLink {
    /// Launched without one.
    None,
    /// A link to the hub the device is linked to: the window boots on it, as it always did.
    Open(String),
    /// A link to any other hub: the window boots where it would without a link, and the person is
    /// asked ([`open_hub_from_link`]) before anything is linked.
    ///
    /// [`open_hub_from_link`]: crate::window::open_hub_from_link
    Ask(String),
}

pub(crate) fn cold_start_link(deep_link: Option<&str>, linked: Option<&str>) -> ColdStartLink {
    let Some(target) = deep_link else {
        return ColdStartLink::None;
    };
    let hub = target.parse::<tauri::Url>().ok().as_ref().and_then(trusted_hub_origin);
    match hub {
        Some(hub) if linked == Some(hub.as_str()) => ColdStartLink::Open(target.to_string()),
        _ => ColdStartLink::Ask(target.to_string()),
    }
}

/// Where the window goes after forgetting the hub.
///
/// Two callers, two intents. The 410 path (`choose = false`) keeps the plain onboarding: the hub
/// is gone, the SaaS routes as it sees fit — and with a single hub that means straight back in,
/// which is right there. The USER path («switch business», `choose = true`) needs the opposite:
/// `?choose=1` is the SaaS's own affordance for forcing the hub list even when a lone hub would
/// auto-redirect — without it, the owner with two businesses and one tablet bounces right back
/// into the hub they were trying to leave.
pub(crate) fn forget_destination(base: &str, choose: bool) -> String {
    if choose {
        format!("{}?choose=1", onboarding_url(base))
    } else {
        onboarding_url(base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{tempdir, url};

    // ── Forgetting BY CHOICE lands on the chooser, forgetting on a 410 does not ──────────────
    //
    // `/shell/` redirects a single-hub user straight back into their hub — correct after a 410
    // (the hub is gone, the SaaS will route somewhere sane), useless for «I want to pick»: the
    // owner with two businesses and one tablet would bounce right back into the one they were
    // trying to leave. `?choose=1` is the SaaS's own affordance for forcing the list.

    #[test]
    fn forgetting_by_choice_lands_on_the_chooser() {
        assert_eq!(
            forget_destination("https://erplora.com", true),
            "https://erplora.com/shell/?choose=1"
        );
    }

    #[test]
    fn forgetting_on_a_410_keeps_the_plain_onboarding() {
        // The 410 path sends no `choose`: with the hub gone there is nothing to pick between,
        // and the plain onboarding lets the SaaS route (or re-onboard) as it sees fit.
        assert_eq!(
            forget_destination("https://erplora.com", false),
            "https://erplora.com/shell/"
        );
    }

    // ── Which hub the window starts linked to ───────────────────────

    #[test]
    fn a_fresh_install_starts_linked_to_nothing_and_lets_the_saas_choose() {
        let (entries, linked) = boot_link(None, None, "https://erplora.com");
        assert_eq!(entries, vec!["https://erplora.com".to_string()]);
        assert_eq!(linked, None);
    }

    #[test]
    fn the_remembered_hub_is_the_linked_one_at_boot() {
        let (_, linked) = boot_link(None, Some("https://panaderia.a.erplora.com"), "https://erplora.com");
        assert_eq!(linked.as_deref(), Some("https://panaderia.a.erplora.com"));
    }

    // ── A link to another hub waits for the person's word ───────────────────────────────────────

    #[test]
    fn a_cold_start_by_a_link_to_another_hub_boots_on_the_linked_one_and_asks() {
        let (_, linked) = boot_link(None, Some("https://panaderia.a.erplora.com"), "https://erplora.com");
        assert_eq!(linked.as_deref(), Some("https://panaderia.a.erplora.com"));
        assert_eq!(
            cold_start_link(Some("https://otronegocio.a.erplora.com/?shell=1"), linked.as_deref()),
            ColdStartLink::Ask("https://otronegocio.a.erplora.com/?shell=1".into())
        );
    }

    #[test]
    fn a_cold_start_by_a_link_to_the_linked_hub_boots_on_it_without_asking() {
        assert_eq!(
            cold_start_link(
                Some("https://panaderia.a.erplora.com/?shell=1"),
                Some("https://panaderia.a.erplora.com")
            ),
            ColdStartLink::Open("https://panaderia.a.erplora.com/?shell=1".into())
        );
    }

    #[test]
    fn a_cold_start_by_a_link_on_a_device_linked_to_nothing_asks() {
        assert_eq!(
            cold_start_link(Some("https://otronegocio.a.erplora.com/?shell=1"), None),
            ColdStartLink::Ask("https://otronegocio.a.erplora.com/?shell=1".into())
        );
        assert_eq!(cold_start_link(None, Some("https://panaderia.a.erplora.com")), ColdStartLink::None);
    }

    #[test]
    fn the_development_override_links_a_local_hub_but_not_a_local_saas() {
        let (entries, linked) =
            boot_link(Some("http://127.0.0.1:5173/"), None, "https://erplora.com");
        assert_eq!(linked.as_deref(), Some("http://127.0.0.1:5173"));
        assert!(entries.contains(&"http://127.0.0.1:5173".to_string()));
        let (entries, linked) = boot_link(
            Some("http://127.0.0.1:8001/shell/"),
            Some("https://panaderia.a.erplora.com"),
            "http://127.0.0.1:8001",
        );
        assert_eq!(linked, None, "the local SaaS became the linked hub");
        assert_eq!(entries, vec!["http://127.0.0.1:8001".to_string()]);
    }

    // ── shell_capture_origin: the contract of the ?shell=1 marker ────────────────────────────

    #[test]
    fn capture_requires_the_shell_1_marker() {
        assert_eq!(shell_capture_origin(&url("https://demo.erplora.com/")), None);
        assert_eq!(
            shell_capture_origin(&url("https://demo.erplora.com/?shell=2")),
            None
        );
        assert_eq!(
            shell_capture_origin(&url("https://demo.erplora.com/?other=1")),
            None
        );
    }

    #[test]
    fn capture_returns_the_https_origin() {
        assert_eq!(
            shell_capture_origin(&url("https://demo.erplora.com/pos?shell=1&x=2")),
            Some("https://demo.erplora.com".to_string())
        );
    }

    #[test]
    fn capture_accepts_http_only_on_loopback() {
        // Remote http is NOT captured (a deployed hub is always https).
        assert_eq!(shell_capture_origin(&url("http://evil.com/?shell=1")), None);
        // Loopback is (local development: runtime :8787 / Vite :5173).
        assert_eq!(
            shell_capture_origin(&url("http://127.0.0.1:8787/?shell=1")),
            Some("http://127.0.0.1:8787".to_string())
        );
        assert_eq!(
            shell_capture_origin(&url("http://localhost:5173/?shell=1")),
            Some("http://localhost:5173".to_string())
        );
    }

    #[test]
    fn capture_refuses_non_http_schemes() {
        assert_eq!(shell_capture_origin(&url("tauri://localhost/?shell=1")), None);
        assert_eq!(shell_capture_origin(&url("file:///tmp/x?shell=1")), None);
    }

    // ── The marker says "remember me"; it does not say WHO may ask ────────────────────────────
    //
    // If `?shell=1` were the whole capture contract, ANY https origin that carried it would become
    // the origin this installation boots at, for good. The window still loads pages that are not a
    // hub: the SaaS, Google's sign-in and, on the desktop, any foreign page (on Android a foreign
    // page opens in the system browser instead). So one link would be enough: an open redirect on
    // the SaaS, an injection into a third-party page the same window loads, or simply a link a user
    // taps. From then on the till would open full-screen, chrome-less and titled "ERPlora" on
    // somebody else's page, and the operator would type the hub password into it.
    //
    // The destination rule is therefore the SAME one the deep link applies and the same one
    // `capabilities/*.json` declares: only a hub of ours, or the loopback of this machine. The
    // three must agree — an origin the capabilities refuse is a till that looks fine and cannot
    // print, which is the failure `remote_acl.rs` exists to prevent.

    #[test]
    fn capture_refuses_an_origin_that_is_not_a_hub_of_ours() {
        // A captured origin keeps a non-default port, but a stranger's domain with one is still
        // not this till's hub.
        assert_eq!(
            shell_capture_origin(&url("https://hub.example.com:8443/?shell=1")),
            None
        );
        assert_eq!(shell_capture_origin(&url("https://evil.com/?shell=1")), None);
    }

    #[test]
    fn capture_refuses_the_saas_apex() {
        // `erplora.com` is where the app BOOTS, not a hub — and it is deliberately kept out of the
        // hardware capability. Remembering it would pin the till to the marketing site
        // on every cold start, with the onboarding one redirect further away than before.
        assert_eq!(shell_capture_origin(&url("https://erplora.com/?shell=1")), None);
        assert_eq!(
            shell_capture_origin(&url("https://erplora.com/shell/?shell=1")),
            None
        );
    }

    #[test]
    fn capture_refuses_the_domains_that_only_look_like_ours() {
        // Same three shapes `remote_acl.rs` rejects at the ACL, checked here at the capture: a
        // boundary that only holds in one of the two places holds nowhere.
        assert_eq!(
            shell_capture_origin(&url("https://demo.a.erplora.com.attacker.com/?shell=1")),
            None
        );
        assert_eq!(shell_capture_origin(&url("https://myerplora.com/?shell=1")), None);
        // Reads as ours to a human; the host is `evil.com`.
        assert_eq!(
            shell_capture_origin(&url("https://demo.a.erplora.com@evil.com/?shell=1")),
            None
        );
    }

    #[test]
    fn capture_accepts_every_shape_a_real_cloud_hub_takes() {
        // A lettered aura and a numbered aura — TWO labels under the registrable domain. A rule
        // that only allowed one would kill the app in production.
        assert_eq!(
            shell_capture_origin(&url("https://panaderia.a.erplora.com/pos?shell=1&x=2")),
            Some("https://panaderia.a.erplora.com".to_string())
        );
        assert_eq!(
            shell_capture_origin(&url("https://panaderia.3.erplora.com/?shell=1")),
            Some("https://panaderia.3.erplora.com".to_string())
        );
    }

    #[test]
    fn capture_accepts_loopback_over_tls_too() {
        // A dev PWA served over TLS (`vite --https`) is the same machine as the plain-http one, so
        // the loopback exemption belongs to the HOST, not to the scheme.
        assert_eq!(
            shell_capture_origin(&url("https://127.0.0.1:5173/?shell=1")),
            Some("https://127.0.0.1:5173".to_string())
        );
    }

    // ── An install poisoned by an older build must not stay poisoned ──────────────────────────

    #[test]
    fn a_persisted_origin_that_is_not_a_hub_of_ours_is_refused_on_load() {
        // Refusing at the capture only protects installs that have not been captured yet. Every
        // till that already ran a build without the check carries the foreign origin in
        // `hub.url`, and an upgrade that keeps reading it blindly fixes nothing where it matters.
        // `None` sends the window back to the onboarding, which is the one screen it can leave.
        let dir = tempdir();
        std::fs::write(dir.join(HUB_URL_FILE), "https://evil.com").expect("write");
        assert_eq!(load_hub_url(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_persisted_value_that_is_not_even_a_url_is_refused_on_load() {
        // `hub.url` is a plain file in the user's data dir: nothing stops it being edited, or
        // truncated by a crash. Without the check `initial_url_for` would hand whatever it said
        // to the webview.
        let dir = tempdir();
        std::fs::write(dir.join(HUB_URL_FILE), "not a url").expect("write");
        assert_eq!(load_hub_url(&dir), None);
        std::fs::write(dir.join(HUB_URL_FILE), "javascript:alert(1)").expect("write");
        assert_eq!(load_hub_url(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_persisted_hub_loads_reduced_to_its_ORIGIN() {
        // Whatever ends up in the file, what boots is the origin: a stored path or query would
        // otherwise be replayed on every cold start.
        let dir = tempdir();
        std::fs::write(dir.join(HUB_URL_FILE), "https://demo.a.erplora.com/pos?x=1\n").expect("write");
        assert_eq!(load_hub_url(&dir), Some("https://demo.a.erplora.com".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── A user with SEVERAL hubs ──────────────────────────────────────────────────────────────

    #[test]
    fn capturing_a_second_hub_replaces_the_first() {
        // How switching hubs works, and the reason it needs no extra machinery: `/shell/?choose=1`
        // lists the user's hubs and 302s into the chosen one carrying the marker, so the capture
        // that persists hub B is the same one that persisted hub A. Were the first capture sticky,
        // a till moved to another business would keep opening the old one.
        let dir = tempdir();
        persist_hub_url(&dir, "https://a.a.erplora.com").expect("first");
        persist_hub_url(&dir, "https://b.a.erplora.com").expect("second");
        assert_eq!(load_hub_url(&dir), Some("https://b.a.erplora.com".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Persisting hub.url ───────────────────────────────────────────────────────────────────

    #[test]
    fn hub_url_roundtrip_persist_load_clear() {
        let dir = tempdir();
        assert_eq!(load_hub_url(&dir), None);
        persist_hub_url(&dir, "https://demo.erplora.com").expect("persist");
        assert_eq!(load_hub_url(&dir), Some("https://demo.erplora.com".to_string()));
        clear_hub_url(&dir);
        assert_eq!(load_hub_url(&dir), None);
        clear_hub_url(&dir); // best effort: clearing again does not fail
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_hub_url_ignores_blank_files_and_trims() {
        let dir = tempdir();
        std::fs::write(dir.join(HUB_URL_FILE), "  \n").expect("write");
        assert_eq!(load_hub_url(&dir), None);
        std::fs::write(dir.join(HUB_URL_FILE), "https://a.erplora.com\n").expect("write");
        assert_eq!(load_hub_url(&dir), Some("https://a.erplora.com".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persist_hub_url_creates_the_directory() {
        let dir = tempdir().join("anidado");
        persist_hub_url(&dir, "https://b.erplora.com").expect("persist");
        assert_eq!(load_hub_url(&dir), Some("https://b.erplora.com".to_string()));
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    // ── Start URL: precedence dev override → persisted hub → onboarding ──────────────────────

    #[test]
    fn initial_url_precedence() {
        // The development override wins over the persisted hub.
        assert_eq!(
            initial_url_for(
                None,
                Some("http://127.0.0.1:8001/shell/"),
                Some("https://demo.erplora.com"),
                "https://erplora.com"
            ),
            "http://127.0.0.1:8001/shell/"
        );
        // Persisted hub → app mode (origin + "/").
        assert_eq!(
            initial_url_for(None, None, Some("https://demo.erplora.com"), "https://erplora.com"),
            "https://demo.erplora.com/"
        );
        // Nothing persisted → the SaaS onboarding.
        assert_eq!(
            initial_url_for(None, None, None, "https://erplora.com"),
            "https://erplora.com/shell/"
        );
    }

    #[test]
    fn a_cold_start_through_a_link_opens_the_hub_the_link_named() {
        // Windows and Linux answer a link by launching the executable with the URL appended, so at
        // a COLD start the link is all there is. If the persisted hub won, clicking "Open Terminal"
        // for hub B from a till that remembers hub A would quietly open A — the wrong business,
        // and with no error to notice it by.
        assert_eq!(
            initial_url_for(
                Some("https://demo.b.erplora.com/?shell=1"),
                None,
                Some("https://demo.a.erplora.com"),
                "https://erplora.com"
            ),
            "https://demo.b.erplora.com/?shell=1"
        );
        // It also beats the dev override: the override is static configuration, the link is what
        // the user is asking for right now.
        assert_eq!(
            initial_url_for(
                Some("https://demo.b.erplora.com/?shell=1"),
                Some("http://127.0.0.1:8001/shell/"),
                None,
                "https://erplora.com"
            ),
            "https://demo.b.erplora.com/?shell=1"
        );
    }

    // ── The remembered hub no longer exists: the app must NOT be walled in ───────────────────

    #[test]
    fn a_deleted_hub_is_forgotten_at_start() {
        // Seen on a Mac: `hub.url` pointed at a hub a production purge had deleted, and the app
        // opened on «404 page not found» — for good and with NO way out. The existing `forget_hub`
        // does not help here: the frontend calls it on a 410, and in this case the PWA never loads
        // at all because the 404 is served by the edge.
        assert!(should_forget_hub(HubProbe::Status(404)));
        // 410 is the server's explicit `hub_not_found` contract.
        assert!(should_forget_hub(HubProbe::Status(410)));
    }

    #[test]
    fn a_live_hub_is_not_forgotten() {
        assert!(!should_forget_hub(HubProbe::Status(200)));
        // The hub redirects to the login: it exists.
        assert!(!should_forget_hub(HubProbe::Status(302)));
    }

    #[test]
    fn being_unauthenticated_does_not_mean_the_hub_is_gone() {
        // Confusing the two would throw the user back to the onboarding every time a session
        // expires.
        assert!(!should_forget_hub(HubProbe::Status(401)));
        assert!(!should_forget_hub(HubProbe::Status(403)));
    }

    #[test]
    fn a_hub_that_is_down_is_not_forgotten() {
        // Down ≠ gone. Forgetting it over a 30-second outage would delete the user's hub and send
        // them through the onboarding again, which is FAR worse than waiting.
        for s in [500, 502, 503, 504] {
            assert!(!should_forget_hub(HubProbe::Status(s)), "{s} must not forget the hub");
        }
    }

    #[test]
    fn no_network_forgets_nothing() {
        // A till starts in venues with bad Wi-Fi, with the router rebooting or with the laptop not
        // yet associated. Deleting the hub for not reaching it would be catastrophic: the hub is
        // perfectly alive and the user would end up on the onboarding without knowing why.
        assert!(!should_forget_hub(HubProbe::Unreachable));
    }

    #[test]
    fn onboarding_url_normalizes_the_base() {
        assert_eq!(onboarding_url("https://erplora.com"), "https://erplora.com/shell/");
        assert_eq!(
            initial_url_for(None, None, None, "https://erplora.com/"),
            "https://erplora.com/shell/"
        );
        assert_eq!(normalize_base("  https://erplora.com/  "), "https://erplora.com");
    }
}
