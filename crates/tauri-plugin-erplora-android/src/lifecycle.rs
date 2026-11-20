//! Leaving the app the way the system Back does.

use tauri::Runtime;

#[cfg(target_os = "android")]
use crate::Empty;
use crate::{Error, ErploraAndroid};

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
}

#[cfg(test)]
mod tests {
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
}
