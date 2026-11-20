//! Guard on Android's Back button never closing the app (app#61).
//!
//! On a page that does not take Back — erplora.com's sign-in, Google's — with no history left in
//! the WebView, Tauri's handler lets the press through and Android finishes the activity. The
//! process then aborts while the window is torn down (`FORTIFY: pthread_mutex_lock called on a
//! destroyed mutex`), and Google Play counts it as a crash.
//!
//! `BackToBackground` (plugin module, where the Kotlin unit tests reach it on every PR) sends the
//! app to the background instead. What no unit test can reach is the wiring: `MainActivity`
//! belongs to `:app`, which compiles only in the release job, so dropping the call would be silent.

mod common;

use common::{code_of, function_body, read, MAIN_ACTIVITY};

const ON_CREATE: &str = "override fun onCreate(";
const BACK_TO_BACKGROUND: &str =
    "BackToBackground.install(onBackPressedDispatcher) { moveTaskToBack(true) }";

/// What is wrong with how an activity handles the Back nobody else wants, if anything. Only
/// `onCreate` counts: installed there, before the activity starts, the handler sits below Tauri's.
fn wiring_fault(source: &str) -> Option<&'static str> {
    let activity = code_of(source);
    let Some(body) = function_body(&activity, ON_CREATE) else {
        return Some(
            "no longer overrides onCreate: nothing sends the app to the background, and Back on a \
             page that does not take it closes the activity and aborts the process (app#61)",
        );
    };
    if !body.contains(BACK_TO_BACKGROUND) {
        return Some(
            "onCreate does not install BackToBackground with moveTaskToBack(true): Back on a page \
             that does not take it closes the activity and aborts the process (app#61)",
        );
    }
    None
}

/// The whole user-visible promise: Back on any screen, with nowhere left to go, leaves the app in
/// the background, exactly where it was.
#[test]
fn back_with_nowhere_to_go_sends_the_app_to_the_background() {
    let fault = wiring_fault(&read(MAIN_ACTIVITY));
    assert_eq!(fault, None, "{MAIN_ACTIVITY}");
}

/// Every way of dropping the handler that leaves its words in the file.
#[test]
fn a_handler_that_does_not_run_is_a_fault() {
    let wired = "override fun onCreate(savedInstanceState: Bundle?) {\n  \
                 super.onCreate(savedInstanceState)\n  \
                 BackToBackground.install(onBackPressedDispatcher) { moveTaskToBack(true) }\n}\n";
    assert_eq!(wiring_fault(wired), None, "the guard must accept the real wiring");

    for (how, source) in [
        ("commented out", wired.replace("  BackToBackground", "  // BackToBackground")),
        ("inside a block comment", format!("/*\n{wired}*/\n")),
        (
            "moved to a function nobody calls",
            "override fun onCreate(savedInstanceState: Bundle?) {\n  \
             super.onCreate(savedInstanceState)\n}\n\
             private fun back() {\n  \
             BackToBackground.install(onBackPressedDispatcher) { moveTaskToBack(true) }\n}\n"
                .to_string(),
        ),
        (
            "finishing the activity instead",
            wired.replace("moveTaskToBack(true)", "finish()"),
        ),
        ("no onCreate at all", "class MainActivity : TauriActivity() {\n}\n".to_string()),
    ] {
        assert!(wiring_fault(&source).is_some(), "the guard let this through: {how}");
    }
}
