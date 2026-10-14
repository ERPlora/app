//! Guard for the Android permissions the till needs to reach its hardware (hub#337, ADR-0196 §9).
//!
//! Android's twin of `macos_local_network.rs`, and the same failure mode: a missing permission
//! does not raise anything. It makes the hardware **quietly unreachable** — the sweep of port 9100
//! becomes 254 timeouts and the till reports an empty venue while the printer sits there switched
//! on. Since ADR-0196 the installable app is the ONLY way to the hardware, so this is the whole
//! hardware story on Android, not a corner of it.
//!
//! What makes Android worse than macOS is *where the declaration lives*. `gen/android` is a
//! **generated** project, and this is what `cargo tauri android init` actually does with it —
//! measured with tauri-cli 2.11.2 on this tree, not read in a changelog:
//!
//! - with the manifest present, it leaves the file **untouched**: the hand-written permissions and
//!   comments survive, and `git status` comes back clean;
//! - with the manifest **missing**, it writes Tauri's template back — `INTERNET` and nothing else.
//!   Gone: the four hardware permissions and the `erplora://` intent filter (hub#345). Kept: the
//!   leanback feature and the file provider, which come from the template.
//!
//! So the hazard is not "someone reruns init", it is "the file is not there when init runs": a
//! `rm -rf gen/android` to unstick Gradle, a merge that drops the file, a clean checkout on a
//! machine where it was wiped. And the loss is completely silent — the APK still builds, still
//! installs, still runs. Only the printers stop existing.
//!
//! So the declaration is kept in two places on purpose (the lesson of hub#345, where Android alone
//! needed the deep-link scheme declared twice):
//!
//! 1. in the plugin's own Android library manifest, under `crates/`, where no generator reaches —
//!    the manifest merger folds it back into the APK even if `gen/android` is thrown away and
//!    rebuilt from scratch;
//! 2. in the generated app manifest, which is what a reader opens first.
//!
//! These tests are the tripwire on both, plus one on the release job, which is the only place the
//! *merged* manifest exists at all.
//!
//! Why loudly, and not "it will be obvious": asking for a permission the merged manifest never
//! declared is not answered with a dialog. Android answers DENIED immediately, forever, without
//! showing the user anything. The till would then say "grant local network access to ERPlora in
//! your device settings" (hub#338) and send the user looking for a toggle that does not exist.

use std::fs;
use std::path::PathBuf;

/// Everything the merged manifest must declare for the till to reach a network printer and tell
/// the kitchen about it.
///
/// `INTERNET` is in the list even though Tauri's template writes it: on Android 16 and below it is
/// also what implicitly allowed the LAN, which is precisely why losing the rest of this list stays
/// invisible until someone installs the till on a newer device.
const REQUIRED: &[&str] = &[
    "android.permission.INTERNET",
    "android.permission.ACCESS_NETWORK_STATE",
    "android.permission.CHANGE_WIFI_MULTICAST_STATE",
    "android.permission.ACCESS_LOCAL_NETWORK",
    "android.permission.POST_NOTIFICATIONS",
    // Bluetooth Classic SPP printing (ADR-0204, hub#388). Runtime permission since API 31, and
    // the same silent failure as the LAN one: without it the bonded list is empty and the till
    // reports no bluetooth printers.
    "android.permission.BLUETOOTH_CONNECT",
    // Reading the staff badge off the device's own reader (hub#988). An install-time `normal`
    // permission — no dialog, so it is not in `PermissionPolicy` — but the same "declared or
    // nothing" rule applies: without it `enableReaderMode` throws and a tablet cannot enrol a card.
    "android.permission.NFC",
    // Staying alive to hear the next notice with the screen off (hub#2307). Both are install-time
    // `normal` permissions, so there is no dialog either; but without them `startForeground`
    // throws a SecurityException and Android freezes the page the moment the screen goes dark.
    "android.permission.FOREGROUND_SERVICE",
    "android.permission.FOREGROUND_SERVICE_SPECIAL_USE",
    // A module asking where the device is (hub#2552); the first one is `attendance`, which can
    // require the employee to clock in near the venue. RUNTIME permissions: wry's
    // `RustWebChromeClient.onGeolocationPermissionsShowPrompt` asks for them when a page calls
    // `navigator.geolocation`. Without the declaration the request is answered DENIED with no
    // dialog, the page gets PERMISSION_DENIED, and a personal-device clock-in is rejected forever.
    "android.permission.ACCESS_FINE_LOCATION",
    "android.permission.ACCESS_COARSE_LOCATION",
];

/// The hardware feature the NFC permission drags in behind it (hub#988).
///
/// This is not a permission and it is not decoration. Declaring `android.permission.NFC` makes
/// Google Play add an **implicit** `android.hardware.nfc` requirement, and Play then hides the app
/// from every device without an NFC chip. That is most cheap counter tablets — the customers
/// LEAST likely to own a USB reader and most in need of the app. The listing would narrow because
/// of a convenience feature, and nothing in the build would say so.
const NFC_FEATURE: &str = "android.hardware.nfc";

/// The hardware feature the location permissions drag in behind them (hub#2552).
///
/// Same trap as [`NFC_FEATURE`], and NOT gated by the target SDK: only the `.gps` / `.network`
/// sub-features stop being implied at targetSdk 21, the parent `android.hardware.location` is
/// implied by ACCESS_COARSE_LOCATION / ACCESS_FINE_LOCATION whatever the target (read back with
/// aapt2 off an APK at minSdk 24 / targetSdk 36). The till does not need a location at all; only
/// a module like `attendance` asks for one, on a personal phone, and degrades when there is none.
const LOCATION_FEATURE: &str = "android.hardware.location";

/// Every hardware feature that one of the permissions in [`REQUIRED`] implies and the till can
/// live without. Each one must be declared `required="false"` or Play hides the app from every
/// device lacking it.
const OPTIONAL_FEATURES: &[&str] = &[NFC_FEATURE, LOCATION_FEATURE];

/// The manifest of the generated Android project — the one a lost file makes `android init` rewrite.
const APP_MANIFEST: &str = include_str!("../gen/android/app/src/main/AndroidManifest.xml");

/// The release job, the only place where the *merged* manifest of a real APK can be read back.
const RELEASE_WORKFLOW: &str = include_str!("../../.github/workflows/tauri-release.yml");

/// Everything outside XML comments.
///
/// Load-bearing, not tidiness: commenting a tag out is the easiest way to "temporarily" drop a
/// permission, and it is exactly the edit nobody remembers to undo.
fn without_comments(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find("-->") else { return out };
        rest = &rest[start + end + "-->".len()..];
    }
    out.push_str(rest);
    out
}

/// Permissions actually DECLARED by a manifest, read out of its live `<uses-permission>` tags.
///
/// Deliberately not a `contains()` on the raw text: these manifests carry long comments that name
/// `ACCESS_LOCAL_NETWORK` and explain why it is there, so a text search would keep passing on the
/// prose after the tag itself was gone. A guard that goes green on the wreckage is worse than no
/// guard at all.
fn declared_permissions(manifest: &str) -> Vec<String> {
    without_comments(manifest)
        .split("<uses-permission")
        .skip(1)
        .filter_map(|tag| {
            tag.split("android:name=\"")
                .nth(1)
                .and_then(|value| value.split('"').next())
                .map(str::to_owned)
        })
        .collect()
}

/// The plugin's Android library manifest: the copy of the declaration that lives outside `gen/`.
fn plugin_manifest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../crates/tauri-plugin-erplora-android/android/src/main/AndroidManifest.xml")
}

// ── The guard reads tags, not prose ─────────────────────────────────────────────────────────────

#[test]
fn a_commented_out_permission_does_not_count_as_declared() {
    // How a permission usually disappears: someone comments it out "for a minute" while chasing
    // something else. The prose around it still names it — this manifest explains every permission
    // it declares — so a text search would call it declared and wave the regression through. This
    // is the test that keeps the rest of the file from going quietly vacuous.
    let manifest = r#"<manifest>
        <uses-permission android:name="android.permission.INTERNET" />
        <!-- ACCESS_LOCAL_NETWORK is what lets the sweep reach port 9100:
             <uses-permission android:name="android.permission.ACCESS_LOCAL_NETWORK" /> -->
    </manifest>"#;

    assert_eq!(declared_permissions(manifest), ["android.permission.INTERNET"]);
}

#[test]
fn an_unterminated_comment_hides_everything_after_it() {
    // A stray `<!--` swallows the rest of the file for Android too, so the guard must read it the
    // way the platform does. Reporting the swallowed tags as declared would be the guard telling
    // us a broken manifest is fine.
    let manifest = r#"<manifest>
        <!-- oops
        <uses-permission android:name="android.permission.ACCESS_LOCAL_NETWORK" />
    </manifest>"#;

    assert!(declared_permissions(manifest).is_empty());
}

// ── The generated manifest still declares what the shell asks for ───────────────────────────────

// Regression test for ERPlora/hub#2552
#[test]
fn the_generated_manifest_declares_every_permission_the_till_needs() {
    let declared = declared_permissions(APP_MANIFEST);
    for permission in REQUIRED {
        assert!(
            declared.iter().any(|d| d == permission),
            "{permission} is no longer declared in gen/android/app/src/main/AndroidManifest.xml.\n\
             That file belongs to a GENERATED project: if it goes missing, `cargo tauri android \
             init` writes Tauri's template back, and the template declares INTERNET and nothing \
             else. Put it back — the full list lives in \
             crates/tauri-plugin-erplora-android/android/src/main/AndroidManifest.xml.\n\
             Declared right now: {declared:?}"
        );
    }
}

#[test]
fn the_generated_manifest_is_not_the_bare_template_again() {
    // The shape of the regression, not just its parts. A project regenerated over a missing file
    // declares exactly one permission — verified by doing it: the failure below is the message
    // this test printed against the template `cargo tauri android init` really wrote. Landing back
    // there is the accident this whole file exists for, and it deserves one sentence that says so
    // rather than five separate "missing X" failures.
    let declared = declared_permissions(APP_MANIFEST);
    assert!(
        declared.len() > 1,
        "the app manifest is back to Tauri's template ({declared:?}) — a regeneration was committed \
         and it took the hardware permissions and the erplora:// intent filter with it"
    );
}

// ── …and a regeneration cannot take the permissions away with it ────────────────────────────────

#[test]
fn the_permissions_also_live_where_regenerating_the_project_cannot_reach_them() {
    // The Android manifest merger folds a library module's `<uses-permission>` tags into the app
    // manifest at build time. Keeping the declaration in the plugin — which is a hand-written
    // crate, not a generated project — is what makes it survive `android init` by construction,
    // instead of surviving because nobody happened to run the command.
    let path = plugin_manifest_path();
    let manifest = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e}\n\
             The plugin ships no Android manifest, so the hardware permissions exist ONLY inside \
             the generated gen/android project. Lose that file once and `cargo tauri android init` \
             brings back a template without them — a green build and a till that never finds a \
             printer.",
            path.display()
        )
    });

    let declared = declared_permissions(&manifest);
    for permission in REQUIRED {
        assert!(
            declared.iter().any(|d| d == permission),
            "{permission} is missing from the plugin manifest, so a regenerated gen/android would \
             ship without it.\nDeclared right now: {declared:?}"
        );
    }
}

// ── A permission must not narrow who can install the app ────────────────────────────────────────

/// Does this manifest say `feature` is OPTIONAL?
///
/// Read as a live `<uses-feature>` tag with `required="false"` on it, for the same reason
/// [`declared_permissions`] reads tags: the prose around it names the feature, and a `contains()`
/// would keep passing on the explanation after the tag was gone.
fn feature_is_optional(manifest: &str, feature: &str) -> bool {
    without_comments(manifest)
        .split("<uses-feature")
        .skip(1)
        .filter(|tag| {
            tag.split("android:name=\"")
                .nth(1)
                .and_then(|value| value.split('"').next())
                == Some(feature)
        })
        .any(|tag| {
            tag.split("android:required=\"")
                .nth(1)
                .and_then(|value| value.split('"').next())
                == Some("false")
        })
}

#[test]
fn a_manifest_that_only_talks_about_the_feature_does_not_declare_it_optional() {
    // The guard has to detect the positive before it is allowed to certify a negative. A manifest
    // that merely mentions the feature — in a comment, or requiring it — is NOT the opt-out.
    assert!(!feature_is_optional(
        r#"<manifest><!-- android.hardware.nfc required=false --></manifest>"#,
        NFC_FEATURE
    ));
    assert!(!feature_is_optional(
        r#"<manifest><uses-feature android:name="android.hardware.nfc" android:required="true" /></manifest>"#,
        NFC_FEATURE
    ));
    assert!(feature_is_optional(
        r#"<manifest><uses-feature android:name="android.hardware.nfc" android:required="false" /></manifest>"#,
        NFC_FEATURE
    ));
    // A sibling feature being optional says nothing about this one: `.gps` is not the parent.
    assert!(!feature_is_optional(
        r#"<manifest><uses-feature android:name="android.hardware.location.gps" android:required="false" /></manifest>"#,
        LOCATION_FEATURE
    ));
}

// Regression test for ERPlora/hub#2552
#[test]
fn an_implied_feature_does_not_hide_the_app_from_devices_without_it() {
    // Both copies, because either one alone would let the requirement back in: the merger takes
    // the STRICTEST of the two, so an app manifest that stays quiet while the plugin says
    // `required="false"` is fine, but a `required="true"` anywhere wins.
    for (name, manifest) in [
        ("gen/android/app/src/main/AndroidManifest.xml", APP_MANIFEST.to_string()),
        (
            "crates/tauri-plugin-erplora-android/.../AndroidManifest.xml",
            fs::read_to_string(plugin_manifest_path()).expect("the plugin manifest"),
        ),
    ] {
        for feature in OPTIONAL_FEATURES {
            assert!(
                feature_is_optional(&manifest, feature),
                "{name} declares a permission that implies {feature} without saying it is \
                 optional. Google Play adds that IMPLICIT requirement and hides the app from every \
                 device without it — the cheap counter tablets this till is for. Put back: \
                 <uses-feature android:name=\"{feature}\" android:required=\"false\" /> \
                 (hub#988, hub#2552)"
            );
        }
    }
}

// ── The only check that reads the MERGED manifest ───────────────────────────────────────────────

#[test]
fn the_release_build_reads_the_permissions_back_off_the_apk() {
    // Everything above reads source files. None of it proves the merger actually folded the
    // plugin's declaration into the APK — the merged manifest exists only after
    // `cargo tauri android build`, in the one artifact a customer installs. The release job
    // already reads the app id back off that APK with `aapt dump badging` (ADR-0160); the
    // permissions get the same treatment, because "the docs say the merger does it" is not a test.
    //
    // For this to bite, the job that runs it has to watch the file it reads: `test-shell.yml`
    // lists `tauri-release.yml` in its `paths` filter for exactly that reason.
    assert!(
        RELEASE_WORKFLOW.contains("dump permissions"),
        "the release job never asks the built APK which permissions it declares, so a merger that \
         quietly dropped them would ship"
    );
    for permission in REQUIRED {
        assert!(
            RELEASE_WORKFLOW.contains(permission),
            "the release job does not check {permission} on the built APK"
        );
    }
    // And the features those permissions drag in with them (NFC, location). Invisible everywhere
    // else: an implicit hardware requirement breaks no build and fails no test — it just makes
    // Play stop offering the app to devices without that hardware, months later, silently.
    // Matched as the job's own regex (`: ?name=`) because aapt1 prints
    // `uses-feature-not-required:name=` and aapt2 puts a space after the colon, so the step
    // tolerates both. What must not disappear is the CHECK, one per feature.
    assert!(
        RELEASE_WORKFLOW.contains("uses-feature-not-required"),
        "the release job never reads back which features the built APK declares OPTIONAL"
    );
    for feature in OPTIONAL_FEATURES {
        assert!(
            RELEASE_WORKFLOW.contains(&format!("uses-feature-not-required: ?name='{feature}'")),
            "the release job never reads back whether the built APK still says {feature} is \
             OPTIONAL, so a merged manifest that requires it would ship and narrow the listing \
             (hub#988, hub#2552)"
        );
    }
}

/// The location pair, on its own: the one `attendance` needs to geofence a clock-in from a
/// personal phone. Without it the WebView answers `PERMISSION_DENIED` with no dialog.
// Regression test for ERPlora/hub#2552
#[test]
fn the_location_permissions_are_declared_in_both_manifests_hub2552() {
    let plugin = fs::read_to_string(plugin_manifest_path()).expect("the plugin manifest");
    for (label, manifest) in [("gen/android", APP_MANIFEST), ("plugin", plugin.as_str())] {
        let declared = declared_permissions(manifest);
        for permission in [
            "android.permission.ACCESS_FINE_LOCATION",
            "android.permission.ACCESS_COARSE_LOCATION",
        ] {
            assert!(
                declared.iter().any(|p| p == permission),
                "{label} manifest does not declare {permission} (hub#2552)"
            );
        }
    }
}
