//! Guard on the Android window staying lit while the app is in the foreground (app#26).
//!
//! QA, 2026-11-07, Lenovo TB361FU with Android 16 and app 1.1.34: with the till open and nobody
//! touching the tablet, the screen went dark at the system's sleep timeout (`dumpsys window` →
//! `mHoldScreenWindow=null`, `Display #0 state=OFF`). A till or a kitchen screen is in service all
//! day; a black panel on the counter has to be woken up every time.
//!
//! The flag lives in `ScreenOn` (plugin module), where the Kotlin unit tests reach it on every PR.
//! What no unit test can reach is the wiring: `MainActivity` belongs to `:app`, and `:app` cannot
//! even be configured without files that `cargo tauri android build` writes, so nothing compiles it
//! until the release job. Dropping the call would be silent — green PR, green tags, and a till
//! going dark again.
//!
//! `FLAG_KEEP_SCREEN_ON` is the lever because Android honours it only while the window is visible:
//! the power button still turns the screen off, and in the background the system decides again.
//! No wake lock, no foreground service and no permission (app#23 retired the service).

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

/// A Kotlin file without its comments: a commented-out call is not the call. Block comments nest
/// in Kotlin, and their lines need not start with `*`.
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

const ON_CREATE: &str = "override fun onCreate(";
const KEEP_SCREEN_ON: &str = "ScreenOn.apply(window::addFlags)";

/// What is wrong with how an activity keeps its window lit, if anything. Only `onCreate`'s own
/// text counts, up to the next function of the class: that is where the window is set up, once per
/// activity, recreations included.
fn wiring_fault(source: &str) -> Option<&'static str> {
    let activity = code_of(source);
    let Some(start) = activity.find(ON_CREATE) else {
        return Some(
            "no longer overrides onCreate: nothing sets FLAG_KEEP_SCREEN_ON on the window, and the \
             till goes dark at the system's sleep timeout (app#26)",
        );
    };
    let rest = &activity[start + ON_CREATE.len()..];
    let body = rest.find("fun ").map_or(rest, |next| &rest[..next]);
    if !body.contains(KEEP_SCREEN_ON) {
        return Some(
            "onCreate does not apply ScreenOn to the window: the till goes dark at the system's \
             sleep timeout while it is open (app#26)",
        );
    }
    None
}

/// The activity must keep its window lit.
///
/// This is the whole user-visible promise of app#26: leave the till open on the counter, and the
/// screen stays on until someone presses the power button or leaves the app.
#[test]
fn the_activity_keeps_its_window_lit() {
    let fault = wiring_fault(&read(MAIN_ACTIVITY));
    assert_eq!(fault, None, "{MAIN_ACTIVITY}");
}

/// Every way of dropping the call that leaves its words in the file. `:app` compiles only at
/// release, so nothing else would notice any of these on a PR.
#[test]
fn a_call_that_does_not_run_is_a_fault() {
    let wired = "override fun onCreate(savedInstanceState: Bundle?) {\n  \
                 super.onCreate(savedInstanceState)\n  \
                 ScreenOn.apply(window::addFlags)\n}\n";
    assert_eq!(wiring_fault(wired), None, "the guard must accept the real wiring");

    for (how, source) in [
        (
            "commented out",
            "override fun onCreate(savedInstanceState: Bundle?) {\n  \
             super.onCreate(savedInstanceState)\n  \
             // ScreenOn.apply(window::addFlags)\n}\n",
        ),
        (
            "inside a block comment",
            "override fun onCreate(savedInstanceState: Bundle?) {\n  \
             super.onCreate(savedInstanceState)\n  \
             /* ScreenOn.apply(window::addFlags) */\n}\n",
        ),
        (
            "moved to a function nobody calls",
            "override fun onCreate(savedInstanceState: Bundle?) {\n  \
             super.onCreate(savedInstanceState)\n}\n\
             private fun keepLit() {\n  ScreenOn.apply(window::addFlags)\n}\n",
        ),
        (
            "no onCreate at all",
            "class MainActivity : TauriActivity() {\n}\n",
        ),
    ] {
        assert!(wiring_fault(source).is_some(), "the guard must reject a call {how}");
    }
}
