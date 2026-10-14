//! A tap on a system notice opens the screen it is about (hub#2305) — and both ends of that are
//! strings nothing compiles against, so both can go missing in silence.
//!
//! * The page subscribes to the notification plugin's `actionPerformed` through
//!   `plugin:notification|register_listener`. Without `notification:allow-register-listener` in the
//!   hub PWA's capability the ACL refuses it from the remote origin, the shell logs one warning and
//!   every tap goes back to opening the app wherever it was — which is also what an older app does,
//!   so nothing would look broken.
//! * The page sends the notice with an `id` (the only handle the tap comes back with on Android and
//!   iOS alike). A shell whose `erplora_notify` stops reading it shows the notice under a random id,
//!   and the tap no longer matches any destination.

use std::path::PathBuf;

const REGISTER_LISTENER: &str = "notification:allow-register-listener";

fn read(relative_to_manifest: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative_to_manifest);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn permissions_of(capability: &str) -> Vec<String> {
    let json: serde_json::Value =
        serde_json::from_str(&read(capability)).expect("the capability is not valid JSON");
    json["permissions"]
        .as_array()
        .expect("permissions must be an array")
        .iter()
        .map(|value| value.as_str().expect("every permission is a string").to_owned())
        .collect()
}

#[test]
fn the_hub_pwa_may_hear_a_notice_being_tapped() {
    let permissions = permissions_of("capabilities/default.json");
    assert!(
        permissions.iter().any(|p| p == REGISTER_LISTENER),
        "capabilities/default.json does not grant {REGISTER_LISTENER}: the hub PWA cannot hear a \
         tap on a notice, and tapping «New booking» opens the app wherever it was (hub#2305)"
    );
}

#[test]
fn only_the_hub_pwa_gets_to_listen() {
    // The SaaS apex and the degraded page never send a notice; nothing there has a tap to follow.
    for other in ["capabilities/onboarding.json", "capabilities/degraded.json"] {
        assert!(
            !permissions_of(other).iter().any(|p| p.starts_with("notification:")),
            "{other} grants a notification permission it has no use for"
        );
    }
}

#[test]
fn the_page_and_the_shell_name_the_same_event_and_argument() {
    // The page half (`apps/web/src/main.ts`, `apps/web/src/lib/bridge-transport.ts`) did not move
    // with the app; only the shell half is checked here (README.md, «Tests retirados»).
    let shell = read("src/lib.rs");
    assert!(
        shell.contains(
            "fn erplora_notify(app: tauri::AppHandle, title: String, body: String, id: Option<i64>, path: Option<String>)"
        ),
        "`erplora_notify` no longer takes the `id` and screen the page sends"
    );
    // Taking the `id` is not enough: a command that shows the notice under `None` compiles with a
    // mere warning, and every tap comes back under a random id that leads nowhere.
    assert!(
        shell.contains("notice_builder(&app, &title, &body, id, path.as_deref()).show()"),
        "`erplora_notify` takes the `id` but no longer hands it to the notice it shows"
    );
    // On the computer the plugin reports no click (hub#2360): the shell shows the notice itself.
    assert!(
        shell.contains("notify_on_desktop(app, title, body, id, path, notice_tap::deliver);"),
        "`erplora_notify` no longer shows the desktop notice itself: a click opens nothing"
    );
}

// ── hub#2360: the tap the page was not there to hear ────────────────────────────────────────────
//
// A click on the computer and a tap that starts the app on Android both land in the shell before
// (or without) the page hearing them. The shell keeps the tap and the page claims it through a
// command and an event — two more strings nothing compiles against.

#[test]
fn the_page_claims_the_tap_the_shell_kept() {
    // The page half (`apps/web/src/main.ts`) did not move with the app; only the shell half is
    // checked here (README.md, «Tests retirados»).
    let shell = read("src/notice_tap.rs");
    assert!(
        shell.contains("pub const NOTICE_TAPPED_EVENT: &str = \"erplora://notice-tapped\";"),
        "the shell and the page no longer name the same «a tap is waiting» event"
    );
}

#[test]
fn the_hub_pwa_may_claim_a_kept_tap() {
    let lib = read("src/lib.rs");
    assert!(lib.contains("fn erplora_take_notice_tap("), "the claim command is gone");
    assert!(
        lib.contains("launch_tap_payload(app.erplora_android().take_notice_tap())"),
        "the claim no longer asks Kotlin for the tap that started the app"
    );
    let handler = &lib[lib.find("generate_handler![").expect("generate_handler!")..];
    assert!(handler.contains("erplora_take_notice_tap"), "the claim command is not registered");
    assert!(
        read("build.rs").contains("\"erplora_take_notice_tap\""),
        "build.rs does not declare the claim command, so the ACL has no permission for it"
    );
    assert!(
        permissions_of("capabilities/default.json").iter().any(|p| p == "allow-erplora-take-notice-tap"),
        "the hub PWA may not claim a kept tap: a click on the computer brings the app up and opens nothing"
    );
    for other in ["capabilities/onboarding.json", "capabilities/degraded.json"] {
        assert!(
            !permissions_of(other).iter().any(|p| p == "allow-erplora-take-notice-tap"),
            "{other} may claim a notice tap it never sent"
        );
    }
}

#[test]
fn a_tap_that_reaches_a_dead_process_is_kept_by_the_activity() {
    // The system killed the process but kept the task: the activity is restored with its old
    // launcher intent and the tap arrives through `onNewIntent` before the WebView — and so before
    // any plugin — exists (rv-2411, measured on the emulator). Only the activity sees it.
    let activity = read("gen/android/app/src/main/java/com/erplora/app/MainActivity.kt");
    let hook = activity
        .split("override fun onNewIntent(intent: Intent)")
        .nth(1)
        .expect("MainActivity does not hear a tap delivered to a restored activity");
    let hook = hook.split("\n  }").next().unwrap_or_default();
    assert!(hook.contains("super.onNewIntent(intent)"), "the plugins no longer get the new intent");
    assert!(hook.contains("NoticeTaps.newIntent(this, intent)"), "the tap is not kept for the page");
}
