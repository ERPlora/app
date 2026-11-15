//! The question the app asks before a link links another hub.

use serde::{Deserialize, Serialize};
use tauri::Runtime;

use crate::{Error, ErploraAndroid};

/// The question the app asks before a link links another hub, answered by an
/// `AlertDialog`. Mirror of `ErploraAndroidPlugin.askToOpenHub`; a test below checks the two never
/// drift apart. Only Rust calls it: it is in neither `build.rs` nor the default permissions, so no
/// page can show the question, let alone answer it.
pub const ASK_TO_OPEN_HUB_COMMAND: &str = "askToOpenHub";

/// The words of that question in one language: written by the app, shown by Kotlin as they come.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Wording {
    pub title: String,
    pub message: String,
    pub open: String,
    pub cancel: String,
}

/// Both languages travel: Kotlin picks the device's (Spanish unless it says otherwise).
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[derive(Debug, Serialize)]
struct AskToOpenHubArgs<'a> {
    en: &'a Wording,
    es: &'a Wording,
}

/// What `askToOpenHub` answers: `open` is «Open». An answer without it is a no.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[derive(Debug, Deserialize)]
struct AskToOpenHubAnswer {
    #[serde(default)]
    open: bool,
}

impl<R: Runtime> ErploraAndroid<R> {
    /// Asks the person «Open <hub> on this device?» in an `AlertDialog` before a link links another
    /// hub; `true` only for «Open». Both languages travel and the device's is shown.
    ///
    /// ⚠️ **Blocks** until the person answers: call it from a thread of its own.
    pub fn ask_to_open_hub(&self, en: &Wording, es: &Wording) -> Result<bool, Error> {
        #[cfg(target_os = "android")]
        {
            return self
                .0
                .run_mobile_plugin::<AskToOpenHubAnswer>(
                    ASK_TO_OPEN_HUB_COMMAND,
                    AskToOpenHubArgs { en, es },
                )
                .map(|answer| answer.open)
                .map_err(|e| Error::PluginInvoke(e.to_string()));
        }
        #[cfg(not(target_os = "android"))]
        {
            // Unreachable by construction: the desktop asks with its own dialog. A no all the same.
            let _ = (en, es);
            Err(Error::PluginInvoke("ask_to_open_hub is Android only".into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    // ── The question before a link links another hub ───────────────────────────────────────

    fn question_in(language: &str) -> Wording {
        Wording {
            title: format!("title-{language}"),
            message: format!("message-{language}"),
            open: format!("open-{language}"),
            cancel: format!("cancel-{language}"),
        }
    }

    #[test]
    fn the_hub_question_crosses_to_kotlin_in_both_languages_under_the_keys_kotlin_reads() {
        let (en, es) = (question_in("en"), question_in("es"));
        let json = serde_json::to_value(AskToOpenHubArgs { en: &en, es: &es }).expect("serializable");
        for language in ["en", "es"] {
            for key in ["title", "message", "open", "cancel"] {
                assert_eq!(json[language][key], format!("{key}-{language}"), "{language}.{key}");
            }
        }
        let code = without_kotlin_comments(ERPLORA_ANDROID_PLUGIN_KT);
        let mut literals = vec![
            format!("fun {ASK_TO_OPEN_HUB_COMMAND}(invoke: Invoke)"),
            "put(\"open\"".to_string(),
        ];
        literals.extend(["title", "message", "open", "cancel"].map(|key| format!("getString(\"{key}\"")));
        for literal in literals {
            assert!(
                code.contains(&literal),
                "{literal} is not in ErploraAndroidPlugin.kt — a link to another hub would never be asked \
                 about on a real device"
            );
        }
        let languages = without_kotlin_comments(HUB_QUESTION_KT);
        for literal in ["\"es\"", "\"en\""] {
            assert!(languages.contains(literal), "{literal} is not in HubQuestion.kt");
        }
    }

    const HUB_QUESTION_KT: &str =
        include_str!("../android/src/main/java/com/erplora/android/HubQuestion.kt");

    #[test]
    fn only_an_explicit_open_opens() {
        let read = |answer| serde_json::from_value::<AskToOpenHubAnswer>(answer).map(|a| a.open);
        assert_eq!(read(serde_json::json!({ "open": true })).ok(), Some(true));
        assert_eq!(read(serde_json::json!({ "open": false })).ok(), Some(false));
        assert_eq!(read(serde_json::json!({})).ok(), Some(false), "an answer without `open` linked the hub");
    }

    #[test]
    fn no_page_can_show_or_answer_the_hub_question() {
        // Only Rust asks: a command in `build.rs` gets an `allow-*` permission a capability could
        // grant, and the page is exactly who must not be able to answer.
        let build = include_str!("../build.rs");
        let permissions = include_str!("../permissions/default.toml");
        for name in ["ask_to_open_hub", "ask-to-open-hub", ASK_TO_OPEN_HUB_COMMAND] {
            assert!(!build.contains(name), "build.rs declares {name}");
            assert!(!permissions.contains(name), "the default permissions grant {name}");
        }
    }
}
