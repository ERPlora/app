//! Publishing a file into Android's public Downloads collection.

use serde::{Deserialize, Serialize};
use tauri::Runtime;

use crate::{Error, ErploraAndroid};

/// The code an Android with no public Downloads collection is refused under.
///
/// Mirror of `DownloadPublisher.DOWNLOADS_UNREACHABLE`, and the same word `src-tauri` and the
/// page's `save-download.ts` already read: it is the ONE refusal the user can act on, so it must survive
/// the trip from Kotlin to the page intact. A test below checks the two sides never drift apart.
pub const DOWNLOADS_UNREACHABLE: &str = "downloads_unreachable";

/// What the shell asks the native side to publish, and where it staged the bytes.
///
/// A **path** and not the bytes: the shell has already written the file into its own cache, so an
/// export crosses the JNI boundary once instead of being copied onto a tablet's heap a second time.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublishArgs {
    source_path: String,
    name: String,
}

/// Where a published file ended up, in words to put in front of the user.
///
/// Not a path: `MediaStore` answers with `content://media/external/downloads/1234`, which says
/// nothing to anybody. What comes back is the folder and the name the file actually got — the only
/// sign it exists at all inside an app with no download shelf.
#[derive(Debug, Deserialize)]
pub struct PublishedDownload {
    pub location: String,
}

/// Reads the rejection Kotlin sent back and decides which of the two sentences it is.
///
/// Tauri renders a plugin rejection as `[code] - message` and hands Rust a plain string, so the
/// code has to be recognised out of the text. Getting this wrong is not cosmetic: an Android 9
/// till would be told "could not save" instead of "open your business in a browser", and a full
/// disk would be told to go and use a browser, which fixes nothing.
fn classify_publish_error(message: &str) -> Error {
    if message.contains(DOWNLOADS_UNREACHABLE) {
        Error::DownloadsUnreachable
    } else {
        Error::PluginInvoke(message.to_string())
    }
}

impl<R: Runtime> ErploraAndroid<R> {
    /// Publishes the file at `source_path` into Android's **public** Downloads collection under
    /// `name`, and answers with the place to show the user.
    ///
    /// This is how a business gets its own data off the tablet it keeps it on. Everywhere else the
    /// shell just writes to the Downloads folder; on Android there is no such folder to write to —
    /// what `download_dir()` resolves is app-scoped storage that Android 11 closed to every file
    /// manager, so the file would exist and be unreachable. `MediaStore` is the
    /// way in, and it needs Kotlin.
    ///
    /// ⚠️ **Blocks.** The call is dispatched onto Android's main looper and waits for the answer,
    /// so calling it FROM the main thread deadlocks. The shell's `save_download` is
    /// `#[tauri::command(async)]` for exactly this reason.
    pub fn save_to_downloads(
        &self,
        source_path: &std::path::Path,
        name: &str,
    ) -> Result<PublishedDownload, Error> {
        #[cfg(target_os = "android")]
        {
            return self
                .0
                .run_mobile_plugin(
                    "saveToDownloads",
                    PublishArgs {
                        source_path: source_path.display().to_string(),
                        name: name.to_string(),
                    },
                )
                .map_err(|e| classify_publish_error(&e.to_string()));
        }
        #[cfg(not(target_os = "android"))]
        {
            // Unreachable by construction: `save_target` only routes here on Android. Said out loud
            // anyway, because a desktop that somehow got here has NOT saved anything.
            let _ = (source_path, name);
            Err(Error::PluginInvoke(
                "save_to_downloads is Android only".into(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    // ── Publishing a file into the public Downloads collection ───────────────────────────────────

    const DOWNLOAD_PUBLISHER_KT: &str =
        include_str!("../android/src/main/java/com/erplora/android/DownloadPublisher.kt");

    /// The one refusal the user can act on has to survive the trip back from Kotlin.
    ///
    /// Kotlin rejects with a **code**, Tauri renders the rejection as `[code] - message`, and by
    /// the time it reaches Rust it is a plain string. If that string is not recognised the shell
    /// reports a generic failure and the page prints *«could not save»* instead of *«open your
    /// business in a browser»* — the sentence that tells the user what to do instead.
    #[test]
    fn an_android_too_old_to_publish_comes_back_as_downloads_unreachable() {
        let rejected = format!("[{DOWNLOADS_UNREACHABLE}] - Android 9 has no Downloads collection");
        assert!(matches!(
            classify_publish_error(&rejected),
            Error::DownloadsUnreachable
        ));
    }

    #[test]
    fn every_other_failure_keeps_its_own_words() {
        // A full disk, a revoked provider, a Kotlin exception: none of them is "this device cannot
        // save files", and dressing them up as that would send the user to a browser for a problem
        // a browser does not fix.
        for message in [
            "java.io.IOException: No space left on device",
            "insert into MediaStore returned no row",
            "",
        ] {
            assert!(
                matches!(classify_publish_error(message), Error::PluginInvoke(_)),
                "{message:?} is not the phone refusal"
            );
        }
    }

    #[test]
    fn the_refusal_reaches_the_page_under_the_name_the_shell_reads() {
        // `src-tauri` maps this variant onto `ShellError::DownloadsUnreachable`, which serializes
        // to the same word the web checks for (`save-download.ts`). One spelling, end to end.
        assert_eq!(
            serde_json::to_string(&Error::DownloadsUnreachable).unwrap(),
            "\"downloads_unreachable\""
        );
    }

    #[test]
    fn rust_and_kotlin_spell_the_refusal_the_same_way() {
        // Same trap as the permission strings above: the code is a bare literal on both sides, so
        // a typo in either one is invisible until a real Android 9 till says the wrong sentence.
        assert!(
            DOWNLOAD_PUBLISHER_KT.contains(DOWNLOADS_UNREACHABLE),
            "DownloadPublisher.kt never rejects with {DOWNLOADS_UNREACHABLE}, so Rust would \
             classify an old Android as a generic failure"
        );
    }

    #[test]
    fn publishing_needs_no_permission_the_plugin_would_have_to_ask_for() {
        // Why `MediaStore` and not `getExternalStoragePublicDirectory`: inserting into the public
        // Downloads collection needs NO permission from API 29 on. If this ever grows a
        // `WRITE_EXTERNAL_STORAGE`, it also grows a runtime dialog the save command has to wait
        // for — a different design, not a line to slip in.
        let code = without_kotlin_comments(DOWNLOAD_PUBLISHER_KT);
        assert!(
            !code.contains("WRITE_EXTERNAL_STORAGE"),
            "publishing now wants a runtime permission; the save path has to ask for it first"
        );
    }
}
