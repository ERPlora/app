//! The boundary of `print_document` (hub#2006): an A4 document the page holds, shown in a window of
//! the shell's own and handed to the SYSTEM print dialog — the printer list and «Save as PDF» are
//! the operating system's, not ours.
//!
//! Inside the webview `window.print()` prints nothing (hub#862), so an invoice only had the till
//! roll. Tauri 2 already prints a webview natively (`Webview::print`: `NSPrintOperation` on macOS,
//! the WebView2 dialog on Windows); what the shell adds is a window whose ONLY content is the
//! document, served from memory under its own scheme. Three frontiers come with that:
//!
//! 1. **The document runs no code.** It is html from a remote page, shown in a window of the app:
//!    its response carries a CSP with `default-src 'none'` and no script source at all.
//! 2. **The print window has no IPC.** No capability names it, so nothing inside it can `invoke`.
//! 3. **Desktop and Android have the dialog.** Android's comes from its own print service
//!    (`PrintManager`, hub#2008); any other platform answers `native_print_unsupported` and the
//!    print door keeps its usual route.

use erplora_tauri_lib::{
    native_print_supported, print_document_id, print_document_url, print_window_label,
    PrintDocuments, ShellError, MAX_PRINT_DOCUMENT_BYTES, PRINT_DOCUMENT_CSP,
};

#[test]
fn the_desktop_has_a_system_print_dialog() {
    for os in ["macos", "windows", "linux"] {
        assert!(native_print_supported(os), "{os} prints a webview natively");
    }
}

#[test]
fn android_has_the_system_print_service() {
    // wry has no `print` on Android, so the shell hands the document to Android's own
    // `PrintManager` through its plugin: printer list and «Save as PDF» (hub#2008).
    assert!(native_print_supported("android"));
}

#[test]
fn ios_has_no_native_print_yet() {
    // No shell half on iOS: it must refuse, and the door keeps its route there.
    assert!(!native_print_supported("ios"), "ios must refuse, not pretend");
}

#[test]
fn the_document_url_follows_the_platform_custom_scheme_form() {
    // WebView2 (Windows) and the Android WebView only load custom schemes as `http://<scheme>.localhost`.
    assert_eq!(
        print_document_url("windows", 7),
        "http://erplora-print.localhost/7"
    );
    assert_eq!(
        print_document_url("android", 7),
        "http://erplora-print.localhost/7"
    );
    assert_eq!(
        print_document_url("macos", 7),
        "erplora-print://localhost/7"
    );
    assert_eq!(
        print_document_url("linux", 7),
        "erplora-print://localhost/7"
    );
}

#[test]
fn the_request_path_names_the_document() {
    assert_eq!(print_document_id("/7"), Some(7));
    assert_eq!(print_document_id("/42/"), Some(42));
    assert_eq!(print_document_id("/"), None);
    assert_eq!(print_document_id("/../etc/passwd"), None);
    assert_eq!(print_document_id("/7?x=1"), None);
    // One spelling per document: `+7` would parse as 7 and alias it.
    assert_eq!(print_document_id("/+7"), None);
}

#[test]
fn a_stored_document_is_served_until_its_window_is_gone() {
    let docs = PrintDocuments::default();
    let id = docs
        .insert("<p>F-1</p>".into())
        .expect("a small document is taken");
    assert_eq!(docs.get(id).as_deref(), Some("<p>F-1</p>"));
    // Served twice: a webview may ask again (a reload, a second paint), and a 404 there prints blank.
    assert_eq!(docs.get(id).as_deref(), Some("<p>F-1</p>"));
    docs.remove(id);
    assert_eq!(docs.get(id), None);
}

#[test]
fn each_document_gets_its_own_id_and_window() {
    let docs = PrintDocuments::default();
    let a = docs.insert("<p>a</p>".into()).unwrap_or_default();
    let b = docs.insert("<p>b</p>".into()).unwrap_or_default();
    assert_ne!(a, b);
    assert_ne!(print_window_label(a), print_window_label(b));
    assert!(print_window_label(a).starts_with("print-"));
}

#[test]
fn an_empty_document_is_refused() {
    let docs = PrintDocuments::default();
    assert!(matches!(
        docs.insert("   ".into()),
        Err(ShellError::PrintDocumentRefused)
    ));
}

#[test]
fn an_oversized_document_is_refused() {
    let docs = PrintDocuments::default();
    let huge = "x".repeat(MAX_PRINT_DOCUMENT_BYTES + 1);
    assert!(matches!(
        docs.insert(huge),
        Err(ShellError::PrintDocumentRefused)
    ));
}

#[test]
fn the_document_runs_no_code() {
    let csp = PRINT_DOCUMENT_CSP;
    assert!(csp.contains("default-src 'none'"), "{csp}");
    assert!(
        !csp.contains("script-src"),
        "no script source may be allowed: {csp}"
    );
    // The invoice carries its styles inline and its QR/logo as data: or https images.
    assert!(csp.contains("style-src 'unsafe-inline'"), "{csp}");
    assert!(csp.contains("img-src"), "{csp}");
}

#[test]
fn no_capability_reaches_the_print_window() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("capabilities");
    for entry in std::fs::read_dir(&dir).expect("capabilities dir") {
        let path = entry.expect("entry").path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let raw = std::fs::read_to_string(&path).expect("capability file");
        let json: serde_json::Value = serde_json::from_str(&raw).expect("capability json");
        let windows: Vec<&str> = json["windows"]
            .as_array()
            .map(|w| w.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        assert!(
            windows
                .iter()
                .all(|w| !w.contains('*') && !w.starts_with("print-")),
            "{} hands IPC to the print window: {windows:?}",
            path.display()
        );
    }
}
