//! The runtime permissions the till needs, and the device settings page where a refused one is turned back on.

use serde::Serialize;
use tauri::Runtime;

#[cfg(target_os = "android")]
use crate::Empty;
use crate::{Error, ErploraAndroid};

/// A permission's state as the page sees it: `{ "android.permission.X": true }`.
pub type PermissionStatus = std::collections::HashMap<String, bool>;

/// Local network access (API 37+). Without it the printer sweep is 254 silent timeouts.
///
/// Mirror of `PermissionPolicy.ACCESS_LOCAL_NETWORK` on the Kotlin side. The key of the map above
/// **is** the permission string, so Rust needs the same literal to read the answer — and a test
/// below checks the two never drift apart.
pub const ACCESS_LOCAL_NETWORK: &str = "android.permission.ACCESS_LOCAL_NETWORK";

/// System notifications (API 33+). Mirror of `PermissionPolicy.POST_NOTIFICATIONS`.
pub const POST_NOTIFICATIONS: &str = "android.permission.POST_NOTIFICATIONS";

/// Talking to bonded Bluetooth devices (API 31+): the SPP printer transport.
/// Mirror of `PermissionPolicy.BLUETOOTH_CONNECT`. Denied, the bonded list comes back empty and
/// the RFCOMM connect throws — the same silent-failure family as the two above.
pub const BLUETOOTH_CONNECT: &str = "android.permission.BLUETOOTH_CONNECT";

/// The scope of a `request_permissions` call: the permissions the OPERATION that asks is about to
/// use, or `None` when the caller sent none (a web older than the scope), which keeps asking for
/// everything so the app and the web, shipped separately, cannot break each other.
#[derive(Debug, Serialize)]
struct RequestPermissionsArgs {
    permissions: Option<Vec<String>>,
}

/// `openAppSettings` resolves with no data (`invoke.resolve()` → `null`): read it as anything.
#[cfg(target_os = "android")]
type OpenAppSettingsAnswer = serde::de::IgnoredAny;

impl<R: Runtime> ErploraAndroid<R> {
    /// Opens this app's page in the device settings, the only place left to turn a
    /// permission on once Android stops showing its dialog. On desktop there is no such page.
    pub fn open_app_settings(&self) -> Result<(), Error> {
        #[cfg(target_os = "android")]
        {
            return self
                .0
                .run_mobile_plugin::<OpenAppSettingsAnswer>("openAppSettings", Empty {})
                .map(|_| ())
                .map_err(|e| Error::PluginInvoke(e.to_string()));
        }
        #[cfg(not(target_os = "android"))]
        Ok(())
    }

    /// The permissions granted right now. Always empty on desktop: there is nothing to grant.
    pub fn check_permissions(&self) -> Result<PermissionStatus, Error> {
        #[cfg(target_os = "android")]
        {
            return self
                .0
                .run_mobile_plugin("checkPermissions", Empty {})
                .map_err(|e| Error::PluginInvoke(e.to_string()));
        }
        #[cfg(not(target_os = "android"))]
        Ok(PermissionStatus::new())
    }

    /// Asks for what is missing, scoped to what the calling operation is about to use.
    /// `None` = no scope sent (an older web): the whole batch. Idempotent: if
    /// everything in scope is granted, no dialog.
    pub fn request_permissions(
        &self,
        permissions: Option<Vec<String>>,
    ) -> Result<PermissionStatus, Error> {
        #[cfg(target_os = "android")]
        {
            return self
                .0
                .run_mobile_plugin("requestPermissions", RequestPermissionsArgs { permissions })
                .map_err(|e| Error::PluginInvoke(e.to_string()));
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = permissions;
            Ok(PermissionStatus::new())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    /// Once Android stops showing a permission dialog, the only way back is the app's
    /// page in the device settings, and the shell offers a button that takes the owner there.
    /// Four places have to agree or the tap dies in silence on a real device only: the build
    /// (which generates the permission), the invoke handler, the default permission set the hub
    /// capability grants, and the Kotlin command that opens the page.
    const PLUGIN_KT_SOURCE: &str =
        include_str!("../android/src/main/java/com/erplora/android/ErploraAndroidPlugin.kt");
    const BUILD_RS_SOURCE: &str = include_str!("../build.rs");
    const LIB_RS_SOURCE: &str = include_str!("lib.rs");
    const DEFAULT_PERMISSIONS_TOML: &str = include_str!("../permissions/default.toml");

    #[test]
    fn open_app_settings_is_declared_wired_granted_and_implemented() {
        assert!(
            BUILD_RS_SOURCE.contains("\"open_app_settings\""),
            "build.rs does not declare open_app_settings: no permission is generated"
        );
        // Only the code above the tests: this very assertion spells the command too.
        let production = LIB_RS_SOURCE.split("#[cfg(test)]").next().unwrap_or_default();
        let handler = production
            .split("generate_handler![")
            .nth(1)
            .and_then(|rest| rest.split(']').next())
            .expect("no invoke handler");
        assert!(
            handler.split(',').any(|c| c.trim() == "open_app_settings"),
            "open_app_settings is not wired to the invoke handler"
        );
        assert!(
            DEFAULT_PERMISSIONS_TOML.contains("\"allow-open-app-settings\""),
            "erplora-android:default does not grant allow-open-app-settings: the hub's capability \
             would refuse it"
        );
        let signature = "fun openAppSettings(invoke: Invoke)";
        let before = PLUGIN_KT_SOURCE.split(signature).next().unwrap_or_default();
        assert!(before.trim_end().ends_with("@Command"), "Kotlin openAppSettings is not a @Command");
        let body = PLUGIN_KT_SOURCE
            .split(signature)
            .nth(1)
            .and_then(|rest| rest.split("\n    }").next())
            .expect("no Kotlin openAppSettings command");
        assert!(
            body.contains("Settings.ACTION_APPLICATION_DETAILS_SETTINGS"),
            "openAppSettings does not open the app's own page in the device settings"
        );
        assert!(
            body.contains("Uri.fromParts(\"package\", activity.packageName, null)"),
            "openAppSettings does not point the settings at THIS app"
        );
        assert!(body.contains("invoke.resolve()"), "openAppSettings never answers: the web would wait forever");
        assert!(
            body.contains("invoke.reject("),
            "openAppSettings swallows a device with no settings page: the web must hear it to fall back"
        );
    }

    /// wry hands the microphone and the position to WHATEVER page asks. The
    /// origin check is `WebPermissionGate.kt` (and its JUnit tests), and it only bites if the plugin
    /// puts it in front of wry's client: from `load` (which Tauri calls from wry's
    /// `on_webview_created`, after `setWebChromeClient`), through `post`, so it finds the client
    /// whatever that order becomes.
    const WEB_PERMISSION_GATE_KT: &str =
        include_str!("../android/src/main/java/com/erplora/android/WebPermissionGate.kt");

    /// The Rust half of the app, which decides «one of ours» for the navigation and the hub link.
    const SHELL_LIB_RS: &str = include_str!("../../../src-tauri/src/lib.rs");

    #[test]
    fn the_webview_permissions_are_guarded_by_origin_from_load() {
        let load = PLUGIN_KT
            .split("override fun load(webView: WebView)")
            .nth(1)
            .and_then(|rest| rest.split("\n    }").next())
            .expect("no Kotlin load(webView)");
        assert!(
            load.contains("webView.post { WebPermissionGate.install(webView) }"),
            "the plugin never puts the origin check in front of wry's WebChromeClient: once granted, \
             any page the window loads gets the microphone and the position"
        );
        // One domain on both halves: a hub the Rust side trusts must keep its microphone.
        assert!(SHELL_LIB_RS.contains("const HUB_DOMAIN_SUFFIX: &str = \".erplora.com\";"));
        assert!(WEB_PERMISSION_GATE_KT.contains("const val DOMAIN = \"erplora.com\""));
        // And the install only reads wry's client where the WebView can hand it back:
        // `WebViewCompat.getWebChromeClient` THROWS on API 24-25 with the factory WebView, and from
        // `post` that is a crash at start (`clientFor` and its JUnit test decide; this is the wiring).
        let install = WEB_PERMISSION_GATE_KT
            .split("fun install(webView: WebView)")
            .nth(1)
            .and_then(|rest| rest.split("\n    }").next())
            .expect("no Kotlin install(webView)");
        assert!(
            install.contains("val canRead = WebViewFeature.isFeatureSupported(WebViewFeature.GET_WEB_CHROME_CLIENT)")
                && install.contains("clientFor(canRead)"),
            "install reads the WebChromeClient without asking whether this WebView can hand it back: \
             on Android 7 with its factory WebView that throws inside `post` and kills the app at start"
        );
    }

    #[test]
    fn the_status_serializes_as_a_map_of_permission_to_bool() {
        // The contract the page reads: `{ "android.permission.X": true }`.
        let mut estado = PermissionStatus::new();
        estado.insert("android.permission.POST_NOTIFICATIONS".into(), true);
        let json = serde_json::to_string(&estado).unwrap();
        assert!(json.contains("\"android.permission.POST_NOTIFICATIONS\":true"));
    }

    #[test]
    fn rust_and_kotlin_spell_the_permissions_the_same_way() {
        for permission in [ACCESS_LOCAL_NETWORK, POST_NOTIFICATIONS, BLUETOOTH_CONNECT] {
            assert!(
                PERMISSION_POLICY_KT.contains(permission),
                "{permission} is not in PermissionPolicy.kt — the status map would never mention it"
            );
        }
    }

    /// Every `android.permission.*` the Kotlin policy names, read out of its source.
    ///
    /// Read rather than listed so the guards below cannot drift from the one place that decides
    /// what the shell ASKS for: a permission added there and forgotten everywhere else is the bug
    /// this file is about.
    fn permissions_the_plugin_can_ask_for() -> Vec<&'static str> {
        PERMISSION_POLICY_KT
            .match_indices("\"android.permission.")
            .filter_map(|(at, _)| PERMISSION_POLICY_KT[at + 1..].split('"').next())
            .collect()
    }

    /// Asking for a permission the merged manifest never declared is not a dialog the user can
    /// say yes to: Android answers DENIED at once, forever, and shows nothing. The till would
    /// then tell the user to grant local network access in the system settings and send
    /// them looking for a toggle that does not exist.
    ///
    /// The plugin declares them in its OWN Android library manifest, which the manifest merger
    /// folds into the app at build time. That is what makes the declaration survive
    /// `cargo tauri android init` — the generator rewrites `src-tauri/gen/android`, it does not
    /// reach into `crates/`.
    #[test]
    fn the_plugin_declares_every_permission_it_can_ask_for() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/android/src/main/AndroidManifest.xml");
        let manifest = std::fs::read_to_string(path).unwrap_or_else(|e| {
            panic!(
                "{path}: {e}\n\
                 The plugin ships no Android manifest, so these permissions exist only in the \
                 GENERATED gen/android project. Lose that file and `cargo tauri android init` \
                 writes Tauri's template back — INTERNET and nothing else — and the till stops \
                 finding printers with a perfectly green build."
            )
        });

        let declared = declared_permissions(&manifest);
        let asked_for = permissions_the_plugin_can_ask_for();
        assert!(!asked_for.is_empty(), "PermissionPolicy.kt names no permission at all");
        for permission in asked_for {
            assert!(
                declared.iter().any(|d| d == permission),
                "PermissionPolicy asks for {permission}, but no manifest of this plugin declares \
                 it: Android would answer that request DENIED without ever showing a dialog.\n\
                 Declared right now: {declared:?}"
            );
        }
    }

    /// The scope of a permission request has to reach Kotlin under the key Kotlin reads.
    /// If either side renamed it, the argument would silently vanish and the plugin
    /// would fall back to requesting its whole batch — the exact out-of-context notifications
    /// dialog this contract exists to end, with no error anywhere.
    #[test]
    fn the_permission_scope_travels_under_the_key_kotlin_reads() {
        let args = RequestPermissionsArgs {
            permissions: Some(vec![ACCESS_LOCAL_NETWORK.to_string()]),
        };
        let json = serde_json::to_value(&args).expect("serializable");
        assert_eq!(json["permissions"][0], ACCESS_LOCAL_NETWORK);

        const PLUGIN_KT: &str =
            include_str!("../android/src/main/java/com/erplora/android/ErploraAndroidPlugin.kt");
        assert!(
            PLUGIN_KT.contains("optJSONArray(\"permissions\")"),
            "ErploraAndroidPlugin.kt no longer reads the \"permissions\" scope: every request \
             would go back to asking for the whole batch"
        );
    }

    /// An absent scope must serialize as an ABSENT key, not as `null`: Kotlin's `optJSONArray`
    /// treats both as "no scope", but the wire contract is "an old web sends nothing", and
    /// pinning it keeps the fallback path honest.
    #[test]
    fn no_scope_still_serializes_and_means_the_whole_batch() {
        let args = RequestPermissionsArgs { permissions: None };
        let json = serde_json::to_value(&args).expect("serializable");
        assert!(json["permissions"].is_null());
    }
}
