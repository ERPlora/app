//! Guard on the Android window keeping the hub's forms away from the system autofill (app#15).
//!
//! QA, 2026-10-30, Lenovo tablet with Android 16, app 1.1.31 and Google Password Manager: saving a
//! reservation in the hub popped «Save password to Google Password Manager?» with the customer's
//! e-mail as the user and their notes as the password. Whoever taps «Save» puts a customer's data
//! — health data too, «gluten free» — in the tablet's Google account.
//!
//! The decision lives in `AutofillOptOut` (plugin module), where the Kotlin unit tests reach it on
//! every PR. What no unit test can reach is the wiring: `MainActivity` belongs to `:app`, and
//! `:app` cannot even be configured without files that `cargo tauri android build` writes, so
//! nothing compiles it until the release job. Dropping the override would be silent — green PR,
//! green tags, and a till offering to save customers as passwords again.
//!
//! The lever is the activity because the WebView takes its `AutofillManager` from the context it
//! was created with (wry hands it the activity): with none, Chromium's `AutofillManagerWrapper`
//! treats autofill as disabled and never tells Android a field was entered or a form was sent.
//! `importantForAutofill` on the WebView is not a lever: Chromium never reads it, and Android 14+
//! includes a WebView in the autofill structure whatever its importance.

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    // `src-tauri` → up one.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("cannot resolve the repository root")
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// A Kotlin file without its comments: a commented-out override is not the override. Block comments
/// nest in Kotlin, and their lines need not start with `*`.
fn code_of(source: &str) -> String {
    let mut code = String::new();
    let mut rest = source;
    let mut depth = 0;
    while let Some(c) = rest.chars().next() {
        if rest.starts_with("/*") {
            depth += 1;
            rest = &rest[2..];
        } else if depth > 0 && rest.starts_with("*/") {
            depth -= 1;
            rest = &rest[2..];
        } else if depth == 0 && rest.starts_with("//") {
            rest = rest.find('\n').map_or("", |end| &rest[end..]);
        } else {
            if depth == 0 {
                code.push(c);
            }
            rest = &rest[c.len_utf8()..];
        }
    }
    code
}

const MAIN_ACTIVITY: &str =
    "src-tauri/gen/android/app/src/main/java/com/erplora/app/MainActivity.kt";

const OVERRIDE: &str = "override fun getSystemService(name: String)";

/// What is wrong with how an activity hands out its system services, if anything. Only the
/// override's own text counts, up to the next function of the class.
fn wiring_fault(source: &str) -> Option<&'static str> {
    let activity = code_of(source);
    let Some(start) = activity.find(OVERRIDE) else {
        return Some(
            "no longer overrides getSystemService: the WebView gets the platform's \
             AutofillManager again and hands the hub's forms to the autofill service (app#15)",
        );
    };
    let rest = &activity[start + OVERRIDE.len()..];
    let body = rest.find("fun ").map_or(rest, |next| &rest[..next]);
    if !body.contains("AutofillOptOut.systemService(name)") {
        return Some(
            "overrides getSystemService without going through AutofillOptOut: the autofill \
             service would see the customers' data again (app#15)",
        );
    }
    if !body.contains("super.getSystemService(") {
        return Some(
            "overrides getSystemService without asking the platform for the other services: \
             no keyboard, no window, and the app dies on start",
        );
    }
    None
}

/// The activity must hand out its system services through the opt-out.
///
/// This is the whole user-visible promise of app#15: fill in a form in the hub, save it, and the
/// tablet does not offer to keep any of it as a password.
#[test]
fn the_activity_hands_out_no_autofill_manager() {
    let fault = wiring_fault(&read(MAIN_ACTIVITY));
    assert_eq!(fault, None, "{MAIN_ACTIVITY}");
}

/// Every way of switching the override off that leaves its words in the file. `:app` compiles only
/// at release, so nothing else would notice any of these on a PR.
#[test]
fn an_override_that_does_nothing_is_a_fault() {
    let wired = "override fun getSystemService(name: String): Any? =\n  \
                 AutofillOptOut.systemService(name) { super.getSystemService(it) }\n";
    assert_eq!(wiring_fault(wired), None, "the guard must accept the real wiring");

    for (how, source) in [
        ("removed", String::new()),
        ("commented out by line", format!("// {}", wired.replace('\n', "\n// "))),
        ("commented out by block", format!("/*\n{wired}*/\n")),
        (
            "straight to the platform",
            "override fun getSystemService(name: String): Any? =\n  super.getSystemService(name)\n"
                .to_string(),
        ),
        (
            "opt-out left in a helper nobody calls",
            "override fun getSystemService(name: String): Any? = super.getSystemService(name)\n\
             private fun unused(name: String): Any? = AutofillOptOut.systemService(name) { null }\n"
                .to_string(),
        ),
        (
            "no service at all, the keyboard included",
            "override fun getSystemService(name: String): Any? =\n  \
             AutofillOptOut.systemService(name) { null }\n"
                .to_string(),
        ),
    ] {
        assert!(wiring_fault(&source).is_some(), "the guard let this through: {how}");
    }
}
