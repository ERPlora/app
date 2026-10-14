//! The boundary of `open_external_url`: which addresses the app is willing to hand to the SYSTEM
//! browser (hub#475).
//!
//! The web app has one door out of the till — `openExternal` — and inside the installed app it was
//! a **no-op**: no `shell`/`opener` plugin is exposed to the page and the webview spawns no window,
//! so `window.open` did nothing at all. The eight buttons behind it are the ones that CHARGE: the
//! module checkout ADR-0114 §4 deliberately took out of the app for Google Play, the plans page,
//! the billing portal, the plan-limit upsell.
//!
//! Making that work means the page can ask the app to launch the user's browser, and that is a new
//! frontier: an unchecked opener turns the till into a launcher for any address — a phishing page
//! wearing the OS trust of an installed app, a `file://` path, or a custom scheme that starts
//! another program. So the destination is CHECKED, in Rust, the way ADR-0221 checks who may drive
//! the hardware and ADR-0225 checks what a deep link may open.
//!
//! Note what is deliberately DIFFERENT here: the apex `erplora.com` is **allowed**, while
//! `hub_url_for_host`/`trusted_hub_origin` refuse it on purpose. Those two answer "may this page
//! drive the cash drawer / become this till's home?"; this one answers "may the user's browser be
//! sent here?". The apex is the SaaS — it is where every one of the eight destinations lives — and
//! opening it in a browser grants nothing but a visit.

use erplora_tauri_lib::external_browser_url;

/// The destination `openExternal` builds for a module purchase (ADR-0114 §4).
const CHECKOUT: &str = "https://erplora.com/dashboard/marketplace/modules/pos/?hub=h1&utm_source=hub";

#[test]
fn opens_the_saas_apex_where_the_checkout_lives() {
    assert_eq!(external_browser_url(CHECKOUT).as_deref(), Some(CHECKOUT));
}

#[test]
fn opens_a_hub_of_ours_whatever_its_aura() {
    for raw in [
        "https://demo.a.erplora.com/files/report.pdf",
        "https://demo.1.erplora.com/",
        "https://www.erplora.com/legal/",
    ] {
        assert!(
            external_browser_url(raw).is_some(),
            "{raw} is ours and must be openable"
        );
    }
}

/// The assistant's paid upgrade is its own Stripe subscription (ADR-0033), and the SaaS answers
/// its checkout with Stripe's hosted page — there is no erplora.com page in between. Without this
/// the assistant's «See plans» could only fail inside the installed app, so it navigated the till
/// window itself and left the owner stranded on a payment page with no way back (hub#1914).
#[test]
fn opens_stripe_hosted_checkout_for_the_assistant_upgrade() {
    const STRIPE: &str = "https://checkout.stripe.com/c/pay/cs_live_a1B2c3#fidkdWxOYHwnPyd1blpxYHZxWjA0";
    assert_eq!(external_browser_url(STRIPE).as_deref(), Some(STRIPE));
}

#[test]
fn stripe_is_admitted_by_its_exact_checkout_host_only() {
    for raw in [
        // Other Stripe hosts are not a checkout the till starts.
        "https://dashboard.stripe.com/",
        "https://stripe.com/",
        // Look-alikes of the checkout host.
        "https://checkout.stripe.com.evil.example/c/pay/cs_x",
        "https://evilcheckout.stripe.com/c/pay/cs_x",
        // Downgradeable trip.
        "http://checkout.stripe.com/c/pay/cs_x",
        // Credentials in front of the real host.
        "https://support%40evil.example@checkout.stripe.com/c/pay/cs_x",
    ] {
        assert_eq!(external_browser_url(raw), None, "{raw} must be refused");
    }
}

#[test]
fn opens_the_development_loopback() {
    // `VITE_CLOUD_API_URL=http://127.0.0.1:8001` is how the shell is developed against a local
    // SaaS. Loopback is not reachable from another machine, so nobody outside can steer it.
    assert!(external_browser_url("http://127.0.0.1:8001/dashboard/billing/").is_some());
    assert!(external_browser_url("http://localhost:8001/dashboard/billing/").is_some());
}

#[test]
fn hands_the_browser_a_PARSED_address_not_the_page_s_text() {
    // What reaches the operating system is what the URL parser produced, not the string the page
    // sent. Scheme and host come back lowercased and the path is left exactly as written — a
    // destination is a destination however it was typed, and nothing in between gets to reinterpret
    // the path.
    assert_eq!(
        external_browser_url("HTTPS://ERPLORA.COM/dashboard/Billing/").as_deref(),
        Some("https://erplora.com/dashboard/Billing/")
    );
    // Surrounding whitespace is a copy-paste artefact, not a different address.
    assert_eq!(
        external_browser_url("  https://erplora.com/dashboard/  ").as_deref(),
        Some("https://erplora.com/dashboard/")
    );
}

#[test]
fn refuses_a_destination_that_is_not_ours() {
    for raw in [
        "https://evil.example/dashboard/billing/",
        // Merely starting with our name.
        "https://erplora.com.evil.example/",
        // Merely ending with our label, one character off the suffix.
        "https://notperplora.com/",
        "https://xn--erplra-4va.com/",
    ] {
        assert_eq!(
            external_browser_url(raw),
            None,
            "{raw} must not reach the user's browser through us"
        );
    }
}

#[test]
fn refuses_userinfo_confusion() {
    // The host really is ours here, so only the explicit credential check refuses it. A browser
    // shows `support@erplora.com` in the address bar of a page nobody at ERPlora wrote.
    assert_eq!(
        external_browser_url("https://support%40evil.example@erplora.com/dashboard/"),
        None
    );
    // And the classic the other way round: the host is the attacker's.
    assert_eq!(external_browser_url("https://erplora.com@evil.example/"), None);
}

#[test]
fn refuses_every_scheme_but_https_and_loopback_http() {
    for raw in [
        // A local file handed to the OS opener runs whatever the desktop associates with it.
        "file:///etc/passwd",
        "javascript:alert(1)",
        // Custom schemes start OTHER programs — including our own app, in a loop.
        "erplora://hub/demo.a.erplora.com",
        "ms-settings:privacy",
        "mailto:support@erplora.com",
        // Plain http outside loopback: our own domain, but a downgradeable trip.
        "http://erplora.com/dashboard/",
        "http://demo.a.erplora.com/",
    ] {
        assert_eq!(
            external_browser_url(raw),
            None,
            "{raw} must not be handed to the operating system"
        );
    }
}

#[test]
fn refuses_what_is_not_a_url_at_all() {
    for raw in ["", "   ", "not a url", "//erplora.com/dashboard/"] {
        assert_eq!(external_browser_url(raw), None, "{raw:?} must be refused");
    }
}
