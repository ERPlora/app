//! A tab of the device's browser over the app, for the browser half of «Continue with Google».

use serde::Serialize;
use tauri::Runtime;

use crate::{Error, ErploraAndroid};

/// Mirror of `ErploraAndroidPlugin.openCustomTab`. Only Rust calls it: it is in neither `build.rs` nor
/// the default permissions, so no page can open a tab.
pub const OPEN_CUSTOM_TAB_COMMAND: &str = "openCustomTab";

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[derive(Debug, Serialize)]
struct OpenCustomTabArgs<'a> {
    url: &'a str,
}

impl<R: Runtime> ErploraAndroid<R> {
    /// Opens `url` in a tab of the device's browser, over the app.
    pub fn open_custom_tab(&self, url: &str) -> Result<(), Error> {
        #[cfg(target_os = "android")]
        {
            return self
                .0
                .run_mobile_plugin::<serde::de::IgnoredAny>(OPEN_CUSTOM_TAB_COMMAND, OpenCustomTabArgs { url })
                .map(|_| ())
                .map_err(|e| Error::PluginInvoke(e.to_string()));
        }
        #[cfg(not(target_os = "android"))]
        {
            // The window never sends the sign-in to the browser there; the caller opens the browser.
            let _ = url;
            Err(Error::PluginInvoke("a browser tab over the app is Android only".into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    const CUSTOM_TAB_KT: &str = include_str!("../android/src/main/java/com/erplora/android/CustomTab.kt");

    #[test]
    fn the_address_crosses_to_kotlin_under_the_key_kotlin_reads() {
        let args = serde_json::to_value(OpenCustomTabArgs { url: "https://erplora.com/auth/app/google/" }).expect("serializable");
        assert_eq!(args, serde_json::json!({ "url": "https://erplora.com/auth/app/google/" }));
        assert!(
            without_kotlin_comments(ERPLORA_ANDROID_PLUGIN_KT).contains(&format!("fun {OPEN_CUSTOM_TAB_COMMAND}(invoke: Invoke)")),
            "ErploraAndroidPlugin.kt has no `{OPEN_CUSTOM_TAB_COMMAND}`: Google's sign-in would never reach the browser"
        );
        let kotlin = without_kotlin_comments(CUSTOM_TAB_KT);
        for literal in ["getString(\"url\"", "CustomTabsIntent"] {
            assert!(kotlin.contains(literal), "{literal} is not in CustomTab.kt");
        }
    }

    #[test]
    fn no_page_can_open_a_tab() {
        for name in ["open_custom_tab", "open-custom-tab", OPEN_CUSTOM_TAB_COMMAND] {
            assert!(!BUILD_RS.contains(name), "build.rs declares {name}");
            assert!(!DEFAULT_PERMISSIONS.contains(name), "the default permissions grant {name}");
        }
    }
}
