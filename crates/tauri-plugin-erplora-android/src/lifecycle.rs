//! Leaving the app the way the system Back does, and keeping it alive with the screen off.

use serde::Serialize;
use tauri::Runtime;

#[cfg(target_os = "android")]
use crate::Empty;
use crate::{Error, ErploraAndroid};

/// What the page asks of the listening service: `on`, and the words of the ongoing notification in
/// the app's language (the translations live with the page). Off, no words.
#[derive(Debug, Serialize)]
pub(crate) struct KeepListeningArgs {
    pub(crate) on: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) channel: Option<String>,
}

/// `keepListening` resolves with no data (`invoke.resolve()` → `null`): read it as anything.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
type KeepListeningAnswer = serde::de::IgnoredAny;

/// `leaveApp` resolves with no data (`invoke.resolve()` → `null`): read it as anything.
type LeaveAppAnswer = serde::de::IgnoredAny;

impl<R: Runtime> ErploraAndroid<R> {
    /// Sends the app to the background, as the system Back does on a root screen.
    pub fn leave_app(&self) -> Result<(), Error> {
        #[cfg(target_os = "android")]
        {
            return self
                .0
                .run_mobile_plugin::<LeaveAppAnswer>("leaveApp", Empty {})
                .map(|_| ())
                .map_err(|e| Error::PluginInvoke(e.to_string()));
        }
        #[cfg(not(target_os = "android"))]
        Ok(())
    }

    /// Keeps the app running with the screen off (a foreground service and
    /// its ongoing notification) or lets Android reclaim it again. Every notice is born in the page,
    /// so this is what makes them arrive while nobody looks at the device. On desktop the app is not
    /// reclaimed.
    pub(crate) fn keep_listening(&self, args: KeepListeningArgs) -> Result<(), Error> {
        #[cfg(target_os = "android")]
        {
            return self
                .0
                .run_mobile_plugin::<KeepListeningAnswer>("keepListening", args)
                .map(|_| ())
                .map_err(|e| Error::PluginInvoke(e.to_string()));
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = args;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    /// Once the shell takes the Android Back button (Tauri's `onBackButtonPress`),
    /// Tauri no longer leaves the app on its own when the WebView has no history left: the shell
    /// has to ask for it, and Tauri's own `exit` has no ACL permission to grant. `leave_app` is that
    /// door. Four places have to agree or the press dies in silence: the build (which generates the
    /// permission), the invoke handler, the default permission set the hub capability grants, and
    /// the Kotlin command that actually leaves.
    #[test]
    fn leave_app_is_declared_wired_granted_and_implemented() {
        assert!(BUILD_RS.contains("\"leave_app\""), "build.rs does not declare leave_app: no permission is generated");
        // Only the code above the tests: this very assertion spells the handler too.
        let production = LIB_RS.split("#[cfg(test)]").next().unwrap_or_default();
        assert!(
            production
                .split("generate_handler![")
                .nth(1)
                .and_then(|rest| rest.split(']').next())
                .is_some_and(|handler| handler.split(',').any(|c| c.trim() == "leave_app")),
            "leave_app is not wired to the invoke handler"
        );
        assert!(
            DEFAULT_PERMISSIONS.contains("\"allow-leave-app\""),
            "erplora-android:default does not grant allow-leave-app: the hub's capability would refuse it"
        );
        let kotlin = PLUGIN_KT.split("fun leaveApp(invoke: Invoke)").nth(1).expect("no Kotlin leaveApp command");
        let before = PLUGIN_KT.split("fun leaveApp(invoke: Invoke)").next().unwrap_or_default();
        assert!(before.trim_end().ends_with("@Command"), "Kotlin leaveApp is not a @Command");
        let body = kotlin.split("\n    }").next().unwrap_or_default();
        assert!(body.contains("moveTaskToBack(true)"), "leaveApp does not leave the way the system Back does");
        assert!(body.contains("invoke.resolve()"), "leaveApp never answers: the web would wait forever");
    }

    // ── Listening with the screen off ─────────────────────────────────────────────────────────
    //
    // The notices come out of the page (the bell's poll, the event socket), so they only arrive while
    // Android keeps the app running. A foreground service is what keeps it: six places have to agree
    // on it and none of them is seen by the compiler — the build (the permission), the handler, the
    // default set the hub's capability grants, the Kotlin command, the service class and the manifest
    // that declares it with the type and permissions Android 14 demands. Any one missing and the app
    // behaves exactly as before, with every other test green.

    /// The service and its decisions, read when the test runs: without the file the tests below say
    /// so instead of the crate not building.
    fn notice_listening_kt() -> String {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/android/src/main/java/com/erplora/android/NoticeListening.kt");
        std::fs::read_to_string(path).unwrap_or_else(|_| panic!("there is no NoticeListening.kt: nothing keeps the app listening"))
    }

    #[test]
    fn keep_listening_is_declared_wired_granted_and_implemented() {
        assert!(BUILD_RS.contains("\"keep_listening\""), "build.rs does not declare keep_listening");
        let production = production_rust();
        let handler = production
            .split("generate_handler![")
            .nth(1)
            .and_then(|rest| rest.split(']').next())
            .expect("no invoke handler");
        assert!(
            handler.split(',').any(|c| c.trim() == "keep_listening"),
            "keep_listening is not wired to the invoke handler"
        );
        assert!(
            production.contains("run_mobile_plugin::<KeepListeningAnswer>(\"keepListening\", args)"),
            "Rust does not reach Kotlin under the name Kotlin answers to"
        );
        // On a computer the command answers `Ok` whatever it is handed, so no test can see what
        // reaches Kotlin: with `on` or one of the words dropped here, the tablet never listens (or
        // never stops) and every other test stays green.
        assert!(
            production.contains("keep_listening(KeepListeningArgs { on, title, body, channel })"),
            "the command does not hand Kotlin the order and the words exactly as the page sent them"
        );
        assert!(
            DEFAULT_PERMISSIONS.contains("\"allow-keep-listening\""),
            "erplora-android:default does not grant allow-keep-listening: the hub's capability would refuse it"
        );
        let signature = "fun keepListening(invoke: Invoke)";
        let before = PLUGIN_KT.split(signature).next().unwrap_or_default();
        assert!(before.trim_end().ends_with("@Command"), "Kotlin keepListening is not a @Command");
        let body = PLUGIN_KT
            .split(signature)
            .nth(1)
            .and_then(|rest| rest.split("\n    }").next())
            .expect("no Kotlin keepListening command");
        assert!(body.contains("NoticeListeningService.start("), "keepListening never starts the service");
        assert!(body.contains("NoticeListeningService.stop("), "keepListening never stops the service");
        assert!(body.contains("invoke.resolve()"), "keepListening never answers: the web would wait forever");
        assert!(body.contains("invoke.reject("), "keepListening swallows a refusal the page must hear");
    }

    #[test]
    fn the_listening_request_crosses_to_kotlin_under_the_keys_kotlin_reads() {
        let on = serde_json::to_value(KeepListeningArgs {
            on: true,
            title: Some("t".into()),
            body: Some("b".into()),
            channel: Some("c".into()),
        })
        .expect("serializes");
        assert_eq!(on, serde_json::json!({ "on": true, "title": "t", "body": "b", "channel": "c" }));
        let off = serde_json::to_value(KeepListeningArgs { on: false, title: None, body: None, channel: None })
            .expect("serializes");
        assert_eq!(off, serde_json::json!({ "on": false }));
        for key in ["\"on\"", "\"title\"", "\"body\"", "\"channel\""] {
            assert!(PLUGIN_KT.contains(key), "Kotlin does not read {key} from the request");
        }
    }

    #[test]
    fn the_app_is_not_kept_listening_once_its_page_is_gone() {
        // The service keeps the PROCESS; the page is what listens. With the activity destroyed or the
        // app swiped away, a notification saying «listening» would be a promise nothing keeps.
        let on_destroy = PLUGIN_KT
            .split("override fun onDestroy(activity: AppCompatActivity)")
            .nth(1)
            .and_then(|rest| rest.split("\n    }").next())
            .expect("the plugin does not stop anything when the activity is destroyed");
        assert!(on_destroy.contains("NoticeListeningService.stop("), "onDestroy leaves the service running");
        let notice_listening_kt = notice_listening_kt();
        let task_removed = notice_listening_kt
            .split("override fun onTaskRemoved(")
            .nth(1)
            .and_then(|rest| rest.split("\n    }").next())
            .expect("the service does not react to the app being swiped away");
        assert!(task_removed.contains("stopSelf()"), "swiping the app away leaves the service running");
        assert!(
            notice_listening_kt.contains("return NoticeListening.START_MODE"),
            "onStartCommand does not answer with the pinned start mode"
        );
    }

    #[test]
    fn the_page_is_kept_running_exactly_while_the_app_listens() {
        // The service keeps the process, `PageKeeper` keeps the page: without it Chromium freezes
        // the hidden page at 60 s and no notice is born, with the service and its notification in
        // place (measured on the device). What it does has its own test in Kotlin
        // (`PageKeeperTest`); what only lives in the plugin is WHEN it is told — installed with the
        // WebView, on once the service started, off with every way the listening ends.
        let kotlin_fn = |signature: &str| {
            PLUGIN_KT
                .split(signature)
                .nth(1)
                .and_then(|rest| rest.split("\n    }").next())
                .unwrap_or_else(|| panic!("the plugin has no `{signature}`"))
        };
        let load = kotlin_fn("override fun load(webView: WebView)");
        assert!(load.contains("PageKeeper(webView)"), "no keeper is built for the WebView");
        assert!(load.contains("keeper.install(root, owner)"), "the keeper never hears the app leave the screen");
        assert!(load.contains("pageKeeper = keeper"), "the commands cannot reach the keeper");

        let command = kotlin_fn("fun keepListening(invoke: Invoke)");
        let (off, on) = command
            .split_once("NoticeListening.textsOf(")
            .expect("keepListening does not read the words of the notification");
        assert!(off.contains("pageKeeper?.setListening(false)"), "told to stop, the page is still kept running");
        let started = on
            .split_once("NoticeListeningService.start(")
            .map(|(_, after)| after)
            .and_then(|after| after.split("catch").next())
            .expect("keepListening never starts the service");
        assert!(
            started.contains("pageKeeper?.setListening(true)"),
            "the service starts and the page still freezes: the keeper is never told to listen"
        );
        assert!(
            !on.split("NoticeListeningService.start(").next().unwrap_or_default().contains("setListening(true)"),
            "the page is kept running before the service is known to have started"
        );

        let on_destroy = kotlin_fn("override fun onDestroy(activity: AppCompatActivity)");
        assert!(
            on_destroy.contains("pageKeeper?.setListening(false)"),
            "the activity is gone and its page is still kept running"
        );
    }

    #[test]
    fn the_listening_service_is_declared_where_no_generator_can_drop_it() {
        let manifest = without_comments(PLUGIN_MANIFEST);
        let declared = declared_permissions(PLUGIN_MANIFEST);
        for permission in [
            "android.permission.FOREGROUND_SERVICE",
            "android.permission.FOREGROUND_SERVICE_SPECIAL_USE",
        ] {
            assert!(declared.iter().any(|p| p == permission), "{permission} is not declared: startForeground throws");
        }
        let service = manifest
            .split("<service")
            .nth(1)
            .and_then(|rest| rest.split("</service>").next())
            .expect("no <service> in the plugin manifest");
        assert!(
            service.contains("android:name=\"com.erplora.android.NoticeListeningService\""),
            "the declared service is not NoticeListeningService"
        );
        assert!(
            service.contains("android:foregroundServiceType=\"specialUse\""),
            "the service does not declare the type Android 14 demands"
        );
        assert!(service.contains("android:exported=\"false\""), "another app could start or stop the service");
        assert!(
            service.contains("android.app.PROPERTY_SPECIAL_USE_FGS_SUBTYPE"),
            "a specialUse service without its subtype is refused by Google Play's review"
        );
    }
}
