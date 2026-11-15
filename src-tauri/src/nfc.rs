//! The badge, read off the device's own NFC reader.
//!
//! The counter reads badges through a USB reader that behaves as a keyboard, and the shell catches
//! the burst by its speed (`badge-scanner.ts`). A tablet has no USB reader — and has had a
//! reader inside it all along, unused. This is the second origin of the SAME badge path: what comes
//! back is the same kind of string the wedge types, and the web hands it to the very subscribers the
//! wedge feeds. No screen above learns where a card came from.

use serde::Serialize;

use crate::ShellError;

/// Turns a plugin refusal into the word the page reads.
///
/// The three NFC refusals are kept apart all the way through because there are three different
/// things to do about them: buy a reader, switch NFC on, use another card. Collapsing them would
/// send a user with no chip at all into the settings screen looking for a toggle.
fn nfc_shell_error(error: tauri_plugin_erplora_android::Error) -> ShellError {
    use tauri_plugin_erplora_android::Error as PluginError;
    match error {
        PluginError::NfcUnavailable => ShellError::NfcUnavailable,
        PluginError::NfcDisabled => ShellError::NfcDisabled,
        PluginError::NfcRandomUid => ShellError::NfcRandomUid,
        other => ShellError::Io(other.to_string()),
    }
}

/// How long one read waits, with the shell's default when the page names nothing.
fn nfc_read_timeout(requested: Option<u64>) -> u64 {
    requested.unwrap_or(tauri_plugin_erplora_android::NFC_DEFAULT_TIMEOUT_MS)
}

/// What one read produced. An OBJECT and not a bare `Option<String>`, deliberately.
///
/// The web reaches this command through `invokeTauri`, which answers `null` when there is no shell
/// at all — a plain browser. A bare option would make that indistinguishable from "the window
/// closed with nothing tapped", and the two are opposites: one means stop asking forever, the other
/// means ask again right now. An object is never `null`, so the shape itself carries the answer.
#[derive(Debug, Serialize)]
pub struct NfcReadOutcome {
    /// The badge of the card that was tapped, or `null` when the window closed empty.
    pub badge: Option<String>,
}

/// `erplora_nfc_read` — waits for a card on the device's own reader and answers with its badge.
///
/// `badge: null` is the ordinary outcome: the window closed with nothing tapped. The web polls this
/// command while a screen is waiting for a badge, so that non-event must not be an error — it would
/// bury the real refusals under one failure every fifteen seconds.
///
/// `Err(NfcUnavailable)` is how a machine with no reader says *stop asking*, which is also what
/// every desktop build answers by construction. Answering an empty read there instead would leave a
/// poll loop spinning forever on hardware that can never produce a card.
///
/// ⚠️ `(async)` is load-bearing, exactly as in `save_download` and `erplora_print`: the Android arm
/// crosses into Kotlin through `run_mobile_plugin`, which dispatches onto the main looper and
/// BLOCKS for the whole window. On a plain command the till would freeze for fifteen seconds.
#[tauri::command(async)]
pub(crate) fn erplora_nfc_read(
    app: tauri::AppHandle,
    timeout_ms: Option<u64>,
) -> Result<NfcReadOutcome, ShellError> {
    use tauri_plugin_erplora_android::ErploraAndroidExt as _;
    app.erplora_android()
        .nfc_read(nfc_read_timeout(timeout_ms))
        .map(|badge| NfcReadOutcome { badge })
        .map_err(nfc_shell_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── The badge can also arrive by NFC ─────────────────────────────────────────────────────
    //
    // The plugin owns the radio; the shell owns the WORD that reaches the page. Three refusals
    // arrive from Kotlin and three different things are done about them — buy a reader, switch NFC
    // on, use another card — so the mapping has to keep them apart all the way to the web, where
    // `nfc-badge.ts` branches on exactly these strings.

    #[test]
    fn each_nfc_refusal_reaches_the_page_as_itself() {
        for (from, into) in [
            (
                tauri_plugin_erplora_android::Error::NfcUnavailable,
                "nfc_unavailable",
            ),
            (
                tauri_plugin_erplora_android::Error::NfcDisabled,
                "nfc_disabled",
            ),
            (
                tauri_plugin_erplora_android::Error::NfcRandomUid,
                "nfc_random_uid",
            ),
        ] {
            assert_eq!(
                serde_json::to_string(&nfc_shell_error(from)).unwrap(),
                format!("\"{into}\"")
            );
        }
    }

    #[test]
    fn an_unnamed_nfc_failure_does_not_borrow_one_of_the_three_sentences() {
        // A SecurityException, an activity that was not resumed, a plugin that is not there. None
        // of those is "switch NFC on", and dressing one up as that sends the user to a toggle that
        // is already on — the failure would then look like the app lying to them.
        let generic = nfc_shell_error(tauri_plugin_erplora_android::Error::PluginInvoke(
            "activity is not resumed".into(),
        ));
        let word = serde_json::to_string(&generic).unwrap();
        for taken in ["nfc_unavailable", "nfc_disabled", "nfc_random_uid"] {
            assert!(!word.contains(taken), "{word} borrowed {taken}");
        }
        assert!(word.contains("activity is not resumed"), "{word}");
    }

    #[test]
    fn an_empty_read_is_still_an_object_the_web_can_tell_from_no_shell_at_all() {
        // `invokeTauri` answers `null` in a plain browser. If an empty read serialized to `null`
        // too, the loop could not tell "ask again in a moment" from "there is no shell here, stop
        // forever" — and one of the two guesses spins a poll loop in a browser tab.
        let empty = serde_json::to_value(NfcReadOutcome { badge: None }).unwrap();
        assert!(empty.is_object(), "{empty}");
        assert!(empty["badge"].is_null());

        let tapped = serde_json::to_value(NfcReadOutcome {
            badge: Some("04A23B5C6D7E80".into()),
        })
        .unwrap();
        assert_eq!(tapped["badge"], "04A23B5C6D7E80");
    }

    #[test]
    fn the_shell_asks_for_a_bounded_window_by_default() {
        // Reader mode with no deadline outlives the screen that opened it, and the next tap is
        // delivered to a callback nobody is waiting on. The plugin clamps; the shell must still
        // send something sane when the page names nothing.
        assert_eq!(
            nfc_read_timeout(None),
            tauri_plugin_erplora_android::NFC_DEFAULT_TIMEOUT_MS
        );
        assert_eq!(nfc_read_timeout(Some(5_000)), 5_000);
    }
}
