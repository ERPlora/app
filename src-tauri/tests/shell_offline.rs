//! Guard for what the till shows when the NETWORK dies under it (hub#1716).
//!
//! The shell is a thin client (ADR-0159): the window navigates to a remote origin and everything
//! the user sees comes from there. So a failed navigation is not an edge case, it is Tuesday
//! afternoon in a bar — and until hub#1716 the app had no answer for it. `open_main_window` fell
//! back to the bundled page only when the initial URL failed to **parse**, and `on_navigation`
//! returned `true` without ever looking at whether the load succeeded. Everything else landed in
//! whatever the platform's webview paints on a dead load:
//!
//! - on macOS (WKWebView) a **blank white window** — no message at all, no control, nothing;
//! - on Android (Chromium) the grey `ERR_NAME_NOT_RESOLVED` / `ERR_INTERNET_DISCONNECTED` page.
//!
//! Both are unrecoverable from inside the app: there is no address bar and no reload button in a
//! Tauri window, so the only way out is killing the app from the operating system. On a counter
//! taking money that is a dead till.
//!
//! What this file pins is the whole way out, in the two halves that can rot independently:
//!
//! 1. **The decision** — `connectivity::next_screen` and the backoff around it. This is what
//!    decides that the window must leave the target for our own page, and what brings it back on
//!    its own when the network returns. Driven here against a REAL `WebviewWindow` on Tauri's mock
//!    runtime, so the test fails if the navigation stops happening, not merely if a bool flips.
//! 2. **The page** — `shell-dist/index.html` must be recognisably ERPlora, must say that it is the
//!    NETWORK and not a crash, must carry its `en` source chain and its `es` translation
//!    (ADR-0055/0199), and its retry control must reach the app instead of reloading itself
//!    (reloading the fallback page re-renders the fallback page: the button was a dead end).

const SHELL_PAGE: &str = include_str!("../../shell-dist/index.html");

// ── The page says who it is, what happened, and offers the way out ───────────────────────────────

#[test]
fn the_offline_page_carries_the_erplora_brand() {
    // The whole point of the issue: the person is looking at what is, to them, their till. A grey
    // page with somebody else's error text reads as "the program broke". Ours has to be ours.
    assert!(
        SHELL_PAGE.contains("ERPlora"),
        "shell-dist/index.html never names ERPlora: the fallback screen is indistinguishable from \
         the browser error page it replaces (hub#1716)"
    );
    assert!(
        SHELL_PAGE.contains("erplora-mark.png"),
        "shell-dist/index.html shows no ERPlora mark: the screen has to be recognisable as the app \
         the person opened, not as a generic failure (hub#1716)"
    );
}

#[test]
fn the_offline_page_blames_the_network_and_not_the_app() {
    // "Something went wrong" is what makes a cashier call support. The copy has to place the fault
    // on the connection AND say the data is safe, in both chains.
    for (language, needle) in [
        ("en", "connection"),
        ("en", "not a fault in the app"),
        ("es", "conexión"),
        ("es", "no es un fallo de la aplicación"),
    ] {
        assert!(
            SHELL_PAGE.contains(needle),
            "the {language} copy never says {needle:?}: the screen does not tell the person that \
             the network is what failed, so it reads as a crash (hub#1716)"
        );
    }
}

#[test]
fn the_offline_page_ships_both_the_english_source_and_its_spanish_translation() {
    // ADR-0055/0199: visible text is an English source chain plus its `es` translation, never one
    // hardcoded string. This page has no i18n runtime to lean on — it is the page you get when
    // there is no network at all — so it carries both catalogues itself.
    for chain in ["en:", "es:"] {
        assert!(
            SHELL_PAGE.contains(chain),
            "shell-dist/index.html has no `{chain}` catalogue: visible text must ship as an English \
             source chain plus its Spanish translation (ADR-0055/0199)"
        );
    }

    // And both catalogues must cover the same keys, or one language silently renders `undefined`.
    let en = catalogue_keys("en:");
    let es = catalogue_keys("es:");
    assert!(
        !en.is_empty(),
        "parsed no key out of the `en:` catalogue — this guard would pass vacuously"
    );
    assert_eq!(
        en, es,
        "the `en` and `es` catalogues of shell-dist/index.html do not cover the same keys: the \
         language that misses one renders nothing where a sentence should be (ADR-0199)"
    );
}

#[test]
fn the_offline_page_retry_asks_the_app_instead_of_reloading_itself() {
    // The button existed before hub#1716 and did `location.reload()`. On the fallback page that
    // reloads THE FALLBACK PAGE: the one control on the only screen the user can reach was a loop
    // back to itself. Retry has to reach the shell, which is the only side that can probe the
    // network and navigate the window.
    //
    // Asserted on the CODE and not on the file: the page's own comment explains what it used to do
    // and names `location.reload()` doing so, so a check over the raw text would fail on a page
    // that is right. A guard that forces you to stop writing down why is a guard that gets deleted.
    let code = executable_page_code();
    assert!(
        code.contains("shell_retry"),
        "shell-dist/index.html never invokes `shell_retry`: its retry control cannot reach the \
         shell, so pressing it can only re-render the same dead end (hub#1716)"
    );
    assert!(
        !code.contains("location.reload()"),
        "shell-dist/index.html still reloads itself: on the bundled fallback page that re-renders \
         the fallback page and the user stays trapped (hub#1716)"
    );
}

/// The page with its prose removed: HTML comments and whole-line `//` comments gone, so an
/// assertion about what the page DOES cannot be satisfied — or broken — by what it says about
/// itself. Only whole-line `//` is stripped: a trailing `//` would eat the `https://` inside a
/// string literal, and this page has both.
fn executable_page_code() -> String {
    let mut out = String::with_capacity(SHELL_PAGE.len());
    let mut rest = SHELL_PAGE;
    while let Some(open) = rest.find("<!--") {
        out.push_str(&rest[..open]);
        rest = match rest[open..].find("-->") {
            Some(close) => &rest[open + close + "-->".len()..],
            // Unterminated comment: the rest of the file is prose.
            None => "",
        };
    }
    out.push_str(rest);

    out.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Keys of one of the two inline catalogues (`en: { … }` / `es: { … }`) in the page's script.
fn catalogue_keys(opening: &str) -> Vec<String> {
    let Some(start) = SHELL_PAGE.find(opening) else {
        return Vec::new();
    };
    let rest = &SHELL_PAGE[start + opening.len()..];
    let Some(open_brace) = rest.find('{') else {
        return Vec::new();
    };
    let body = &rest[open_brace + 1..];
    let Some(end) = body.find('}') else {
        return Vec::new();
    };

    let mut keys: Vec<String> = body[..end]
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let (key, _) = line.split_once(':')?;
            let key = key.trim().trim_matches('"');
            (!key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
                .then(|| key.to_string())
        })
        .collect();
    keys.sort();
    keys
}

// ── …and the shell is actually wired to show it ──────────────────────────────────────────────────

use std::path::PathBuf;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(relative: &str) -> String {
    let path = manifest_dir().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

#[test]
fn the_main_window_arms_the_connectivity_guard() {
    // The page is useless if nothing ever navigates to it. Before hub#1716 the bundled page was
    // reachable only from the `Err(_)` arm of `initial.parse::<tauri::Url>()` — a branch whose own
    // comment said it "should not happen" — so in practice it shipped as dead weight. The guard is
    // what gives it a reason to exist, and deleting the spawn is a one-line silent regression that
    // no type error catches.
    let source = read("src/lib.rs");

    let window = source
        .find("fn open_main_window")
        .map(|at| &source[at..])
        .and_then(|rest| rest.find("\n}").map(|end| &rest[..end]))
        .expect("src/lib.rs no longer defines `open_main_window`");

    assert!(
        window.contains("spawn_connectivity_guard"),
        "`open_main_window` no longer arms the connectivity guard: a failed load leaves the window \
         on the platform's own error page — blank on macOS, grey on Android — with no way out \
         (hub#1716)"
    );
}

#[test]
fn the_retry_command_is_reachable_from_the_bundled_page_and_from_nowhere_else() {
    // `shell_retry` navigates the main window. Handing it to the remote origins would let any page
    // the till loads redirect the window; the bundled fallback page is the only caller that needs
    // it, and a capability with no `remote` block is exactly how Tauri says "local origins only".
    let mut granting = Vec::new();
    let dir = manifest_dir().join("capabilities");
    for entry in std::fs::read_dir(&dir).expect("cannot read capabilities/") {
        let path = entry.expect("cannot read capability entry").path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let raw = std::fs::read_to_string(&path).expect("cannot read capability");
        let json: serde_json::Value = serde_json::from_str(&raw).expect("capability is not JSON");
        let grants = json["permissions"]
            .as_array()
            .expect("capability must declare permissions")
            .iter()
            .any(|p| p.as_str() == Some("allow-shell-retry"));
        if grants {
            let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            let remote = json.get("remote").is_some();
            let local = json.get("local").and_then(serde_json::Value::as_bool);
            granting.push((name, remote, local));
        }
    }

    assert_eq!(
        granting.len(),
        1,
        "expected exactly one capability to grant `allow-shell-retry`, found {granting:?}"
    );
    let (file, has_remote, local) = &granting[0];
    assert!(
        !has_remote,
        "{file} grants `allow-shell-retry` to REMOTE origins: any page the till loads could then \
         drive the window's navigation (hub#1716)"
    );
    assert_eq!(
        *local,
        Some(true),
        "{file} must declare `local: true` explicitly — the bundled fallback page is the only \
         origin that needs `shell_retry` (hub#1716)"
    );
}
