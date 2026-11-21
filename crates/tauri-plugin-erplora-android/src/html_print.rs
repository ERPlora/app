//! The A4 document printed through Android's own print service.

use serde::Serialize;
use tauri::Runtime;

use crate::{Error, ErploraAndroid};

/// The command that opens Android's print screen with an A4 document. Mirror of
/// `ErploraAndroidPlugin.printHtml`; a test below checks the two never drift apart.
pub const PRINT_HTML_COMMAND: &str = "printHtml";

/// An A4 document on its way to Kotlin: its html, already checked by the shell. A
/// string and not a path: an invoice is tens of KB, and the shell caps it well below what the
/// bridge carries.
#[derive(Debug, Serialize)]
struct PrintHtmlArgs {
    html: String,
}

/// What `printHtml` answers with. Kotlin's `invoke.resolve()` with no data reaches Rust as JSON
/// `null`, so the answer is read and ignored whatever its shape — reading it as [`Empty`](crate::Empty) turned a
/// print screen that DID open into a failure.
type PrintHtmlAnswer = serde::de::IgnoredAny;

impl<R: Runtime> ErploraAndroid<R> {
    /// Opens Android's print screen with `html`, preset to A4: every printer the device knows and
    /// «Save as PDF». Resolves once the screen has been asked for; what the user does
    /// in it is the system's.
    ///
    /// ⚠️ **Blocks**, like [`Self::save_to_downloads`]: call it from an `async` command, never
    /// from the main thread.
    pub fn print_html(&self, html: &str) -> Result<(), Error> {
        #[cfg(target_os = "android")]
        {
            return self
                .0
                .run_mobile_plugin::<PrintHtmlAnswer>(
                    PRINT_HTML_COMMAND,
                    PrintHtmlArgs {
                        html: html.to_string(),
                    },
                )
                .map(|_| ())
                .map_err(|e| Error::PluginInvoke(e.to_string()));
        }
        #[cfg(not(target_os = "android"))]
        {
            // Unreachable by construction: the shell only routes here on Android. A refusal all
            // the same, because a desktop that got here has opened no print screen.
            let _ = html;
            Err(Error::PluginInvoke("print_html is Android only".into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    // ── The A4 document through Android's own print service ──────────────────────────────────
    //
    // The WebView that prints lives in Kotlin (`HtmlPrinter.kt`); Rust owns the command name and
    // the argument key, both bare literals on the two sides.

    const HTML_PRINTER_KT: &str =
        include_str!("../android/src/main/java/com/erplora/android/HtmlPrinter.kt");

    #[test]
    fn the_a4_document_crosses_to_kotlin_under_the_key_kotlin_reads() {
        let json = serde_json::to_value(PrintHtmlArgs {
            html: "<p>F-1</p>".into(),
        })
        .expect("serializable");
        assert_eq!(json["html"], "<p>F-1</p>");

        let code = without_kotlin_comments(ERPLORA_ANDROID_PLUGIN_KT);
        for literal in [
            "getString(\"html\"".to_string(),
            format!("fun {PRINT_HTML_COMMAND}(invoke: Invoke)"),
        ] {
            assert!(
                code.contains(&literal),
                "{literal} is not in ErploraAndroidPlugin.kt — the invoice would reach Kotlin as \
                 `command not found` or with no document, on a real device only"
            );
        }
    }

    #[test]
    fn an_answer_with_no_data_is_a_print_screen_that_opened() {
        // `invoke.resolve()` with no data arrives as `null` (and a `JSObject()` as `{}`): both
        // mean the print screen opened. Reading either as an error sent the invoice to the till
        // roll right behind the dialog the user was looking at.
        for answer in [serde_json::Value::Null, serde_json::json!({})] {
            assert!(
                serde_json::from_value::<PrintHtmlAnswer>(answer.clone()).is_ok(),
                "{answer} must read as success"
            );
        }
    }

    #[test]
    fn the_printed_document_runs_no_code() {
        // The same frontier as the desktop print window's CSP: the html comes from a
        // remote page, so the WebView that renders it runs no script and exposes no bridge.
        let code = without_kotlin_comments(HTML_PRINTER_KT);
        assert!(code.contains("javaScriptEnabled = false"), "{code}");
        assert!(!code.contains("javaScriptEnabled = true"), "{code}");
        assert!(!code.contains("addJavascriptInterface"), "{code}");
    }

    #[test]
    fn the_printed_document_is_loaded_with_a_page_margin() {
        // «Save as PDF» lets the WebView print right up to the paper's edge. `withDefaultPageMargin`
        // (HtmlPrinterTest.kt) gives a page without a margin of its own the 1 cm a browser leaves;
        // this pins that the WebView loads the document through it, which no JVM test can see.
        let code = without_kotlin_comments(HTML_PRINTER_KT);
        assert!(
            code.contains("loadDataWithBaseURL(null, withDefaultPageMargin(html),"),
            "{code}"
        );
    }

    #[test]
    fn desktop_refuses_to_print_through_the_android_plugin() {
        // Unreachable by construction (the shell only routes here on Android), and a refusal
        // anyway: a resolved call would read as a dialog that opened.
        let desktop = ErploraAndroid::<tauri::Wry>(std::marker::PhantomData);
        assert!(desktop.print_html("<p>F-1</p>").is_err());
    }
}
