//! The boundary of `save_download`: where a file the page hands us is allowed to land, and on
//! which platforms landing there means the user can actually find it (hub#480).
//!
//! [ADR-0255](hub#475) gave the installed app a way OUT to the system browser and left three
//! `window.open` calls behind, all of them about **saving a file**. Saving is not navigating: the
//! bytes of a `/files` download, of a backup export and of an invoice PDF are fetched by the page
//! with the hub session attached, so no browser — ours or the user's — can be sent to fetch them
//! again. The page has the bytes; what it lacks is somewhere to put them.
//!
//! In a browser that somewhere is the download manager (`<a download>`). Inside the installed app
//! there is none: wry registers no `DownloadListener` on Android at all, and on the desktop its
//! default handler writes to the OS Downloads folder **without a word** — no shelf, no
//! notification, nothing. So the shell saves the file itself and RETURNS the path, which is the
//! only way the user learns it exists.
//!
//! Two frontiers come with that, and both are here:
//!
//! 1. **The page names a file, never a place.** `download_file_name` reduces whatever it sent to a
//!    single leaf name, so nothing it can say escapes the Downloads folder.
//! 2. **A file the user cannot reach is not saved.** On Android `download_dir()` is
//!    `getExternalFilesDir(DIRECTORY_DOWNLOADS)` — app-scoped storage that, since Android 11, the
//!    system Files app and every file manager refuse to browse. Writing there and reporting success
//!    would be hub#475 again in a nicer costume.

use std::path::{Path, PathBuf};

use erplora_tauri_lib::{
    download_file_name, downloads_dir_is_reachable, free_download_path, reachable_downloads_dir,
    save_target, SaveTarget, ShellError,
};

// ── Which platforms have a Downloads folder the USER can reach ───────────────────────────────────

#[test]
fn the_desktop_downloads_folder_is_the_users_own() {
    // `dirs::download_dir()` on all three: `~/Downloads`, `%USERPROFILE%\Downloads`,
    // `$XDG_DOWNLOAD_DIR`. The user opens it from their own file manager.
    for os in ["macos", "windows", "linux"] {
        assert!(
            downloads_dir_is_reachable(os),
            "{os} has a Downloads folder the user can open"
        );
    }
}

#[test]
fn a_phone_downloads_folder_belongs_to_the_APP_not_the_user() {
    // Tauri's Android `getDownloadDir` resolves to `getExternalFilesDir(DIRECTORY_DOWNLOADS)` —
    // `/storage/emulated/0/Android/data/com.erplora.app/files/Download`. Android 11 closed
    // `Android/data` to the Files app and to every third-party file manager, so a file written
    // there exists and is unreachable. Saying "saved" about it is a lie with a path attached.
    for os in ["android", "ios"] {
        assert!(
            !downloads_dir_is_reachable(os),
            "{os} must refuse rather than hide the file"
        );
    }
}

#[test]
fn an_unknown_platform_is_refused_rather_than_assumed() {
    // Fail towards the restrictive side: a target nobody here has reasoned about does not get the
    // benefit of the doubt about where its files end up.
    for os in ["", "freebsd", "MACOS", "solaris"] {
        assert!(
            !downloads_dir_is_reachable(os),
            "{os:?} was never reasoned about and must not be assumed reachable"
        );
    }
}

// ── Where a download may be written, decided in one place ────────────────────────────────────────

#[test]
fn writes_into_the_folder_the_platform_resolved() {
    assert_eq!(
        reachable_downloads_dir("macos", Some(PathBuf::from("/Users/ana/Downloads"))).unwrap(),
        PathBuf::from("/Users/ana/Downloads")
    );
}

#[test]
fn refuses_a_phone_BEFORE_looking_at_the_folder_it_resolved() {
    // Android does resolve a Downloads folder — it is just not one the user can open. Refusing has
    // to happen on the platform, not on whether a path came back, or the refusal never fires.
    let refused = reachable_downloads_dir(
        "android",
        Some(PathBuf::from(
            "/storage/emulated/0/Android/data/com.erplora.app/files/Download",
        )),
    );
    assert!(matches!(refused, Err(ShellError::DownloadsUnreachable)));
}

#[test]
fn a_desktop_with_no_downloads_folder_is_a_plain_failure_not_a_phone() {
    // A Linux box with no `$XDG_DOWNLOAD_DIR` cannot save either, but "this app cannot save files
    // on a phone or tablet" would be the wrong sentence to put in front of that user.
    let missing = reachable_downloads_dir("linux", None);
    assert!(matches!(missing, Err(ShellError::Io(_))));
}

// ── WHERE the bytes go, decided per platform (hub#499) ───────────────────────────────────────────
//
// `reachable_downloads_dir` above answers one question — *is the folder Tauri resolved one the user
// can open?* — and on Android the answer is still no. What changed is that a folder is no longer
// the only way to save: Android has a public **Downloads collection**, published through
// `MediaStore`, which is not a path at all. `save_target` is the whole decision, so the platform
// that gets a folder, the platform that gets the collection and the platform that gets nothing are
// decided in one readable place instead of being spread across the command.

#[test]
fn android_publishes_into_the_public_downloads_collection() {
    // hub#499. Refusing was right while there was nowhere to put the file; there is now, and it is
    // not a folder: `MediaStore.Downloads` is a content collection the Files app browses and every
    // file manager lists. A tablet till can finally take its own backup off the device.
    assert_eq!(
        save_target("android", None).unwrap(),
        SaveTarget::AndroidDownloads
    );
}

#[test]
fn android_ignores_the_folder_tauri_resolved_for_it() {
    // The decision must NOT depend on the resolved path. Tauri hands back
    // `getExternalFilesDir(DIRECTORY_DOWNLOADS)` on Android — app-scoped storage Android 11 closed
    // to every file manager — so a target picked from that value would keep writing the file into
    // the one place the user cannot reach.
    assert_eq!(
        save_target(
            "android",
            Some(PathBuf::from(
                "/storage/emulated/0/Android/data/com.erplora.app/files/Download"
            ))
        )
        .unwrap(),
        SaveTarget::AndroidDownloads
    );
}

#[test]
fn the_desktop_still_writes_into_the_users_own_folder() {
    assert_eq!(
        save_target("windows", Some(PathBuf::from(r"C:\Users\ana\Downloads"))).unwrap(),
        SaveTarget::Folder(PathBuf::from(r"C:\Users\ana\Downloads"))
    );
}

#[test]
fn ios_is_still_refused_because_nobody_has_written_that_path_yet() {
    // The same hole hub#499 closed on Android is still open on iOS, and it needs its own native
    // work (`UIDocumentPickerViewController` or the share sheet). Until that exists the honest
    // answer is the refusal the user can act on — not a file saved somewhere nobody can browse.
    assert!(matches!(
        save_target("ios", Some(PathBuf::from("/var/mobile/.../Downloads"))),
        Err(ShellError::DownloadsUnreachable)
    ));
}

#[test]
fn a_platform_nobody_reasoned_about_gets_no_benefit_of_the_doubt() {
    for os in ["", "freebsd", "ANDROID", "solaris"] {
        assert!(
            matches!(
                save_target(os, Some(PathBuf::from("/downloads"))),
                Err(ShellError::DownloadsUnreachable)
            ),
            "{os:?} was never reasoned about and must not be assumed saveable"
        );
    }
}

#[test]
fn a_desktop_with_no_downloads_folder_is_still_a_plain_failure() {
    assert!(matches!(save_target("linux", None), Err(ShellError::Io(_))));
}

// ── The page names a FILE, never a place ─────────────────────────────────────────────────────────

#[test]
fn keeps_a_plain_file_name_as_it_is() {
    for raw in [
        "factura-2026-0042.pdf",
        "hub.blueprint.zip",
        "menú del día.png",
        "ticket (copia).pdf",
    ] {
        assert_eq!(
            download_file_name(raw).as_deref(),
            Some(raw),
            "{raw} is a perfectly ordinary file name"
        );
    }
}

#[test]
fn trims_the_whitespace_a_copy_paste_brings_along() {
    assert_eq!(
        download_file_name("  informe.pdf \n").as_deref(),
        Some("informe.pdf")
    );
}

#[test]
fn refuses_anything_that_names_a_PLACE_instead_of_a_file() {
    // The whole point of the frontier: whatever the page sends, the file lands in Downloads. A name
    // that carries a separator is a name that is trying to choose a directory.
    for raw in [
        "../../.ssh/authorized_keys",
        "..\\..\\Windows\\System32\\evil.dll",
        "/etc/passwd",
        "C:\\Windows\\notepad.exe",
        "subfolder/report.pdf",
        "subfolder\\report.pdf",
        "..",
        ".",
        "...",
    ] {
        assert_eq!(
            download_file_name(raw),
            None,
            "{raw:?} names a place, not a file"
        );
    }
}

#[test]
fn refuses_the_characters_that_make_a_file_something_else() {
    for raw in [
        // NTFS alternate data stream: `report.pdf:hidden.exe` writes a stream nobody sees listed.
        "report.pdf:hidden.exe",
        // A NUL truncates the name at the syscall boundary — the file written is not the file named.
        "report\0.pdf",
        // Control characters make the reported path unreadable, and the user is told a path.
        "report\r\n.pdf",
        "report\t.pdf",
    ] {
        assert_eq!(download_file_name(raw), None, "{raw:?} must be refused");
    }
}

#[test]
fn refuses_a_name_that_is_not_a_name() {
    for raw in ["", "   ", "\t\n"] {
        assert_eq!(download_file_name(raw), None, "{raw:?} must be refused");
    }
}

#[test]
fn refuses_a_name_no_file_system_would_accept() {
    // 255 bytes is the leaf-name limit on ext4, APFS and NTFS alike. Truncating would save a file
    // under a name the user was never shown; refusing says so instead.
    let long = format!("{}.pdf", "a".repeat(252));
    assert_eq!(long.len(), 256);
    assert_eq!(download_file_name(&long), None);

    let limit = format!("{}.pdf", "a".repeat(251));
    assert_eq!(limit.len(), 255);
    assert_eq!(download_file_name(&limit).as_deref(), Some(limit.as_str()));
}

// ── Saving twice never destroys the first copy ───────────────────────────────────────────────────

/// `free_download_path` asks the world whether a path is taken; the tests answer from a list, so
/// the naming rule is exercised without a temporary directory.
fn taken<'a>(names: &'a [&'a str]) -> impl Fn(&Path) -> bool + 'a {
    move |path: &Path| {
        path.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| names.contains(&n))
    }
}

#[test]
fn uses_the_name_as_given_when_nothing_is_in_the_way() {
    assert_eq!(
        free_download_path(Path::new("/downloads"), "informe.pdf", &taken(&[])),
        Some(PathBuf::from("/downloads/informe.pdf"))
    );
}

#[test]
fn never_overwrites_the_copy_that_is_already_there() {
    // Exporting a backup twice must leave TWO backups. Same convention the browsers use, so the
    // file is where the user expects to look for it.
    assert_eq!(
        free_download_path(
            Path::new("/downloads"),
            "hub.zip",
            &taken(&["hub.zip", "hub (2).zip"])
        ),
        Some(PathBuf::from("/downloads/hub (3).zip"))
    );
}

#[test]
fn counts_from_the_LAST_dot_so_the_extension_survives() {
    // `2026.08.08-backup.zip` split at the first dot would become `2026 (2).08.08-backup.zip`,
    // which no longer reads as a date and no longer sorts next to its sibling.
    assert_eq!(
        free_download_path(
            Path::new("/downloads"),
            "2026.08.08-backup.zip",
            &taken(&["2026.08.08-backup.zip"])
        ),
        Some(PathBuf::from("/downloads/2026.08.08-backup (2).zip"))
    );
}

#[test]
fn a_name_without_an_extension_still_gets_its_number() {
    assert_eq!(
        free_download_path(Path::new("/downloads"), "LICENSE", &taken(&["LICENSE"])),
        Some(PathBuf::from("/downloads/LICENSE (2)"))
    );
}

#[test]
fn a_dot_file_keeps_its_leading_dot() {
    // `.env` has no stem before the dot; treating "" as the stem would produce ` (2).env`, whose
    // name starts with a space.
    assert_eq!(
        free_download_path(Path::new("/downloads"), ".env", &taken(&[".env"])),
        Some(PathBuf::from("/downloads/.env (2)"))
    );
}

#[test]
fn gives_up_instead_of_spinning_forever() {
    // A directory where every candidate is taken is not a reason to loop: the page is told the file
    // could not be saved, which is the contract this whole module exists to keep.
    assert_eq!(
        free_download_path(Path::new("/downloads"), "a.pdf", &|_| true),
        None
    );
}

// ── The command stays wired to both frontiers ────────────────────────────────────────────────────

#[test]
fn the_save_command_checks_the_platform_and_the_name_before_it_writes() {
    // Neither check is reachable from a unit test — the command needs a live `AppHandle` — and
    // neither shows up as a type error when deleted. Drop the platform check and an Android till
    // reports a path into storage no file manager will open; drop the name check and the page
    // chooses where on the disk to write. So the guard is on the source, where the deletion happens.
    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
        .expect("src/lib.rs");

    let command = source
        .find("fn save_download")
        .map(|at| &source[at..])
        .and_then(|rest| rest.find("\n}").map(|end| &rest[..end]))
        .expect("src/lib.rs no longer defines `save_download` (hub#480)");

    assert!(
        command.contains("save_target"),
        "`save_download` no longer asks WHERE this platform saves: on Android the folder Tauri \
         resolves is app-scoped storage no file manager opens, and reporting a path there is \
         hub#475 with a success message on top (hub#480, ADR-0259, hub#499)."
    );
    assert!(
        command.contains("download_file_name"),
        "`save_download` no longer reduces what the page sent to a single file name: the page gets \
         to choose a path on the till's disk (hub#480, ADR-0259)."
    );
}

#[test]
fn the_android_branch_still_goes_through_the_native_publisher() {
    // The Android half cannot be reached from a unit test — it needs a device with a `MediaStore` —
    // and its disappearance is not a type error either: delete the call and the `#[cfg]` block
    // simply stops publishing, `save_target` keeps answering `AndroidDownloads`, and every test
    // above stays green while the tablet is back to not saving anything (hub#499).
    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
        .expect("src/lib.rs");

    assert!(
        source.contains("save_to_downloads"),
        "nothing in the shell calls the plugin's `save_to_downloads` any more: on Android there is \
         no other way to reach the public Downloads collection, so the till silently loses its \
         backup, its invoices and its `/files` downloads (hub#499)."
    );
}
