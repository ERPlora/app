//! Guard for the CSP of the SHELL page — the other half of the boundary in hub#334.
//!
//! **Where this policy actually reaches** (read off tauri 2.11.5, `manager::AppManager::get_asset`
//! → `manager::set_csp`): `app.security.csp` is applied ONLY to the assets served out of
//! `frontendDist` through the Tauri protocol. A webview that navigates to `https://erplora.com` or
//! to a hub carries the policy THAT SERVER sends; the config CSP never touches it. So the cloud
//! origins do NOT belong here — adding them would widen the bundled page's policy and buy the
//! remote pages exactly nothing. Only `remote.urls` gates what the cloud origins may do
//! (`remote_acl.rs`).
//!
//! What this file pins, then, is the page the policy DOES govern: `shell-dist/index.html`, the
//! degraded page the window falls back to. Two failure modes, both silent:
//!
//! - the policy drifts open (an `'unsafe-inline'` "to make something work") and the one bundled
//!   page in the app stops being a sealed box;
//! - the policy stays tight but the page quietly violates it — which is exactly what was shipping
//!   before hub#334: the only control on the page was an inline `onclick`, and CSP has no hash and
//!   no nonce for an inline event handler, so the button did nothing at all.

use std::collections::BTreeMap;

/// Both files exactly as they ship, embedded at compile time.
const TAURI_CONF: &str = include_str!("../tauri.conf.json");
const SHELL_PAGE: &str = include_str!("../../shell-dist/index.html");

/// The token Tauri swaps for a fresh nonce when it serves an HTML asset
/// (`tauri_utils::assets::SCRIPT_NONCE_TOKEN`); it also appends `'nonce-…'` to `script-src`.
const SCRIPT_NONCE_TOKEN: &str = "__TAURI_SCRIPT_NONCE__";

/// `app.security.csp` split into `directive -> sources`.
fn csp_directives() -> BTreeMap<String, Vec<String>> {
    let conf: serde_json::Value =
        serde_json::from_str(TAURI_CONF).expect("tauri.conf.json is not valid JSON");
    let csp = conf["app"]["security"]["csp"]
        .as_str()
        .expect("tauri.conf.json must declare app.security.csp as a string");

    csp.split(';')
        .filter_map(|directive| {
            let mut parts = directive.split_whitespace();
            let name = parts.next()?;
            Some((
                name.to_string(),
                parts.map(str::to_string).collect::<Vec<_>>(),
            ))
        })
        .collect()
}

/// Sources of a fetch directive, falling back to `default-src` exactly like a browser does.
fn effective_sources(directive: &str) -> Vec<String> {
    let directives = csp_directives();
    directives
        .get(directive)
        .or_else(|| directives.get("default-src"))
        .cloned()
        .unwrap_or_default()
}

/// Inline event handlers (`onclick=`, `onload=`, …) present in the page. CSP can allow these only
/// with `'unsafe-inline'` (or `'unsafe-hashes'`): there is no per-handler nonce.
fn inline_event_handlers(html: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (index, _) in html.match_indices(" on") {
        let rest = &html[index + " on".len()..];
        let name: String = rest.chars().take_while(char::is_ascii_alphabetic).collect();
        if name.is_empty() {
            continue;
        }
        if rest[name.len()..].trim_start().starts_with('=') {
            found.push(format!("on{name}"));
        }
    }
    found
}

/// Every `<script …>` opening tag in the page.
fn script_tags(html: &str) -> Vec<&str> {
    html.match_indices("<script")
        .filter_map(|(index, _)| {
            let rest = &html[index..];
            rest.find('>').map(|end| &rest[..=end])
        })
        .collect()
}

/// `src=`/`href=` values that point off the bundled origin.
fn remote_asset_references(html: &str) -> Vec<String> {
    let mut found = Vec::new();
    for attribute in ["src=", "href="] {
        for (index, _) in html.match_indices(attribute) {
            let value: String = html[index + attribute.len()..]
                .trim_start_matches(['"', '\''])
                .chars()
                .take_while(|c| !matches!(c, '"' | '\'' | ' ' | '>'))
                .collect();
            if value.starts_with("http://") || value.starts_with("https://") || value.starts_with("//")
            {
                found.push(value);
            }
        }
    }
    found
}

// ── The policy stays as tight as it is ──────────────────────────────────────────────────────────

#[test]
fn the_shell_page_loads_nothing_off_the_bundle() {
    // `default-src 'self'` — the degraded page ships whole inside the app. Anything it pulled from
    // the network would be code we do not control running in a webview with `withGlobalTauri`.
    assert_eq!(csp_directives().get("default-src"), Some(&vec!["'self'".to_string()]));
}

#[test]
fn the_shell_page_can_neither_inline_nor_evaluate_script() {
    let script = effective_sources("script-src");
    for forbidden in ["'unsafe-inline'", "'unsafe-eval'"] {
        assert!(
            !script.iter().any(|source| source == forbidden),
            "script-src allows {forbidden}: the one bundled page stops being a sealed box, and \
             `onclick=`-style code creeps back in (it is dead everywhere else in ERPlora too)"
        );
    }
}

#[test]
fn the_shell_page_frames_nothing_and_rebases_nowhere() {
    let directives = csp_directives();
    assert_eq!(directives.get("object-src"), Some(&vec!["'none'".to_string()]));
    assert_eq!(directives.get("frame-src"), Some(&vec!["'none'".to_string()]));
    assert_eq!(directives.get("base-uri"), Some(&vec!["'self'".to_string()]));
}

#[test]
fn the_shell_csp_carries_no_cloud_origin() {
    // Deliberate, and the opposite of what `architecture/hub/apps/tauri.md` used to say: this
    // policy never reaches the pages served from erplora.com, so listing the cloud origins here
    // would only widen the bundled page for nothing. The cloud origins live in `remote.urls`.
    for (directive, sources) in csp_directives() {
        for source in sources {
            assert!(
                !source.contains("erplora.com"),
                "{directive} lists `{source}`; the config CSP only ever applies to the bundled \
                 page, so a cloud origin here widens the wrong thing"
            );
        }
    }
}

#[test]
fn the_shell_csp_does_not_pin_form_action() {
    // `form-action` does NOT fall back to `default-src`, so its absence is a decision rather than
    // an oversight — and it must stay absent. Pinning it is what kills federated login: the Google
    // OAuth chain posts across origins and comes back through the SaaS, and a `form-action 'self'`
    // breaks the redirect chain with no error the user can act on.
    assert!(
        !csp_directives().contains_key("form-action"),
        "form-action is pinned in the shell CSP: that is the directive that kills the Google login \
         redirect chain"
    );
}

// ── …and the page it governs actually works under it ────────────────────────────────────────────

#[test]
fn the_degraded_page_has_no_inline_event_handlers() {
    let handlers = inline_event_handlers(SHELL_PAGE);
    assert!(
        handlers.is_empty(),
        "shell-dist/index.html carries inline handlers {handlers:?}; with script-src at 'self' the \
         browser drops them without a word, so the control they are attached to does nothing"
    );
}

#[test]
fn the_degraded_page_wires_its_retry_control_from_a_script() {
    // The page's only control is the retry button. Under this CSP the only way to wire it is from
    // a script the policy admits, so the page must actually carry one.
    assert!(
        SHELL_PAGE.contains("addEventListener"),
        "shell-dist/index.html has no script wiring its retry control: the button is decoration"
    );
}

#[test]
fn every_inline_script_carries_the_tauri_nonce_token() {
    // An inline block is allowed only because Tauri swaps this token for a real nonce as it serves
    // the asset. Drop the attribute and the script is blocked — silently, just like the `onclick`
    // was. (A same-origin `<script src>` is fine too: Tauri hashes bundled `.js` into script-src.)
    for tag in script_tags(SHELL_PAGE) {
        if tag.contains("src=") {
            continue;
        }
        assert!(
            tag.contains(SCRIPT_NONCE_TOKEN),
            "inline script `{tag}` has no {SCRIPT_NONCE_TOKEN}: script-src is 'self', so it will \
             never run"
        );
    }
}

#[test]
fn the_degraded_page_pulls_no_remote_asset() {
    let remote = remote_asset_references(SHELL_PAGE);
    assert!(
        remote.is_empty(),
        "shell-dist/index.html references {remote:?}: the fallback page must render with no \
         network at all — it is the page you get precisely when there is none"
    );
}
