//! The way DOWN: a file the user can find afterwards.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::ShellError;

/// Longest leaf name a file system will take: 255 bytes on ext4, APFS and NTFS alike.
const MAX_FILE_NAME_BYTES: usize = 255;

/// How many `name (n).ext` variants are tried before giving up. A folder with 999 copies of the
/// same export is not a case to keep spinning on; it is a case to tell the user about.
const MAX_DOWNLOAD_COPIES: u32 = 999;

/// Where a saved file landed, as the page will spell it out to the user.
#[derive(Debug, Serialize)]
pub struct SavedDownload {
    /// Where the file is, in the words to put in front of the user — **not always a path**.
    ///
    /// On the desktop it is the absolute path, because that is what the user's file manager opens.
    /// On Android there is no path to show: the file lives in the public Downloads **collection**
    /// and `MediaStore` names it `content://media/external/downloads/1234`, so what travels is the
    /// folder and the name — `Download/factura-2026-0042.pdf`.
    ///
    /// Either way it is not a detail: inside the installed app there is no download shelf, no
    /// notification and no Downloads button, so this string is the ONLY trace the user gets that
    /// the file exists at all.
    pub path: String,
}

/// Is the folder the OS calls "Downloads" one the **user** can reach on this platform?
///
/// Desktop: yes — `dirs::download_dir()` resolves to `~/Downloads`, `%USERPROFILE%\Downloads` or
/// `$XDG_DOWNLOAD_DIR`, all of which open in the user's own file manager.
///
/// Android: **no**, and this is the whole reason the predicate exists. Tauri's `download_dir()`
/// there is `getExternalFilesDir(DIRECTORY_DOWNLOADS)` —
/// `/storage/emulated/0/Android/data/com.erplora.app/files/Download` — and Android 11 closed
/// `Android/data` to the system Files app and to every third-party file manager. Writing there and
/// answering "saved to …" would be a failed save with a success message on top: the file exists,
/// the user cannot get to it, and nothing says so.
///
/// Anything else is refused rather than assumed. A platform nobody here has reasoned about does not
/// get the benefit of the doubt about where its files end up.
pub fn downloads_dir_is_reachable(target_os: &str) -> bool {
    matches!(target_os, "macos" | "windows" | "linux")
}

/// The folder a download may be written to — or the refusal to hand back to the page.
///
/// One place, because the two ways this can fail are not the same sentence. A phone gets
/// [`ShellError::DownloadsUnreachable`], which the page turns into *«this app cannot save files on a
/// phone or tablet»* — something the user can act on. A desktop that resolved no Downloads folder at
/// all (a Linux box with no `$XDG_DOWNLOAD_DIR`) is a plain failure, and putting the phone sentence
/// in front of that user would just be wrong.
///
/// The platform is checked **before** the resolved path, and that order is the point: Android does
/// return a Downloads folder — it is simply one no file manager will open — so a check that only
/// looked at whether a path came back would never fire.
pub fn reachable_downloads_dir(
    target_os: &str,
    resolved: Option<PathBuf>,
) -> Result<PathBuf, ShellError> {
    if !downloads_dir_is_reachable(target_os) {
        return Err(ShellError::DownloadsUnreachable);
    }
    resolved.ok_or_else(|| ShellError::Io(format!("no downloads directory on {target_os}")))
}

/// Where a saved file goes on this platform.
///
/// Two ways to save exist, and they are not variations of one thing. A desktop has a **folder** the
/// user opens in their own file manager. Android has no such folder for us — what `download_dir()`
/// resolves is app-scoped storage Android 11 closed to every file manager — but it does have a
/// public **Downloads collection**, which is not a path at all: a row in `MediaStore` that the Files
/// app browses and that the shell can only reach through Kotlin.
#[derive(Debug, PartialEq, Eq)]
pub enum SaveTarget {
    /// A folder on this device's file system, which the user can open.
    Folder(PathBuf),
    /// Android's public Downloads collection, published through `MediaStore` by our own plugin.
    AndroidDownloads,
}

/// The one place that decides where a download goes — or that it cannot go anywhere.
///
/// Android is answered **before** the resolved path is even looked at, and that order is the point:
/// Android does hand back a Downloads folder, it is simply one no file manager will open, so a
/// decision made from that value would keep writing the file where the user cannot reach it.
/// Everything else falls through to [`reachable_downloads_dir`], which keeps the two
/// remaining answers apart: a phone with nowhere to save gets [`ShellError::DownloadsUnreachable`],
/// which the page turns into a sentence the user can act on, and a desktop that resolved no
/// Downloads folder at all gets a plain failure — putting the phone sentence in front of that user
/// would just be wrong.
///
/// iOS is still refused, on purpose: closing the same hole there needs its own native work
/// (`UIDocumentPickerViewController` or the share sheet) and nothing here has been built or tried.
pub fn save_target(target_os: &str, resolved: Option<PathBuf>) -> Result<SaveTarget, ShellError> {
    if target_os == "android" {
        return Ok(SaveTarget::AndroidDownloads);
    }
    reachable_downloads_dir(target_os, resolved).map(SaveTarget::Folder)
}

/// The single, safe leaf name a saved file may land under — or `None`.
///
/// The page names a **file**; the shell chooses the **place**. That split is the frontier: the
/// destination folder is decided here and the only thing the page contributes is the last path
/// component, so nothing it can send walks out of Downloads and into `~/.ssh` or `System32`.
///
/// Refused: anything carrying a path separator or a drive/stream colon (`report.pdf:hidden.exe` is
/// an NTFS alternate data stream — a file that never shows up in a listing), anything with a
/// control character (a NUL truncates the name at the syscall boundary, so the file written is not
/// the file named, and the rest make the path we *report* unreadable), the pure-dot names that mean
/// "this directory" and "the one above", the empty name, and anything past what a file system will
/// take. Truncating a too-long name would save the file under a name the user was never shown.
pub fn download_file_name(raw: &str) -> Option<String> {
    // Trim BEFORE measuring, on purpose: the limit has to be about the name that is actually
    // written, and that is the trimmed one. Measuring first would refuse a perfectly good file for
    // the trailing newline a copy-paste brought along.
    let name = raw.trim();
    if name.is_empty() || name.len() > MAX_FILE_NAME_BYTES {
        return None;
    }
    // `.`, `..`, `...` — every one of them names a directory, not a file in it.
    if name.chars().all(|c| c == '.') {
        return None;
    }
    if name
        .chars()
        .any(|c| matches!(c, '/' | '\\' | ':') || c.is_control())
    {
        return None;
    }
    Some(name.to_string())
}

/// The path to write to inside `dir`, never on top of a file that is already there — or `None` when
/// every candidate is taken.
///
/// Saving twice must leave two files. A till exports its backup, exports it again after a change,
/// and silently overwriting the first one would destroy the copy the user was keeping. Same `name
/// (2).ext` convention every browser uses, so the file is where the user goes looking for it.
///
/// The number goes in before the **last** dot: `2026.08.08-backup.zip` becomes
/// `2026.08.08-backup (2).zip` and not `2026 (2).08.08-backup.zip`, which no longer reads as a date
/// and no longer sorts beside its sibling.
///
/// `taken` is asked, not assumed, so the rule is exercised without touching a disk.
pub fn free_download_path(
    dir: &Path,
    name: &str,
    taken: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let first = dir.join(name);
    if !taken(&first) {
        return Some(first);
    }
    // An empty stem means the dot is the FIRST character (`.env`): there is no name before it to
    // number, so the whole thing is the stem and ` (2)` goes on the end.
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name, String::new()),
    };
    (2..=MAX_DOWNLOAD_COPIES).find_map(|n| {
        let candidate = dir.join(format!("{stem} ({n}){extension}"));
        (!taken(&candidate)).then_some(candidate)
    })
}

/// `save_download` — writes bytes the page already holds into the user's Downloads folder and
/// answers with the path.
///
/// The sibling of `open_external_url`, for the page's downloads, which are about **saving a file**
/// rather than going somewhere. Sending a browser is no answer for them — the bytes of a `/files`
/// download, of a backup export and of an invoice PDF are fetched with the hub session attached,
/// and a separate program has no session to fetch them again with.
///
/// So the page keeps the bytes and the shell keeps the disk. What comes back is the **path**,
/// because inside the installed app nothing else would say the file arrived: there is no download
/// shelf, no notification and no Downloads button, and wry's own default handler writes the file
/// without a word.
///
/// Refusals are returned, never swallowed — and one of them is a sentence in its own right:
/// `downloads_unreachable` means this device has no Downloads folder the user could open, which the
/// page must say out loud instead of reporting a path into storage nobody can browse.
///
/// ⚠️ `(async)` is not decoration. The Android half hands the file to Kotlin through
/// `run_mobile_plugin`, which dispatches onto Android's main looper and BLOCKS waiting for the
/// answer; a plain `#[tauri::command]` runs on that very thread, so the till would hang forever on
/// the press that saves its backup. The body stays synchronous — the attribute only moves it off
/// the main thread.
#[tauri::command(async)]
pub(crate) fn save_download(
    app: tauri::AppHandle,
    name: String,
    data_base64: String,
) -> Result<SavedDownload, ShellError> {
    use base64::Engine as _;
    use tauri::Manager;

    // `std::env::consts::OS` is the TARGET the binary was compiled for, so on the Android build it
    // reads `"android"` — the same string the tests reason about.
    let target = save_target(std::env::consts::OS, app.path().download_dir().ok())
        .inspect_err(|e| log::warn!("shell: nowhere to save {name:?}: {e}"))?;
    let Some(file_name) = download_file_name(&name) else {
        log::warn!("shell: refused to save {name:?} — that is not a file name");
        return Err(ShellError::DownloadRefused);
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_base64.as_bytes())
        .map_err(|_| ShellError::DownloadRefused)?;

    match target {
        SaveTarget::Folder(dir) => {
            std::fs::create_dir_all(&dir).map_err(|e| ShellError::Io(e.to_string()))?;
            let path = free_download_path(&dir, &file_name, &|candidate| candidate.exists())
                .ok_or(ShellError::DownloadRefused)?;
            std::fs::write(&path, &bytes).map_err(|e| ShellError::Io(e.to_string()))?;
            Ok(SavedDownload {
                path: path.display().to_string(),
            })
        }
        SaveTarget::AndroidDownloads => {
            #[cfg(target_os = "android")]
            {
                publish_to_android_downloads(&app, &file_name, &bytes)
            }
            #[cfg(not(target_os = "android"))]
            {
                // Unreachable: `save_target` only answers this on Android. Even here the command
                // refuses rather than report a save that did not happen.
                let _ = (&app, &file_name, &bytes);
                Err(ShellError::DownloadsUnreachable)
            }
        }
    }
}

/// Hands the bytes to Kotlin so they land in Android's **public** Downloads collection.
///
/// The file is staged in the app's own cache first and handed over as a **path**: a 5 MB export has
/// already crossed the `invoke` boundary once as base64, and copying it onto a tablet's heap again
/// to cross JNI would be the second copy that matters. The staged file is removed either way —
/// including when publishing fails, so a refused save leaves nothing behind to fill the device.
///
/// What comes back is not a path but a **place to say**: `MediaStore` answers with
/// `content://media/external/downloads/1234`, and what the user needs is *Download/factura.pdf* —
/// the folder they will open and the name they will look for. Inside the installed app that
/// sentence is the only sign the file exists at all.
#[cfg(target_os = "android")]
fn publish_to_android_downloads(
    app: &tauri::AppHandle,
    file_name: &str,
    bytes: &[u8],
) -> Result<SavedDownload, ShellError> {
    use tauri::Manager;
    use tauri_plugin_erplora_android::ErploraAndroidExt as _;

    let staging = app
        .path()
        .app_cache_dir()
        .map_err(|e| ShellError::Io(e.to_string()))?
        .join("downloads");
    std::fs::create_dir_all(&staging).map_err(|e| ShellError::Io(e.to_string()))?;
    let staged = staging.join(file_name);
    std::fs::write(&staged, bytes).map_err(|e| ShellError::Io(e.to_string()))?;

    let published = app.erplora_android().save_to_downloads(&staged, file_name);
    let _ = std::fs::remove_file(&staged);

    match published {
        Ok(saved) => Ok(SavedDownload {
            path: saved.location,
        }),
        // The one refusal the user can act on has to keep its own sentence all the way to the page.
        Err(tauri_plugin_erplora_android::Error::DownloadsUnreachable) => {
            log::warn!("shell: this Android has no public Downloads collection");
            Err(ShellError::DownloadsUnreachable)
        }
        Err(e) => {
            log::warn!("shell: could not publish {file_name:?} to Downloads: {e}");
            Err(ShellError::Io(e.to_string()))
        }
    }
}
