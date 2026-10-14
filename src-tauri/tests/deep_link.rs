//! The `erplora://` deep link and its trust boundary (ADR-0196 §7, hub#345).
//!
//! A link is the only thing that lets a page **outside** the app steer the app. That makes it a
//! trust boundary, not a convenience: whoever writes the link picks the destination, and the
//! destination is a window that can open the cash drawer (ADR-0221). So the app does not open what
//! the link says — it **resolves** it, and only a hub of ours resolves at all.
//!
//! The other half is registration. A deep link that no operating system knows about is a dead
//! link, and the failure is silent: the browser simply does nothing and the user stares at a page
//! that did not react. Each platform reads the declaration from a different file, so each file is
//! asserted here.

use erplora_tauri_lib::{deep_link_from_args, resolve_deep_link, DEEP_LINK_SCHEME};

/// Convenience: the destination the app must navigate to for a hub host.
fn hub(host: &str) -> Option<String> {
    Some(format!("https://{host}/?shell=1"))
}

// ── The link opens the hub the SaaS pointed at ───────────────────────────────────────────────────

#[test]
fn a_hub_link_resolves_to_that_hub() {
    assert_eq!(
        resolve_deep_link("erplora://hub/demo.a.erplora.com"),
        hub("demo.a.erplora.com")
    );
}

#[test]
fn the_resolved_url_carries_the_capture_marker() {
    // `?shell=1` is the existing capture contract (ADR-0159): navigating with it is what makes the
    // shell remember this hub as `hub.url`. Without it, opening the app through a link would work
    // once and be forgotten on the next cold start.
    let opened = resolve_deep_link("erplora://hub/demo.a.erplora.com").expect("a hub link resolves");
    assert!(opened.ends_with("/?shell=1"), "{opened}");
}

#[test]
fn a_trailing_slash_is_the_same_link() {
    assert_eq!(
        resolve_deep_link("erplora://hub/demo.a.erplora.com/"),
        hub("demo.a.erplora.com")
    );
}

#[test]
fn the_host_is_matched_case_insensitively() {
    // DNS is case insensitive and mail clients love to capitalise. Rejecting this would turn a
    // perfectly valid link into a dead one.
    assert_eq!(
        resolve_deep_link("ERPLORA://HUB/DEMO.A.ERPLORA.COM"),
        hub("demo.a.erplora.com")
    );
}

#[test]
fn any_aura_is_a_hub() {
    // Hubs live at `{slug}.{aura}.erplora.com`, with auras by letter on Hetzner and by number on
    // the AWS fallback. Enumerating auras would turn "open a new aura" into "ship a new app".
    for host in [
        "demo.a.erplora.com",
        "demo.b.erplora.com",
        "demo.1.erplora.com",
        "my-salon.a.erplora.com",
        "single-label.erplora.com",
    ] {
        assert_eq!(
            resolve_deep_link(&format!("erplora://hub/{host}")),
            hub(host),
            "{host} is a hub of ours"
        );
    }
}

#[test]
fn development_hubs_on_loopback_still_resolve() {
    // The dev hub is plain http on loopback, which no third party can steer.
    assert_eq!(
        resolve_deep_link("erplora://hub/127.0.0.1:8787"),
        Some("http://127.0.0.1:8787/?shell=1".to_string())
    );
    assert_eq!(
        resolve_deep_link("erplora://hub/localhost:5173"),
        Some("http://localhost:5173/?shell=1".to_string())
    );
}

// ── Trust boundary: a tampered link must not reach a foreign destination ─────────────────────────

#[test]
fn a_tampered_link_never_leaves_our_domain() {
    // Every one of these is a link an attacker can put in an email. If any of them resolved, the
    // app would render an attacker's page in the window that owns the hardware handlers.
    for hostile in [
        "erplora://hub/evil.com",
        "erplora://hub/demo.a.erplora.com.evil.com",
        "erplora://hub/erplora.com.evil.com",
        "erplora://hub/evil-erplora.com",
        "erplora://hub/evil.com/demo.a.erplora.com",
        "erplora://hub/demo.a.erplora.com/../../evil.com",
        "erplora://hub@evil.com/demo.a.erplora.com",
        "erplora://hub/demo.a.erplora.com@evil.com",
        "erplora://hub/evil.com#demo.a.erplora.com",
        "erplora://hub/demo.a.erplora.com.",
        "erplora://hub/.erplora.com",
        "erplora://hub//evil.com",
    ] {
        assert_eq!(resolve_deep_link(hostile), None, "must not resolve: {hostile}");
    }
}

#[test]
fn the_saas_apex_is_not_a_hub() {
    // `erplora.com` also serves marketing, billing and a third-party checkout — the widest surface
    // the window ever loads, and deliberately kept out of the hardware capability (ADR-0221). A
    // link must not be able to steer the app there either.
    assert_eq!(resolve_deep_link("erplora://hub/erplora.com"), None);
    assert_eq!(resolve_deep_link("erplora://hub/www.erplora.com/"), hub("www.erplora.com"));
}

#[test]
fn a_query_cannot_redirect_the_destination() {
    // The destination is built from the host alone; anything else in the link is ignored, so no
    // parameter can smuggle a second URL in.
    assert_eq!(
        resolve_deep_link("erplora://hub/demo.a.erplora.com?next=https://evil.com"),
        hub("demo.a.erplora.com")
    );
    assert_eq!(
        resolve_deep_link("erplora://hub/demo.a.erplora.com#https://evil.com"),
        hub("demo.a.erplora.com")
    );
}

#[test]
fn a_link_the_app_does_not_understand_is_left_to_the_fallback() {
    // The grammar is `erplora://hub/<host>` and nothing else. If a later version grows a page
    // (`erplora://hub/<host>/pos/table/4`), an app that predates it must NOT half-understand the
    // link and open the hub home: that silently drops the page the user asked for, with no way to
    // tell it went wrong. Refusing hands the click back to the browser fallback, which knows the
    // full https URL and lands on the right page.
    assert_eq!(resolve_deep_link("erplora://hub/demo.a.erplora.com/pos"), None);
    assert_eq!(resolve_deep_link("erplora://hub/demo.a.erplora.com/pos/table/4"), None);
}

#[test]
fn a_percent_encoded_host_is_refused() {
    // Decoding would re-open every check above through the back door, so the host is read raw and
    // anything that is not a plain DNS label is refused.
    assert_eq!(resolve_deep_link("erplora://hub/demo%2Ea%2Eerplora%2Ecom"), None);
    assert_eq!(resolve_deep_link("erplora://hub/evil.com%2F%40demo.a.erplora.com"), None);
}

#[test]
fn a_homograph_host_is_refused() {
    // `а` here is Cyrillic. It is not the aura `a`, and it must not be allowed to look like it.
    assert_eq!(resolve_deep_link("erplora://hub/demo.а.erplora.com"), None);
}

#[test]
fn a_foreign_scheme_is_not_our_link() {
    for foreign in [
        "https://demo.a.erplora.com/",
        "erplora-bridge://hub/demo.a.erplora.com",
        "javascript:alert(1)",
        "file:///etc/passwd",
        "erplora",
        "",
        "   ",
    ] {
        assert_eq!(resolve_deep_link(foreign), None, "must not resolve: {foreign}");
    }
}

#[test]
fn an_unknown_action_is_refused() {
    // The grammar has exactly one action today. Anything else is either a typo or someone probing,
    // and guessing what it meant is how open redirects are born.
    for unknown in [
        "erplora://open/demo.a.erplora.com",
        "erplora://hubs/demo.a.erplora.com",
        "erplora:///demo.a.erplora.com",
        "erplora://demo.a.erplora.com/",
    ] {
        assert_eq!(resolve_deep_link(unknown), None, "must not resolve: {unknown}");
    }
}

#[test]
fn credentials_or_a_port_on_the_action_are_refused() {
    assert_eq!(resolve_deep_link("erplora://user@hub/demo.a.erplora.com"), None);
    assert_eq!(resolve_deep_link("erplora://user:pw@hub/demo.a.erplora.com"), None);
    assert_eq!(resolve_deep_link("erplora://hub:8080/demo.a.erplora.com"), None);
}

#[test]
fn a_loopback_target_must_be_a_plain_port() {
    for hostile in [
        "erplora://hub/127.0.0.1:8787@evil.com",
        "erplora://hub/127.0.0.1:",
        "erplora://hub/127.0.0.1",
        "erplora://hub/evil.com:8787",
        "erplora://hub/localhost.evil.com:5173",
    ] {
        assert_eq!(resolve_deep_link(hostile), None, "must not resolve: {hostile}");
    }
}

#[test]
fn a_malformed_label_is_refused() {
    for hostile in [
        "erplora://hub/-demo.a.erplora.com",
        "erplora://hub/demo-.a.erplora.com",
        "erplora://hub/demo_a.a.erplora.com",
        "erplora://hub/demo..erplora.com",
        "erplora://hub/demo a.erplora.com",
    ] {
        assert_eq!(resolve_deep_link(hostile), None, "must not resolve: {hostile}");
    }
}

// ── Cold start: the link arrives as an argument (Windows/Linux) ──────────────────────────────────

#[test]
fn a_cold_start_opens_the_hub_the_link_named() {
    // On Windows and Linux the OS launches the executable with the URL as an argument. If it were
    // ignored, clicking the link would open the app on the *previous* hub — the wrong one, and
    // silently so.
    let args = [
        "/Applications/ERPlora.app/Contents/MacOS/ERPlora".to_string(),
        "erplora://hub/demo.a.erplora.com".to_string(),
    ];
    assert_eq!(
        deep_link_from_args(args),
        Some("https://demo.a.erplora.com/?shell=1".to_string())
    );
}

#[test]
fn a_normal_launch_carries_no_link() {
    assert_eq!(deep_link_from_args(["erplora.exe".to_string()]), None);
    assert_eq!(
        deep_link_from_args(["erplora.exe".to_string(), "--flag".to_string()]),
        None
    );
}

#[test]
fn a_hostile_argument_does_not_become_a_destination() {
    // The argument list is attacker-reachable through the OS handler: whatever the browser passes
    // lands here verbatim.
    assert_eq!(
        deep_link_from_args(["erplora.exe".to_string(), "erplora://hub/evil.com".to_string()]),
        None
    );
    assert_eq!(
        deep_link_from_args(["erplora.exe".to_string(), "https://evil.com".to_string()]),
        None
    );
}

// ── Registration: a scheme nobody registered is a dead link ──────────────────────────────────────

const TAURI_CONF: &str = include_str!("../tauri.conf.json");
const ANDROID_MANIFEST: &str = include_str!("../gen/android/app/src/main/AndroidManifest.xml");

#[test]
fn the_desktop_bundle_registers_the_scheme() {
    // Windows and Linux only learn about the scheme at INSTALL time, from what the bundler writes
    // (registry key / `.desktop` handler). The bundler reads it from here, so an undeclared scheme
    // means the link does nothing on the two platforms most tills run on.
    let conf: serde_json::Value = serde_json::from_str(TAURI_CONF).expect("tauri.conf.json parses");
    let schemes = conf
        .pointer("/plugins/deep-link/desktop/schemes")
        .and_then(|v| v.as_array())
        .expect("plugins.deep-link.desktop.schemes must be declared");
    assert!(
        schemes.iter().any(|s| s.as_str() == Some(DEEP_LINK_SCHEME)),
        "the desktop bundle does not register {DEEP_LINK_SCHEME}://: {schemes:?}"
    );
}

#[test]
fn android_registers_the_scheme_as_a_browsable_intent() {
    // Android routes a link only to an activity with a BROWSABLE intent filter for the scheme.
    // Without the three pieces together the intent is never delivered and nothing reports it.
    assert!(
        ANDROID_MANIFEST.contains(&format!("android:scheme=\"{DEEP_LINK_SCHEME}\"")),
        "the launcher activity does not declare the {DEEP_LINK_SCHEME} scheme"
    );
    assert!(
        ANDROID_MANIFEST.contains("android.intent.category.BROWSABLE"),
        "without BROWSABLE the system will not hand a link to the app"
    );
    assert!(
        ANDROID_MANIFEST.contains("android.intent.action.VIEW"),
        "without the VIEW action the intent filter matches nothing"
    );
    // `singleTask` is what makes a second link reuse the running app instead of stacking a new
    // instance on top of the till.
    assert!(ANDROID_MANIFEST.contains("android:launchMode=\"singleTask\""));
}

#[test]
fn android_declares_the_scheme_where_the_plugin_filters_it_too() {
    // The intent filter gets the URL as far as the plugin, and the plugin drops it there. Read in
    // tauri-plugin-deep-link 2.4.9 (`DeepLinkPlugin.kt`): `isDeepLink` returns FALSE outright when
    // `plugins.deep-link.mobile` is empty, and otherwise only matches a configured scheme/host. So
    // `desktop.schemes` alone gets Android to hand us the intent and then throws it away —
    // silently, and only on Android, which is the hardest place to notice it.
    let conf: serde_json::Value = serde_json::from_str(TAURI_CONF).expect("tauri.conf.json parses");
    let mobile = conf
        .pointer("/plugins/deep-link/mobile")
        .and_then(|v| v.as_array())
        .expect("plugins.deep-link.mobile must be declared or Android drops every link");
    let matches_our_links = mobile.iter().any(|entry| {
        let schemes = entry.pointer("/scheme").and_then(|v| v.as_array());
        let declares_scheme = schemes.is_some_and(|s| {
            s.iter()
                .any(|v| v.as_str().is_some_and(|s| s.eq_ignore_ascii_case(DEEP_LINK_SCHEME)))
        });
        // The host is the ACTION of the grammar (`erplora://hub/…`), so an entry that names a
        // different one would filter out exactly the links we issue.
        let host_fits = match entry.pointer("/host").and_then(|v| v.as_str()) {
            Some(host) => host.eq_ignore_ascii_case("hub"),
            None => true,
        };
        declares_scheme && host_fits
    });
    assert!(
        matches_our_links,
        "no entry in plugins.deep-link.mobile matches {DEEP_LINK_SCHEME}://hub/… : {mobile:?}"
    );
}
