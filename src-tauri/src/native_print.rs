//! `print_document` — the SYSTEM print dialog for an A4 document the page holds (hub#2006).
//!
//! Inside the webview `window.print()` prints nothing (hub#862), so an invoice only had the till
//! roll. Tauri 2 already prints a webview natively (`Webview::print`: `NSPrintOperation` on macOS,
//! the WebView2 dialog on Windows), so nothing is rendered to PDF here: the shell opens a window
//! whose ONLY content is the document, served from memory under [`PRINT_SCHEME`], and once it has
//! loaded asks the OS to print it. The printer list and «Save as PDF» are the system's.
//!
//! The document is html from a remote page shown in a window of the app, so it is contained three
//! ways: its response forbids every script ([`PRINT_DOCUMENT_CSP`]), no capability names the print
//! window (no IPC), and the window cannot navigate anywhere else ([`print_window_may_navigate`]).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use crate::ShellError;

/// The custom scheme the print window loads its document from.
pub const PRINT_SCHEME: &str = "erplora-print";

/// Largest document taken. An A4 invoice with an inline logo and QR is tens of KB; this only keeps
/// a runaway page from parking megabytes in the shell's memory.
pub const MAX_PRINT_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;

/// The document runs no code: no script source at all. Styles are inline in the document, and its
/// QR and logo arrive as `data:` or `https:` images.
pub const PRINT_DOCUMENT_CSP: &str =
    "default-src 'none'; style-src 'unsafe-inline'; img-src data: blob: https:; font-src data: https:";

/// Does this platform have a system print dialog for a webview? `os` is `std::env::consts::OS`.
///
/// wry implements `print` on macOS, Windows and Linux. On Android it does not, so the shell hands
/// the document to Android's own print service through its plugin instead (hub#2008). Anywhere
/// else the print door keeps its usual route.
pub fn native_print_supported(os: &str) -> bool {
    matches!(os, "macos" | "windows" | "linux" | "android")
}

/// Is `html` a document the shell will hand to a print dialog? Refuses an empty or oversized one:
/// nothing to print, or a runaway page.
pub fn check_print_document(html: &str) -> Result<(), ShellError> {
    if html.trim().is_empty() || html.len() > MAX_PRINT_DOCUMENT_BYTES {
        return Err(ShellError::PrintDocumentRefused);
    }
    Ok(())
}

/// The address of document `id` in the print window. WebView2 and the Android WebView only load a
/// custom scheme as `http://<scheme>.localhost`; WKWebView and WebKitGTK load it as is.
pub fn print_document_url(os: &str, id: u64) -> String {
    match os {
        "windows" | "android" => format!("http://{PRINT_SCHEME}.localhost/{id}"),
        _ => format!("{PRINT_SCHEME}://localhost/{id}"),
    }
}

/// The document a request path names: `/7` (or `/7/`) and nothing else.
pub fn print_document_id(path: &str) -> Option<u64> {
    let rest = path.strip_prefix('/')?;
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    rest.parse().ok()
}

/// The label of the window that prints document `id`. Never `main`, and no capability names it.
pub fn print_window_label(id: u64) -> String {
    format!("print-{id}")
}

/// May the print window go to `url`? Only to its own document: a link inside the invoice must not
/// turn the print window into a browser.
pub fn print_window_may_navigate(url: &str) -> bool {
    url.starts_with(&format!("{PRINT_SCHEME}://localhost/"))
        || url.starts_with(&format!("http://{PRINT_SCHEME}.localhost/"))
}

/// The documents waiting in (or shown by) a print window, by id. A document stays until its window
/// is destroyed: a webview may ask for it again, and a 404 there prints a blank page.
#[derive(Default)]
pub struct PrintDocuments {
    next: AtomicU64,
    docs: Mutex<HashMap<u64, String>>,
}

impl PrintDocuments {
    /// Takes a document and answers its id. Refuses an empty or oversized one: nothing to print.
    pub fn insert(&self, html: String) -> Result<u64, ShellError> {
        check_print_document(&html)?;
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        self.lock().insert(id, html);
        Ok(id)
    }

    pub fn get(&self, id: u64) -> Option<String> {
        self.lock().get(&id).cloned()
    }

    pub fn remove(&self, id: u64) {
        self.lock().remove(&id);
    }

    // A poisoned lock only means another thread panicked mid-insert; the map itself is still whole.
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, String>> {
        self.docs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The response to a request for `path` under [`PRINT_SCHEME`].
    pub fn respond(&self, path: &str) -> tauri::http::Response<Vec<u8>> {
        let doc = print_document_id(path).and_then(|id| self.get(id));
        let builder = tauri::http::Response::builder();
        let response = match doc {
            Some(html) => builder
                .status(200)
                .header("Content-Type", "text/html; charset=utf-8")
                .header("Content-Security-Policy", PRINT_DOCUMENT_CSP)
                .body(html.into_bytes()),
            None => builder.status(404).body(Vec::new()),
        };
        response.unwrap_or_else(|_| tauri::http::Response::new(Vec::new()))
    }
}

/// Opens a window with the document and hands it to the system print dialog once it has loaded.
///
/// The window stays open behind the dialog as the preview of what is printed; the user closes it
/// like any other window, and its document is dropped with it.
#[cfg(desktop)]
pub fn open_print_window<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    html: String,
) -> Result<(), ShellError> {
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use tauri::webview::PageLoadEvent;
    use tauri::{Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

    let docs = app.state::<PrintDocuments>();
    let id = docs.insert(html)?;
    let url = print_document_url(std::env::consts::OS, id)
        .parse()
        .map_err(|e| ShellError::NativePrintFailed(format!("{e}")))?;

    // Once per window: a second `Finished` (a repaint, a reload) must not stack a second dialog.
    let printed = Arc::new(AtomicBool::new(false));
    let built =
        WebviewWindowBuilder::new(app, print_window_label(id), WebviewUrl::CustomProtocol(url))
            .title("ERPlora")
            .inner_size(820.0, 1060.0)
            .on_navigation(|url| print_window_may_navigate(url.as_str()))
            .on_page_load(move |window, payload| {
                if payload.event() == PageLoadEvent::Finished
                    && !printed.swap(true, Ordering::SeqCst)
                {
                    if let Err(e) = window.print() {
                        log::warn!("shell: the system print dialog did not open: {e}");
                    }
                }
            })
            .build();

    let window = match built {
        Ok(window) => window,
        Err(e) => {
            docs.remove(id);
            return Err(ShellError::NativePrintFailed(e.to_string()));
        }
    };
    let handle = app.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::Destroyed = event {
            handle.state::<PrintDocuments>().remove(id);
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_the_document_with_a_csp_that_runs_no_code() {
        let docs = PrintDocuments::default();
        let id = docs.insert("<p>F-1</p>".into()).unwrap_or_default();
        let res = docs.respond(&format!("/{id}"));
        assert_eq!(res.status(), 200);
        assert_eq!(res.body().as_slice(), b"<p>F-1</p>");
        assert_eq!(
            res.headers()
                .get("Content-Security-Policy")
                .and_then(|v| v.to_str().ok()),
            Some(PRINT_DOCUMENT_CSP)
        );
    }

    #[test]
    fn an_unknown_document_is_a_404() {
        let docs = PrintDocuments::default();
        assert_eq!(docs.respond("/99").status(), 404);
        assert_eq!(docs.respond("/../x").status(), 404);
    }

    #[test]
    fn the_print_window_stays_on_its_document() {
        assert!(print_window_may_navigate("erplora-print://localhost/3"));
        assert!(print_window_may_navigate(
            "http://erplora-print.localhost/3"
        ));
        assert!(!print_window_may_navigate("https://evil.example/"));
        assert!(!print_window_may_navigate(
            "http://erplora-print.localhost.evil.example/3"
        ));
    }
}
