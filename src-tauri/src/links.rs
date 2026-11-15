//! Deep link `erplora://`, and the way out to the user's own browser.
//!
//! A link is the one thing that lets a page OUTSIDE the app steer the app, so it is a trust
//! boundary and not a convenience: whoever writes the link picks the destination, and the
//! destination is the window that owns the hardware handlers. The app therefore never opens what
//! the link says — it RESOLVES it, and only a hub of ours resolves at all. Anything else returns
//! `None`, and `None` means "leave the window where it is": the browser that issued the link is the
//! one that owns the fallback.

use crate::{ShellError, HUB_DOMAIN_SUFFIX};

/// Scheme of the installable app's deep link, registered against the single app identity
/// `com.erplora.app`.
pub const DEEP_LINK_SCHEME: &str = "erplora";

/// The only action of the grammar: `erplora://hub/<host>`.
const DEEP_LINK_HUB_ACTION: &str = "hub";

/// The apex itself — the SaaS: marketing, the dashboard, billing and the module checkout. It is
/// [`HUB_DOMAIN_SUFFIX`] without the leading dot that makes that one a suffix match, and it is
/// deliberately NOT a hub: only [`external_browser_url`] accepts it.
const ERPLORA_DOMAIN: &str = "erplora.com";

/// Stripe's hosted checkout, matched by EXACT host. The assistant's paid upgrade is its own Stripe
/// subscription and the SaaS answers it with this page directly — there is no erplora.com page in
/// between to send the browser to. Without it the assistant's «See plans» could only fail inside
/// the installed app. No other Stripe host, and no suffix match: the dashboard or a look-alike is
/// not a checkout the till starts.
const STRIPE_CHECKOUT_HOST: &str = "checkout.stripe.com";

/// The two sites of the AEAT's QR check: the real one and its test one (`QR_HOSTS` of
/// `apps/verifactu/chain.py` in the Hub). The Hub links a record to the site it was born on.
const AEAT_QR_HOSTS: [&str; 2] = ["www2.agenciatributaria.gob.es", "prewww2.aeat.es"];

/// The page that checks a VeriFactu QR on [`AEAT_QR_HOSTS`] (`QR_PATH` in the same Hub file).
const AEAT_QR_PATH: &str = "/wlpl/TIKE-CONT/ValidarQR";

/// Is this the AEAT's QR check (app#20)? Host AND path, unlike [`STRIPE_CHECKOUT_HOST`]: the AEAT
/// serves far more than the check under those two domains, and this door only has to lead to it.
///
/// No port either: `port()` is `None` when the address spells the 443 that https implies, so anything
/// else is another service of that machine. Judged on the parsed URL, where the dots are resolved and
/// percent-escapes stay as written, so `…/ValidarQR/../Otra` and `…/ValidarQR%2F..%2FOtra` both miss.
fn is_aeat_qr_check(url: &tauri::Url) -> bool {
    url.host_str().is_some_and(|host| AEAT_QR_HOSTS.contains(&host))
        && url.port().is_none()
        && url.path() == AEAT_QR_PATH
}

/// Is this host a hub of ours — `<label>[.<label>…].erplora.com`?
///
/// The apex has no leading dot to strip, which is exactly why it is excluded: `erplora.com` also
/// serves marketing, billing and a third-party checkout, and is deliberately kept out of the
/// hardware capability. Auras are matched by WILDCARD and never enumerated: hubs live at
/// `{slug}.{aura}.erplora.com`, so a list would turn "open a new aura" into "ship a new app to
/// every till".
///
/// Expects an already-lowercased host — for `https` the URL parser guarantees it, and
/// [`hub_url_for_host`] lowercases the opaque host of a deep link before asking.
pub(crate) fn is_hub_domain(host: &str) -> bool {
    match host.strip_suffix(HUB_DOMAIN_SUFFIX) {
        Some(subdomain) => !subdomain.is_empty() && subdomain.split('.').all(is_dns_label),
        None => false,
    }
}

/// Is this host THIS machine? Loopback is the development exemption: nothing outside the machine
/// can serve it, so nobody else can steer a window that points there.
pub(crate) fn is_loopback_host(host: &str) -> bool {
    // `[::1]` with brackets is how `Url::host_str` serializes the IPv6 loopback.
    matches!(host, "127.0.0.1" | "localhost" | "[::1]")
}

/// Is this a plain DNS label? Deliberately narrower than the RFC: lowercase ASCII only, so a
/// percent-escape, a homograph or a stray `@` can never pass for a label.
fn is_dns_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The trust boundary: which destinations the app is willing to open for a link.
///
/// Mirrors `remote.urls` of the hardware capability (`https://*.erplora.com/*` plus loopback for
/// development) on purpose. A destination the capabilities would not trust must not be reachable
/// through a link either — and the reverse would be just as bad: opening a page that cannot
/// `invoke` leaves a till that looks fine and cannot print.
///
/// The production half of the rule lives in [`is_hub_domain`], shared with what the shell is
/// willing to REMEMBER ([`trusted_hub_origin`]): one predicate, so the two cannot drift apart.
///
/// [`trusted_hub_origin`]: crate::hub_url::trusted_hub_origin
fn hub_url_for_host(raw_host: &str) -> Option<String> {
    let host = raw_host.trim().to_ascii_lowercase();

    // Development: the hub is plain http on loopback. Not reachable from another machine, so a
    // third party cannot steer it — but the port must be digits and nothing else, or
    // `127.0.0.1:8787@evil.com` would walk straight through.
    for prefix in ["127.0.0.1:", "localhost:"] {
        if let Some(port) = host.strip_prefix(prefix) {
            if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            return Some(format!("http://{host}/?shell=1"));
        }
    }

    // Production: the same rule the capture applies to what it is willing to remember.
    if !is_hub_domain(&host) {
        return None;
    }
    Some(format!("https://{host}/?shell=1"))
}

/// Resolves a deep link into the URL the window must navigate to. `None` = not a link of ours, or
/// not a destination of ours → **do not navigate**.
///
/// The destination is rebuilt from the host alone, so query and fragment are ignored by
/// construction and no parameter can smuggle a second URL in. The `?shell=1` marker is the capture
/// contract: navigating with it is what makes the shell remember this hub as `hub.url`, so a link
/// opens the app AND leaves it pointing at the right hub next time.
pub fn resolve_deep_link(raw: &str) -> Option<String> {
    let url: tauri::Url = raw.trim().parse().ok()?;
    if url.scheme() != DEEP_LINK_SCHEME {
        return None;
    }
    // Case-insensitive on purpose: for a non-special scheme the host is an OPAQUE host, so the URL
    // parser does NOT lowercase it the way it would for `https`. A link typed or reflowed in
    // uppercase by a mail client is still the same link.
    if !url.host_str()?.eq_ignore_ascii_case(DEEP_LINK_HUB_ACTION) {
        return None;
    }
    // Credentials or a port on the action are not part of the grammar. They carry no meaning here,
    // so accepting them would only add shapes of the same link that have to be reasoned about.
    if !url.username().is_empty() || url.password().is_some() || url.port().is_some() {
        return None;
    }
    let mut segments = url.path_segments()?;
    let target = segments.next()?;
    // Exactly one segment. A single empty tail is the trailing slash of `…/erplora.com/`.
    if segments.any(|s| !s.is_empty()) {
        return None;
    }
    hub_url_for_host(target)
}

/// Picks the deep link out of the process arguments — the cold start path on Windows and Linux,
/// where the OS launches the executable with the URL appended instead of delivering an event.
///
/// Every argument goes through [`resolve_deep_link`], so a hostile one is refused here too: this
/// list is attacker-reachable, since whatever the browser hands the registered handler lands in it
/// verbatim.
pub fn deep_link_from_args<I: IntoIterator<Item = String>>(args: I) -> Option<String> {
    args.into_iter().find_map(|arg| resolve_deep_link(&arg))
}

// ── The way OUT: the user's own browser ──────────────────────────────────────────────────────────

/// The address `openExternal` may hand to the SYSTEM browser, normalized — or `None`.
///
/// The web app has one door out of the till, and behind it are the buttons that CHARGE: the module
/// checkout, which lives on the SaaS website *because Google Play does not allow paying for digital
/// goods inside the app*, the plans page, the billing portal, the plan-limit upsell. In the browser
/// that door is a new tab; inside the installed app `window.open` opens nothing at all — no plugin
/// is exposed to the page and the webview spawns no window.
///
/// Letting the page ask for a browser is a frontier of its own: an unchecked opener turns the till
/// into a launcher for any address the page names — a phishing page wearing the trust of an
/// installed app, a `file://` path handed to whatever the desktop associates with it, or a custom
/// scheme that starts another program. So the destination is checked here, just as the hardware
/// capability checks who may drive the hardware and [`resolve_deep_link`] what a link may open.
///
/// **The apex is IN, and that is the difference from the other two boundaries.** [`is_hub_domain`]
/// and [`trusted_hub_origin`] refuse `erplora.com` on purpose, because they answer "may this page
/// open the cash drawer / become this till's home?". This one answers "may the user's browser be
/// sent here?" — and the apex is the SaaS, where those paid destinations live. A visit grants
/// nothing: the page opens in the browser's own sandbox, with no `invoke` and no way back in.
///
/// [`trusted_hub_origin`]: crate::hub_url::trusted_hub_origin
pub fn external_browser_url(raw: &str) -> Option<String> {
    // The WHATWG parser already strips leading and trailing spaces, so `trim` changes no outcome.
    // It stays so that the code reads "trim, then parse" without relying on that clause of the URL
    // spec.
    let url: tauri::Url = raw.trim().parse().ok()?;

    // Credentials are the oldest confusion trick there is, and the dangerous half is the one the
    // host check cannot catch: `https://support%40evil.example@erplora.com/` really is our host,
    // and the browser would show a page nobody at ERPlora wrote under an address that reads like
    // support.
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }

    let host = url.host_str()?;
    let allowed = match url.scheme() {
        // Everything we serve: the SaaS at the apex plus every hub, auras by wildcard.
        // Plus Stripe's hosted checkout, where the assistant's upgrade is paid, and the
        // AEAT's public check of a VeriFactu QR (app#20): only a link, nothing is sent to the AEAT.
        "https" => {
            host == ERPLORA_DOMAIN
                || host == STRIPE_CHECKOUT_HOST
                || is_hub_domain(host)
                || is_aeat_qr_check(&url)
        }
        // Development against a local SaaS (`VITE_CLOUD_API_URL=http://127.0.0.1:8001`). Nothing
        // outside this machine can serve loopback, so nobody else can steer it.
        "http" => is_loopback_host(host),
        // Nothing else. `file:`, `javascript:` and custom schemes are not "a web address the user
        // wanted": they are instructions to the operating system.
        _ => false,
    };

    allowed.then(|| url.to_string())
}

/// `open_external_url` — hands `url` to the user's OWN browser, leaving the till where it is.
///
/// This is what the web app's `openExternal` calls inside the installed app. It is a separate
/// program, not a tab of ours, and that is the point on two counts: the till stays exactly as the
/// user left it while they pay, and the payment happens **outside** the app, which is what lets the
/// Android build ship (Play does not allow digital goods to be paid for inside the app).
///
/// A refusal is returned, never swallowed: the page turns it into something the user can read. A
/// button that does nothing when pressed is the defect this command exists to end.
#[tauri::command]
pub(crate) fn open_external_url(app: tauri::AppHandle, url: String) -> Result<(), ShellError> {
    use tauri_plugin_opener::OpenerExt;

    let Some(target) = external_browser_url(&url) else {
        log::warn!("shell: refused to open {url} externally — not an address of ours");
        return Err(ShellError::ExternalUrlRefused);
    };
    // `None` for `with`, deliberately: the plugin's `"inAppBrowser"` opens an Android Custom Tab,
    // which is chrome our app hosts. What is wanted here is the browser as its own app.
    app.opener()
        .open_url(target, None::<&str>)
        .map_err(|e| ShellError::ExternalUrlUnavailable(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_store_copy_answers_the_app_s_links() {
        // The `.exe`/`.msi` register the scheme at install; the Store copy only has what its manifest
        // declares. Without it a click on a notice, or any `erplora://` link, reaches nobody there.
        let manifest = include_str!("../msix/Package.appxmanifest");
        let protocol = format!(r#"<uap:Protocol Name="{DEEP_LINK_SCHEME}""#);
        assert!(manifest.contains(r#"<uap:Extension Category="windows.protocol">"#), "no protocol extension");
        assert!(manifest.contains(&protocol), "the Store copy does not declare {DEEP_LINK_SCHEME}://");
    }
}
