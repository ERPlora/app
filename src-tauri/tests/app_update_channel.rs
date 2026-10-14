//! The two ends of the update channel of the installed app (hub#400), each of which fails MUTELY.
//!
//! The web app asks the shell "which build are you?" and compares it with what the Cloud
//! publishes. Neither half of that is code in this crate — the question is TypeScript and the
//! answer comes from a command — which is exactly why they need a guard here: **both ends can be
//! removed without anything failing to compile.**
//!
//! * The question goes, first, to Tauri's own `plugin:app|version` — the one an app installed
//!   before this channel existed answers — and, where that is refused, to `erplora_bridge_status`,
//!   which carries the same version sealed by the release. Drop the second one and every app built
//!   from hub#2658 on goes quiet, forever, with no error anywhere.
//! * Stop publishing the version manifest from the release workflow and the Cloud has nothing to
//!   report. Same silence, other end.
//!
//! And the version belongs to the LINKED hub (hub#2658): `core:app` is a Tauri plugin no gate can
//! stand in front of, so the hub PWA's capability does not grant it — otherwise any page under
//! erplora.com (another business, the website, the test SaaS) reads which build the till runs.
//! `erplora_bridge_status` is one of the app's own commands, behind `src/hub_link.rs`.
//!
//! Neither is hypothetical bookkeeping: silence is the *correct* behaviour of the feature when it
//! cannot check, so a broken update channel is indistinguishable from a healthy one that has
//! nothing to say. Nothing but these assertions would ever notice.

use std::path::PathBuf;

/// What the page invokes. Kept as literals on both sides: the TypeScript writes the same strings,
/// and there is no shared symbol between a webview bundle and this crate to bind them.
const VERSION_COMMAND: &str = "plugin:app|version";
const GATED_VERSION_COMMAND: &str = "erplora_bridge_status";

/// The permission that grants the gated one to the hub PWA.
const GATED_VERSION_PERMISSION: &str = "allow-erplora-bridge-status";

/// The sets that would hand `plugin:app|version` to every page of the pattern: `core:default` is
/// `["core:path:default", …, "core:app:default", …]`, and `core:app:default` enables `version`
/// (tauri `build.rs`, PLUGINS).
const CORE_DEFAULT: &str = "core:default";
const CORE_APP: &str = "core:app:";

/// The only part of `core:app` the hub PWA keeps: the Android Back button reaches the page as the
/// `back-button` event of that plugin (`onBackButtonPress`, `src/router/back-closes-overlay.ts`,
/// hub#1906). It says nothing about the build, and it is the page on screen that hears it.
const BACK_BUTTON_PERMISSIONS: [&str; 2] = [
    "core:app:allow-register-listener",
    "core:app:allow-remove-listener",
];

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

fn hub_pwa_capability() -> serde_json::Value {
    let raw = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("capabilities/default.json"),
    )
    .expect("cannot read capabilities/default.json");
    serde_json::from_str(&raw).expect("capabilities/default.json is not valid JSON")
}

fn hub_pwa_permissions() -> Vec<String> {
    hub_pwa_capability()["permissions"]
        .as_array()
        .expect("permissions must be an array")
        .iter()
        .map(|value| {
            value
                .as_str()
                .expect("every permission is a string")
                .to_owned()
        })
        .collect()
}

#[test]
fn the_hub_pwa_may_ask_the_shell_which_build_it_is() {
    assert!(
        hub_pwa_permissions()
            .iter()
            .any(|p| p == GATED_VERSION_PERMISSION),
        "capabilities/default.json no longer grants {GATED_VERSION_PERMISSION}, so \
         `{GATED_VERSION_COMMAND}` is refused to the hub PWA and every app built from hub#2658 on \
         stops being able to tell whether it is out of date — in silence, which is also what a \
         healthy till does when there is no news (hub#400)."
    );
}

#[test]
fn no_page_under_erplora_com_reads_the_build_through_tauri() {
    // The pattern lets every page under erplora.com through, and Tauri's `app` plugin answers
    // whoever passes it: no second gate can be put in front of a core plugin (hub#2658).
    for permission in hub_pwa_permissions() {
        let back_button = BACK_BUTTON_PERMISSIONS.contains(&permission.as_str());
        assert!(
            permission != CORE_DEFAULT && (back_button || !permission.starts_with(CORE_APP)),
            "capabilities/default.json grants {permission}, which hands `{VERSION_COMMAND}` to \
             any page under erplora.com — another business, the website, the test SaaS — and not \
             only to the linked hub (hub#2658)"
        );
    }
}

#[test]
fn keeping_the_build_from_other_pages_keeps_the_android_back_button() {
    // Dropping `core:default` for hub#2658 also dropped the listener of `core:app`: Tauri's
    // activity then turns Back into `webView.goBack()` again and the router takes the screen —
    // and the sale sheet, the modal or the side menu with it (hub#1906).
    let granted = hub_pwa_permissions();
    for permission in BACK_BUTTON_PERMISSIONS {
        assert!(
            granted.iter().any(|p| p == permission),
            "capabilities/default.json no longer grants {permission}, so the hub PWA cannot hear \
             the Android Back button and Back leaves the screen with the sheet still open \
             (hub#1906, hub#2658)"
        );
    }
}

// `the_page_and_the_shell_name_the_same_command` read `apps/web/src/lib/app-update.ts`, the page
// half of this channel, which did not move with the app (README.md, «Tests retirados»).

/// The step of `tauri-release.yml` that uploads the installers and the manifest to S3 (AWS, by
/// OIDC; it was Hetzner's object storage in ERPlora/hub).
const UPLOAD_STEP: &str = "Upload installers and latest.json to S3";

#[test]
fn the_release_publishes_the_version_the_cloud_reports() {
    // Without this file there is nothing to compare against: the bucket holds installers, and an
    // installer does not say which version it is. The Cloud reads exactly this key.
    let workflow = read(".github/workflows/tauri-release.yml");
    assert!(
        workflow.contains("latest.json"),
        "the release workflow no longer publishes the version manifest, so the Cloud has nothing \
         to report and every till reads `unknown` forever (hub#400)"
    );
    assert!(
        workflow.contains("downloads/app/latest/latest.json"),
        "the version manifest must land on the key the SaaS reads — `downloads/app/latest/latest.json`"
    );
    // The SaaS reads the AWS bucket (saas-next `public/appdist.py`, `DOWNLOADS_BUCKET`): a manifest
    // still written to Hetzner would be a channel nobody reads.
    assert!(
        !workflow.contains("HETZNER_S3_") && workflow.contains("vars.DOWNLOADS_BUCKET"),
        "the manifest must go to the AWS downloads bucket (`vars.DOWNLOADS_BUCKET`), not Hetzner"
    );
}

#[test]
fn the_manifest_is_refreshed_for_every_channel_the_bucket_serves() {
    // `latest/` is the folder the Cloud serves from. A manifest written apart from the installers
    // could announce a version whose installer is not there yet, or leave `latest/` announcing the
    // previous one — an update that never stops being offered.
    let workflow = read(".github/workflows/tauri-release.yml");
    let manifest_step = workflow
        .split(UPLOAD_STEP)
        .nth(1)
        .expect("the upload step is where the manifest is written");
    assert!(
        manifest_step.contains("latest.json"),
        "the manifest must be written by the same step that uploads the installers, so the two \
         can never disagree about what `latest/` holds"
    );
}
