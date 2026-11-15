//! The system voice (Android's `TextToSpeech`) the assistant reads aloud with.

use serde::{Deserialize, Serialize};
use tauri::Runtime;

#[cfg(target_os = "android")]
use crate::Empty;
use crate::{Error, ErploraAndroid};

/// The system voice has no engine, or no voice for the language: the page reads
/// with the browser's voice if it has one. Mirror of `SystemVoice.TTS_UNAVAILABLE`.
pub const TTS_UNAVAILABLE: &str = "tts_unavailable";

/// The system voice broke while reading. Mirror of `SystemVoice.TTS_FAILED`.
pub const TTS_FAILED: &str = "tts_failed";

/// What the assistant asks the system voice to read: the text and its language
/// as a BCP 47 tag with its region (`es-ES`, `en-US`).
#[derive(Debug, Serialize)]
pub(crate) struct TtsSpeakArgs {
    pub(crate) text: String,
    pub(crate) language: String,
}

/// The language the page asks the system voice about.
#[derive(Debug, Serialize)]
pub(crate) struct TtsLanguageArgs {
    pub(crate) language: String,
}

/// Whether the system voice can read a language: `{ "available": bool }`, the same both ways.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TtsAvailability {
    #[serde(default)]
    pub available: bool,
}

/// `ttsSpeak` and `ttsStop` resolve with no data (`invoke.resolve()` → `null`): read it as anything.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
type TtsAnswer = serde::de::IgnoredAny;

/// Reads the rejection Kotlin sent back (`[code] - message`, as for NFC) and decides which refusal
/// of the system voice it is: the page reads with another voice on `tts_unavailable` only.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn classify_tts_error(message: &str) -> Error {
    if message.contains(TTS_UNAVAILABLE) {
        Error::TtsUnavailable
    } else if message.contains(TTS_FAILED) {
        Error::TtsFailed
    } else {
        Error::PluginInvoke(message.to_string())
    }
}

impl<R: Runtime> ErploraAndroid<R> {
    /// Whether the system voice (Android's `TextToSpeech`) can read the language.
    /// The first call makes the engine and waits for it to start. On desktop, never: Windows'
    /// WebView2 reads with the page's own `speechSynthesis`, which is where `false` sends the page.
    ///
    /// ⚠️ **Blocks** until the engine answers: call it off the async workers (`spawn_blocking`).
    pub(crate) fn tts_available(&self, args: TtsLanguageArgs) -> Result<bool, Error> {
        #[cfg(target_os = "android")]
        {
            return self
                .0
                .run_mobile_plugin::<TtsAvailability>("ttsAvailable", args)
                .map(|answer| answer.available)
                .map_err(|e| classify_tts_error(&e.to_string()));
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = args;
            Ok(false)
        }
    }

    /// Reads the text aloud in the system voice, ending what it was reading.
    /// Answers once the whole text was read or stopped; refuses with [`Error::TtsUnavailable`] (no
    /// engine or no voice for the language) or [`Error::TtsFailed`]. On desktop there is no system
    /// voice to read with here: the page reads with its own.
    ///
    /// ⚠️ **Blocks** for the whole reading: call it off the async workers (`spawn_blocking`).
    pub(crate) fn tts_speak(&self, args: TtsSpeakArgs) -> Result<(), Error> {
        #[cfg(target_os = "android")]
        {
            return self
                .0
                .run_mobile_plugin::<TtsAnswer>("ttsSpeak", args)
                .map(|_| ())
                .map_err(|e| classify_tts_error(&e.to_string()));
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = args;
            Err(Error::TtsUnavailable)
        }
    }

    /// Stops the system voice; what was being read answers as ended.
    pub(crate) fn tts_stop(&self) -> Result<(), Error> {
        #[cfg(target_os = "android")]
        {
            return self
                .0
                .run_mobile_plugin::<TtsAnswer>("ttsStop", Empty {})
                .map(|_| ())
                .map_err(|e| Error::PluginInvoke(e.to_string()));
        }
        #[cfg(not(target_os = "android"))]
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    // ── The system voice ───────────────────────────────────────────────────────────────────────

    /// The Android WebView has no `speechSynthesis`, so the assistant's «Read aloud» reads through
    /// these three commands in the app. Five places have to agree or the assistant stays mute on a
    /// real device only: the build (which generates the permissions), the invoke handler, the
    /// default set the hub capability grants, the name Rust calls Kotlin by and the Kotlin command.
    const SYSTEM_VOICE_KT: &str = include_str!("../android/src/main/java/com/erplora/android/SystemVoice.kt");

    #[test]
    fn the_system_voice_commands_are_declared_wired_granted_and_implemented() {
        let production = production_rust();
        let handler = production
            .split("generate_handler![")
            .nth(1)
            .and_then(|rest| rest.split(']').next())
            .expect("no invoke handler");
        for (command, kotlin, permission) in [
            ("tts_available", "ttsAvailable", "allow-tts-available"),
            ("tts_speak", "ttsSpeak", "allow-tts-speak"),
            ("tts_stop", "ttsStop", "allow-tts-stop"),
        ] {
            assert!(BUILD_RS.contains(&format!("\"{command}\"")), "build.rs does not declare {command}");
            assert!(
                handler.split(',').any(|c| c.trim() == command),
                "{command} is not wired to the invoke handler"
            );
            assert!(
                DEFAULT_PERMISSIONS.contains(&format!("\"{permission}\"")),
                "erplora-android:default does not grant {permission}: the hub's capability would refuse it"
            );
            assert!(
                production.contains(&format!("(\"{kotlin}\"")),
                "Rust does not reach Kotlin under the name Kotlin answers to ({kotlin})"
            );
            let signature = format!("fun {kotlin}(invoke: Invoke)");
            let before = PLUGIN_KT.split(&signature).next().unwrap_or_default();
            assert!(before.trim_end().ends_with("@Command"), "Kotlin {kotlin} is not a @Command");
            let body = PLUGIN_KT
                .split(&signature)
                .nth(1)
                .and_then(|rest| rest.split("\n    }").next())
                .unwrap_or_else(|| panic!("no Kotlin {kotlin} command"));
            assert!(body.contains("invoke.resolve("), "{kotlin} never answers: the page would wait forever");
            assert!(body.contains("systemVoice."), "{kotlin} does not go through the system voice");
        }
    }

    #[test]
    fn the_speech_request_crosses_to_kotlin_under_the_keys_kotlin_reads() {
        let speak = serde_json::to_value(TtsSpeakArgs { text: "Hola".into(), language: "es-ES".into() })
            .expect("serializes");
        assert_eq!(speak, serde_json::json!({ "text": "Hola", "language": "es-ES" }));
        let available = serde_json::to_value(TtsLanguageArgs { language: "en-US".into() }).expect("serializes");
        assert_eq!(available, serde_json::json!({ "language": "en-US" }));
        for key in ["getString(\"text\"", "getString(\"language\""] {
            assert!(PLUGIN_KT.contains(key), "Kotlin does not read {key}) from the request");
        }
    }

    #[test]
    fn the_availability_reads_from_what_kotlin_sends_and_reaches_the_page_as_it() {
        let read: TtsAvailability = serde_json::from_str(r#"{"available":true}"#).expect("reads");
        assert!(read.available);
        let none: TtsAvailability = serde_json::from_str("{}").expect("an empty answer reads");
        assert!(!none.available, "an answer without the flag is no voice, never a voice");
        assert_eq!(
            serde_json::to_value(TtsAvailability { available: true }).expect("serializes"),
            serde_json::json!({ "available": true })
        );
        assert!(PLUGIN_KT.contains("put(\"available\""), "Kotlin does not answer under the key Rust reads");
    }

    #[test]
    fn rust_and_kotlin_spell_the_voice_refusals_the_same_way() {
        for literal in [TTS_UNAVAILABLE, TTS_FAILED] {
            assert!(
                SYSTEM_VOICE_KT.contains(&format!("\"{literal}\"")),
                "{literal} is not in SystemVoice.kt: the page could not tell «no voice» from «it broke»"
            );
        }
    }

    #[test]
    fn each_voice_refusal_is_classified_as_itself() {
        // `tts_unavailable` sends the page to the browser's voice; `tts_failed` is a reading that broke.
        assert!(matches!(
            classify_tts_error(&format!("[{TTS_UNAVAILABLE}] - no voice for the language")),
            Error::TtsUnavailable
        ));
        assert!(matches!(classify_tts_error(&format!("[{TTS_FAILED}] - the engine broke")), Error::TtsFailed));
        for message in ["java.lang.IllegalStateException", "the webview is unreachable", ""] {
            assert!(
                matches!(classify_tts_error(message), Error::PluginInvoke(_)),
                "{message:?} is not one of the two refusals"
            );
        }
    }

    #[test]
    fn the_voice_refusals_reach_the_page_under_the_names_the_page_reads() {
        for (error, word) in [(Error::TtsUnavailable, "tts_unavailable"), (Error::TtsFailed, "tts_failed")] {
            assert_eq!(serde_json::to_string(&error).unwrap(), format!("\"{word}\""));
        }
    }

    #[test]
    fn a_computer_has_no_system_voice_and_says_so() {
        // Windows (WebView2) and the other desktops read with the page's own `speechSynthesis`:
        // «not available» sends the page there, and reading here refuses instead of pretending.
        let desktop = ErploraAndroid::<tauri::Wry>(std::marker::PhantomData);
        assert!(!desktop.tts_available(TtsLanguageArgs { language: "es-ES".into() }).expect("answers"));
        assert!(matches!(
            desktop.tts_speak(TtsSpeakArgs { text: "Hola".into(), language: "es-ES".into() }),
            Err(Error::TtsUnavailable)
        ));
        assert!(desktop.tts_stop().is_ok());
    }

    #[test]
    fn the_reading_waits_off_the_async_workers() {
        // `ttsSpeak` answers when the WHOLE text has been read: blocking a worker of the async runtime
        // that long would stall every other command of the app while the assistant speaks.
        let production = LIB_RS.split("#[cfg(test)]").next().unwrap_or_default();
        for command in ["async fn tts_speak<", "async fn tts_available<"] {
            let body = production
                .split(command)
                .nth(1)
                .and_then(|rest| rest.split("\n}").next())
                .unwrap_or_else(|| panic!("no {command}"));
            assert!(body.contains("spawn_blocking("), "{command} blocks an async worker while it waits");
        }
    }

    #[test]
    fn the_system_voice_is_shut_down_with_the_activity() {
        let on_destroy = PLUGIN_KT
            .split("override fun onDestroy(activity: AppCompatActivity)")
            .nth(1)
            .and_then(|rest| rest.split("\n    }").next())
            .expect("no Kotlin onDestroy");
        assert!(
            on_destroy.contains("systemVoice.shutdown()"),
            "the engine outlives its activity: its service stays bound and a reading waits forever"
        );
    }

    #[test]
    fn the_app_can_see_the_system_voice_engine() {
        // From Android 11 (API 30) an app only sees the services it declares it looks for: without
        // this query `TextToSpeech` finds no engine, its init fails and the assistant stays mute,
        // with every test green.
        let manifest = without_comments(PLUGIN_MANIFEST);
        let queries = manifest.split("<queries>").nth(1).and_then(|rest| rest.split("</queries>").next());
        assert!(
            queries.is_some_and(|q| q.contains("android:name=\"android.intent.action.TTS_SERVICE\"")),
            "the plugin manifest does not query android.intent.action.TTS_SERVICE"
        );
    }
}
