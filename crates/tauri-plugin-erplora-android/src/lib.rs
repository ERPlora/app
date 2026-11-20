//! The app's native Android side: runtime permissions, Bluetooth and NFC, the system voice, the
//! Downloads collection, the print service and the notices.
//!
//! Declaring a permission in the manifest is not enough, and the two the till needs fail
//! **silently**:
//!
//! - without `ACCESS_LOCAL_NETWORK` (API 37+) the port 9100 sweep is 254 **timeouts**, so discovery
//!   returns `[]` and the venue looks as if it had no printers;
//! - without `POST_NOTIFICATIONS` (API 33+) the notification never shows, and the order reaches the
//!   kitchen without anybody noticing.
//!
//! On desktop there is nothing to ask for: the commands exist all the same and answer «granted», so
//! the page can call them without branching per platform.


use serde::{Deserialize, Serialize};
use tauri::{
    ipc::Invoke,
    plugin::{Builder, TauriPlugin},
    Manager, Runtime,
};

mod bluetooth;
mod downloads;
mod html_print;
mod hub_question;
mod lifecycle;
mod nfc;
mod notices;
mod permissions;
mod voice;

pub use bluetooth::{BluetoothPrinter, BluetoothPrinterList, BLUETOOTH_PERMISSION_DENIED};
pub use downloads::{PublishedDownload, DOWNLOADS_UNREACHABLE};
pub use html_print::PRINT_HTML_COMMAND;
pub use hub_question::{Wording, ASK_TO_OPEN_HUB_COMMAND};
pub use nfc::{NfcBadgeRead, NFC_DEFAULT_TIMEOUT_MS, NFC_DISABLED, NFC_RANDOM_UID, NFC_UNAVAILABLE};
pub use notices::{LaunchNoticeTap, REMEMBER_NOTICE_COMMAND, WATCH_NOTICE_TAPS_COMMAND};
pub use permissions::{PermissionStatus, ACCESS_LOCAL_NETWORK, BLUETOOTH_CONNECT, POST_NOTIFICATIONS};
pub use voice::{TtsAvailability, TTS_FAILED, TTS_UNAVAILABLE};

use voice::{TtsLanguageArgs, TtsSpeakArgs};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    PluginInvoke(String),
    /// This Android is older than the public Downloads collection (API 29), so there is nowhere to
    /// put a file that the user could then open. The shell turns this into its own sentence.
    #[error("downloads_unreachable")]
    DownloadsUnreachable,
    /// No NFC reader on this device — a desktop, or a tablet sold without one.
    #[error("nfc_unavailable")]
    NfcUnavailable,
    /// There is a reader and it is switched off. The only one of the three the user can fix.
    #[error("nfc_disabled")]
    NfcDisabled,
    /// The card randomises its id on every tap, so it can never be matched again after enrolment.
    #[error("nfc_random_uid")]
    NfcRandomUid,
    /// No engine for the system voice, or no voice for the language: every desktop by construction.
    #[error("tts_unavailable")]
    TtsUnavailable,
    /// The system voice broke while reading.
    #[error("tts_failed")]
    TtsFailed,
}

impl Serialize for Error {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Empty {}

#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "com.erplora.android";

/// The plugin's state: the handle of the Kotlin module on Android, nothing on desktop.
#[cfg(target_os = "android")]
pub struct ErploraAndroid<R: Runtime>(tauri::plugin::PluginHandle<R>);

#[cfg(not(target_os = "android"))]
pub struct ErploraAndroid<R: Runtime>(std::marker::PhantomData<fn() -> R>);

pub trait ErploraAndroidExt<R: Runtime> {
    fn erplora_android(&self) -> &ErploraAndroid<R>;
}

impl<R: Runtime, T: Manager<R>> ErploraAndroidExt<R> for T {
    fn erplora_android(&self) -> &ErploraAndroid<R> {
        self.state::<ErploraAndroid<R>>().inner()
    }
}

/// Leaves the app the way the system Back does on a root screen (the task goes to
/// the background; nothing is killed). The shell calls it when it holds the Back button and there
/// is nothing left to close nor to go back to. On desktop there is no such button: a no-op.
#[tauri::command]
async fn leave_app<R: Runtime>(app: tauri::AppHandle<R>) -> Result<(), Error> {
    app.erplora_android().leave_app()
}

/// Takes the owner to this app's page in the device settings. The shell only offers it
/// on a device that really has a permission refused, so on desktop it is never called: a no-op.
#[tauri::command]
async fn open_app_settings<R: Runtime>(app: tauri::AppHandle<R>) -> Result<(), Error> {
    app.erplora_android().open_app_settings()
}

/// Whether the system voice reads `language` (`es-ES`): `{ available }`. The
/// first call waits for the engine to start, so it waits on a blocking thread.
#[tauri::command]
async fn tts_available<R: Runtime>(app: tauri::AppHandle<R>, language: String) -> Result<TtsAvailability, Error> {
    tauri::async_runtime::spawn_blocking(move || app.erplora_android().tts_available(TtsLanguageArgs { language }))
        .await
        .map_err(|e| Error::PluginInvoke(e.to_string()))?
        .map(|available| TtsAvailability { available })
}

/// Reads `text` aloud in the system voice and answers when it was read or
/// stopped. The wait is the whole reading, so it happens on a blocking thread, never on an async
/// worker the rest of the app's commands need.
#[tauri::command]
async fn tts_speak<R: Runtime>(app: tauri::AppHandle<R>, text: String, language: String) -> Result<(), Error> {
    tauri::async_runtime::spawn_blocking(move || app.erplora_android().tts_speak(TtsSpeakArgs { text, language }))
        .await
        .map_err(|e| Error::PluginInvoke(e.to_string()))?
}

/// Stops the system voice; the pending `tts_speak` answers as ended.
#[tauri::command]
async fn tts_stop<R: Runtime>(app: tauri::AppHandle<R>) -> Result<(), Error> {
    app.erplora_android().tts_stop()
}

#[tauri::command]
async fn check_permissions<R: Runtime>(app: tauri::AppHandle<R>) -> Result<PermissionStatus, Error> {
    app.erplora_android().check_permissions()
}

#[tauri::command]
async fn request_permissions<R: Runtime>(
    app: tauri::AppHandle<R>,
    permissions: Option<Vec<String>>,
) -> Result<PermissionStatus, Error> {
    app.erplora_android().request_permissions(permissions)
}

/// Pins the type of the generated handler so it can be called from the gated one below.
fn handler<R: Runtime, F: Fn(Invoke<R>) -> bool>(commands: F) -> F {
    commands
}

/// The plugin, with `gate` in front of every one of its commands. The capability grants
/// `erplora-android:default` to every page under erplora.com; the app hands in the gate that lets
/// only the LINKED hub through (`hub_link::linked_hub_only`), and a page it refuses gets the error
/// code the gate returns, with no command run.
pub fn init<R, G>(gate: G) -> TauriPlugin<R>
where
    R: Runtime,
    G: Fn(&Invoke<R>) -> Result<(), &'static str> + Send + Sync + 'static,
{
    let commands = handler::<R, _>(tauri::generate_handler![
        check_permissions,
        request_permissions,
        leave_app,
        open_app_settings,
        tts_available,
        tts_speak,
        tts_stop
    ]);
    Builder::new("erplora-android")
        .invoke_handler(move |invoke: Invoke<R>| {
            if let Err(code) = gate(&invoke) {
                invoke.resolver.reject(code);
                // `true` on purpose: on Android, a plugin command the Rust side leaves unhandled
                // falls through to the Kotlin plugin (Tauri's `run_command`), which would run it.
                return true;
            }
            commands(invoke)
        })
        .setup(|app, _api| {
            #[cfg(target_os = "android")]
            let handle = _api.register_android_plugin(PLUGIN_IDENTIFIER, "ErploraAndroidPlugin")?;
            #[cfg(target_os = "android")]
            app.manage(ErploraAndroid(handle));

            #[cfg(not(target_os = "android"))]
            app.manage(ErploraAndroid::<R>(std::marker::PhantomData));

            Ok(())
        })
        .build()
}

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    #[test]
    fn the_error_reaches_the_frontend_as_plain_text() {
        // The same as the rest of the shell: the promise is rejected with a readable message.
        let e = Error::PluginInvoke("sin actividad".into());
        assert_eq!(serde_json::to_string(&e).unwrap(), "\"sin actividad\"");
    }

    // ── Kotlin answers with no data ─────────────────────────────────────────────────────────
    //
    // `invoke.resolve()` with no data reaches Rust as JSON `null`. Read as a struct, that `null`
    // is a deserialize error, so a call that DID its job came back as a failure: the ticket was
    // on paper and the print host reported it `failed`.

    /// Commands whose Kotlin side resolves with a bare `invoke.resolve()` somewhere.
    fn kotlin_commands_resolving_with_no_data(kotlin: &str) -> Vec<String> {
        let code = without_kotlin_comments(kotlin);
        let mut names = Vec::new();
        let mut rest = code.as_str();
        while let Some(at) = rest.find("(invoke: Invoke)") {
            let name = rest[..at]
                .rsplit(|c: char| !c.is_alphanumeric() && c != '_')
                .next()
                .unwrap_or_default()
                .to_string();
            rest = &rest[at + "(invoke: Invoke)".len()..];
            let body_end = rest.find("(invoke: Invoke)").unwrap_or(rest.len());
            if rest[..body_end].contains("invoke.resolve()") {
                names.push(name);
            }
        }
        names
    }

    /// The type each `run_mobile_plugin::<T>(command, …)` reads its answer as, by command name.
    fn rust_answer_types(rust: &str) -> Vec<(String, String)> {
        const CALL: &str = "run_mobile_plugin::<";
        let mut found = Vec::new();
        let mut rest = rust;
        while let Some(at) = rest.find(CALL) {
            rest = &rest[at + CALL.len()..];
            let Some(close) = rest.find(">(") else { break };
            let ty = rest[..close].trim().to_string();
            let arg = rest[close + 2..].trim_start();
            let command = if let Some(quoted) = arg.strip_prefix('"') {
                quoted.split('"').next().unwrap_or_default().to_string()
            } else {
                let ident: String = arg
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                let decl = format!("const {ident}: &str = \"");
                rust.split(&decl)
                    .nth(1)
                    .and_then(|v| v.split('"').next())
                    .unwrap_or_default()
                    .to_string()
            };
            found.push((command, ty));
        }
        found
    }

    #[test]
    fn every_command_kotlin_resolves_with_no_data_is_read_whatever_its_shape() {
        let rust = production_rust();
        let empty_answers = kotlin_commands_resolving_with_no_data(ERPLORA_ANDROID_PLUGIN_KT);
        // Positive control: the scan sees the two commands known to answer with nothing.
        for known in ["bluetoothPrint", "printHtml"] {
            assert!(
                empty_answers.iter().any(|n| n == known),
                "{known} not found by the scan: {empty_answers:?}"
            );
        }
        let answers = rust_answer_types(&rust);
        for command in &empty_answers {
            let ty = answers
                .iter()
                .find(|(c, _)| c == command)
                .map(|(_, t)| t.as_str())
                .unwrap_or_else(|| panic!("no run_mobile_plugin::<T> call for {command}"));
            let reads_anything = ty == "serde::de::IgnoredAny"
                || rust.contains(&format!("type {ty} = serde::de::IgnoredAny;"));
            assert!(
                reads_anything,
                "{command} answers `null` from Kotlin but Rust reads it as {ty}: a call that did \
                 its job would come back as a failure"
            );
        }
    }
}
