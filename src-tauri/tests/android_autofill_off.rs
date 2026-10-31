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

/// A file with every comment line removed: a `//` line explaining a rule is not the rule.
fn code_of(source: &str) -> String {
    source
        .lines()
        .filter(|line| {
            let line = line.trim_start();
            !(line.starts_with("//") || line.starts_with("*") || line.starts_with("/*"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

const MAIN_ACTIVITY: &str =
    "src-tauri/gen/android/app/src/main/java/com/erplora/app/MainActivity.kt";

/// The activity must hand out its system services through the opt-out.
///
/// This is the whole user-visible promise of app#15: fill in a form in the hub, save it, and the
/// tablet does not offer to keep any of it as a password.
#[test]
fn the_activity_hands_out_no_autofill_manager() {
    let activity = code_of(&read(MAIN_ACTIVITY));

    assert!(
        activity.contains("override fun getSystemService(name: String)"),
        "{MAIN_ACTIVITY} no longer overrides getSystemService: the WebView gets the platform's \
         AutofillManager again and hands the hub's forms to the autofill service (app#15)",
    );
    assert!(
        activity.contains("AutofillOptOut.systemService(name)"),
        "{MAIN_ACTIVITY} overrides getSystemService without going through AutofillOptOut: the \
         autofill service would see the customers' data again (app#15)",
    );
}
