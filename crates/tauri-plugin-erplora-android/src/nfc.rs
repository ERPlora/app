//! Reading a badge off the device's own NFC reader.

use serde::{Deserialize, Serialize};
use tauri::Runtime;

use crate::{Error, ErploraAndroid};

/// This device has no NFC reader at all. Mirror of `NfcBadge.NFC_UNAVAILABLE`, and the
/// refusal every non-Android build answers with: there is nothing to switch on, so the shell must
/// stop polling instead of waiting for a card that can never come.
pub const NFC_UNAVAILABLE: &str = "nfc_unavailable";

/// There IS a reader and it is switched off. Mirror of `NfcBadge.NFC_DISABLED` — the one NFC
/// refusal the user can act on, and the reason the three do not share a code.
pub const NFC_DISABLED: &str = "nfc_disabled";

/// The card answers with a fresh id on every tap, so it cannot be anybody's badge. Mirror of
/// `NfcBadge.NFC_RANDOM_UID`. Said out loud on purpose: enrolling one would work, and then never
/// match again.
pub const NFC_RANDOM_UID: &str = "nfc_random_uid";

/// How long one `nfc_read` keeps reader mode open when the caller names nothing. Kotlin clamps and
/// owns the bounds ([`NfcBadge.clampTimeout`]); this is only the default the shell sends.
pub const NFC_DEFAULT_TIMEOUT_MS: u64 = 15_000;

/// How long reader mode may stay open on one call. Kotlin clamps it: an argument nobody
/// typed by hand must never be the reason a till has no reader.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NfcReadArgs {
    timeout_ms: u64,
}

/// The wire shape of `nfc_read`.
///
/// `badge` **absent** is the ordinary outcome: the window closed with no card on the reader. The
/// shell polls this command, so that non-event must not travel as an error — a till waiting for a
/// badge would otherwise log a failure every fifteen seconds.
#[derive(Debug, Default, Deserialize)]
pub struct NfcBadgeRead {
    #[serde(default)]
    pub badge: Option<String>,
}

/// Reads the rejection Kotlin sent back and decides which NFC refusal it is.
///
/// Same trip as `downloads::classify_publish_error`: Tauri renders a plugin rejection as `[code] - message`
/// and hands Rust a plain string. Getting it wrong is not cosmetic — a tablet with no reader would
/// be told to switch NFC on in its settings and sent looking for a toggle that is not there.
fn classify_nfc_error(message: &str) -> Error {
    if message.contains(NFC_UNAVAILABLE) {
        Error::NfcUnavailable
    } else if message.contains(NFC_DISABLED) {
        Error::NfcDisabled
    } else if message.contains(NFC_RANDOM_UID) {
        Error::NfcRandomUid
    } else {
        Error::PluginInvoke(message.to_string())
    }
}

impl<R: Runtime> ErploraAndroid<R> {
    /// Opens NFC reader mode for at most `timeout_ms` and answers with the badge of the card that
    /// was tapped — `None` when the window closed with nothing on the reader.
    ///
    /// The badge is the card's UID as uppercase hex (`NfcBadge.toBadge`): the SAME kind of string a
    /// USB keyboard-wedge reader types, so the shell can hand it to the badge subscribers the wedge
    /// already feeds. One badge path, two origins — nothing above learns where a card came from.
    ///
    /// Three refusals, kept apart because there are three different things to do about them:
    /// [`Error::NfcUnavailable`] (no reader — buy one), [`Error::NfcDisabled`] (switch it on) and
    /// [`Error::NfcRandomUid`] (that card randomises its id; use another one).
    ///
    /// ⚠️ **Blocks** — dispatched onto Android's main looper and waits out the whole window, so the
    /// calling command must be `#[tauri::command(async)]`, exactly like `save_download`.
    pub fn nfc_read(&self, timeout_ms: u64) -> Result<Option<String>, Error> {
        #[cfg(target_os = "android")]
        {
            return self
                .0
                .run_mobile_plugin::<NfcBadgeRead>("nfcRead", NfcReadArgs { timeout_ms })
                .map(|read| read.badge)
                .map_err(|e| classify_nfc_error(&e.to_string()));
        }
        #[cfg(not(target_os = "android"))]
        {
            // A REFUSAL, not a resolved `None`: the shell polls this command, and "nothing tapped"
            // would spin a loop forever on a machine that can never produce a card. `nfc_unavailable`
            // is what tells the shell to stop asking — and it is also true.
            let _ = timeout_ms;
            Err(Error::NfcUnavailable)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    // ── Reading a badge off the device's own NFC reader ──────────────────────────────────────
    //
    // Same shape as the bluetooth block above: the radio lives in Kotlin (`NfcBadge.kt`), and what
    // Rust owns is the CONTRACT across the JNI boundary — the command name, the argument key, the
    // shape of the answer and the three refusal codes. All bare literals on both sides.

    const NFC_BADGE_KT: &str =
        include_str!("../android/src/main/java/com/erplora/android/NfcBadge.kt");

    #[test]
    fn the_read_timeout_crosses_to_kotlin_under_the_key_kotlin_reads() {
        let args = NfcReadArgs { timeout_ms: 15_000 };
        let json = serde_json::to_value(&args).expect("serializable");
        assert_eq!(json["timeoutMs"], 15_000);
        assert!(
            ERPLORA_ANDROID_PLUGIN_KT.contains("\"timeoutMs\""),
            "ErploraAndroidPlugin.kt no longer reads timeoutMs — every read would silently take \
             the default, and a screen that asked for a short poll would hold the radio open"
        );
    }

    #[test]
    fn a_tapped_card_deserializes_from_what_kotlin_sends() {
        let read: NfcBadgeRead =
            serde_json::from_value(serde_json::json!({ "badge": "04A23B5C6D7E80" }))
                .expect("the wire shape Kotlin builds");
        assert_eq!(read.badge.as_deref(), Some("04A23B5C6D7E80"));
    }

    #[test]
    fn nothing_tapped_is_an_answer_and_not_a_failure() {
        // Kotlin resolves with the key ABSENT when the window closed with no card on it. That is
        // the ordinary outcome of every poll — the shell loops on it — so it must not arrive as an
        // error, or a till waiting for a badge would log a failure every fifteen seconds.
        let read: NfcBadgeRead =
            serde_json::from_value(serde_json::json!({})).expect("an empty answer is valid");
        assert!(read.badge.is_none());
    }

    #[test]
    fn rust_and_kotlin_spell_the_nfc_refusals_and_the_command_the_same_way() {
        for literal in [NFC_UNAVAILABLE, NFC_DISABLED, NFC_RANDOM_UID] {
            assert!(
                NFC_BADGE_KT.contains(literal),
                "{literal} is not in NfcBadge.kt — the refusal would reach the page under a name \
                 nothing recognises, and the user would be told the wrong thing to do"
            );
        }
        assert!(
            ERPLORA_ANDROID_PLUGIN_KT.contains("nfcRead"),
            "ErploraAndroidPlugin.kt has no nfcRead command — the mobile call would answer \
             `command not found`, on a real device only"
        );
    }

    #[test]
    fn each_nfc_refusal_is_classified_as_itself() {
        // Three refusals because there are three different things to do about it: buy a reader,
        // switch NFC on, use another card. Collapsing them into one would send the user to the
        // settings screen for a tablet that has no reader to enable.
        assert!(matches!(
            classify_nfc_error(&format!("[{NFC_UNAVAILABLE}] - no reader on this device")),
            Error::NfcUnavailable
        ));
        assert!(matches!(
            classify_nfc_error(&format!("[{NFC_DISABLED}] - NFC is off")),
            Error::NfcDisabled
        ));
        assert!(matches!(
            classify_nfc_error(&format!("[{NFC_RANDOM_UID}] - this card randomises its id")),
            Error::NfcRandomUid
        ));
    }

    #[test]
    fn any_other_nfc_failure_keeps_its_own_words() {
        for message in ["java.lang.SecurityException", "activity is not resumed", ""] {
            assert!(
                matches!(classify_nfc_error(message), Error::PluginInvoke(_)),
                "{message:?} is not one of the three refusals"
            );
        }
    }

    #[test]
    fn the_nfc_refusals_reach_the_page_under_the_names_the_shell_reads() {
        // The shell branches on these words: «tap the card» only appears where there IS a reader,
        // and «switch NFC on» only where switching it on would help.
        for (error, word) in [
            (Error::NfcUnavailable, "nfc_unavailable"),
            (Error::NfcDisabled, "nfc_disabled"),
            (Error::NfcRandomUid, "nfc_random_uid"),
        ] {
            assert_eq!(serde_json::to_string(&error).unwrap(), format!("\"{word}\""));
        }
    }

    #[test]
    fn a_platform_without_nfc_refuses_cleanly_instead_of_waiting() {
        // Desktop has no reader mode. The answer has to be the refusal `nfc_unavailable` and not a
        // resolved `None`: the shell POLLS this command, and a resolved nothing would spin a
        // fifteen-second loop forever on a machine that can never produce a card.
        let desktop = ErploraAndroid::<tauri::Wry>(std::marker::PhantomData);
        assert!(matches!(
            desktop.nfc_read(NFC_DEFAULT_TIMEOUT_MS),
            Err(Error::NfcUnavailable)
        ));
    }

    /// NFC is an install-time (`normal`) permission, so it must NOT join the runtime policy.
    ///
    /// Putting it there would be worse than useless: `requestPermissionForAliases` on a normal
    /// permission shows no dialog, and the batch would grow an entry that can never be denied and
    /// can never be granted by the user — noise in the very map the shell reads to decide what it
    /// may do.
    #[test]
    fn nfc_is_not_a_runtime_permission() {
        assert!(
            !PERMISSION_POLICY_KT.contains("android.permission.NFC"),
            "NFC joined the runtime permission policy; it is a `normal` permission and no dialog \
             will ever be shown for it"
        );
    }
}
