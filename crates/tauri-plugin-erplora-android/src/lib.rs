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

/// A permission's state as the page sees it: `{ "android.permission.X": true }`.
pub type PermissionStatus = std::collections::HashMap<String, bool>;

/// Local network access (API 37+). Without it the printer sweep is 254 silent timeouts.
///
/// Mirror of `PermissionPolicy.ACCESS_LOCAL_NETWORK` on the Kotlin side. The key of the map above
/// **is** the permission string, so Rust needs the same literal to read the answer — and a test
/// below checks the two never drift apart.
pub const ACCESS_LOCAL_NETWORK: &str = "android.permission.ACCESS_LOCAL_NETWORK";

/// System notifications (API 33+). Mirror of `PermissionPolicy.POST_NOTIFICATIONS`.
pub const POST_NOTIFICATIONS: &str = "android.permission.POST_NOTIFICATIONS";

/// Talking to bonded Bluetooth devices (API 31+): the SPP printer transport.
/// Mirror of `PermissionPolicy.BLUETOOTH_CONNECT`. Denied, the bonded list comes back empty and
/// the RFCOMM connect throws — the same silent-failure family as the two above.
pub const BLUETOOTH_CONNECT: &str = "android.permission.BLUETOOTH_CONNECT";

/// The code a bluetooth operation is refused under when `BLUETOOTH_CONNECT` is denied.
/// Mirror of `ErploraAndroidPlugin.BLUETOOTH_PERMISSION_DENIED` — same trip as
/// [`DOWNLOADS_UNREACHABLE`]: Kotlin rejects with a code, Tauri renders `[code] - message`, and
/// the shell recognises it out of the text.
pub const BLUETOOTH_PERMISSION_DENIED: &str = "bluetooth_permission_denied";

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

/// The code an Android with no public Downloads collection is refused under.
///
/// Mirror of `DownloadPublisher.DOWNLOADS_UNREACHABLE`, and the same word `src-tauri` and the
/// page's `save-download.ts` already read: it is the ONE refusal the user can act on, so it must survive
/// the trip from Kotlin to the page intact. A test below checks the two sides never drift apart.
pub const DOWNLOADS_UNREACHABLE: &str = "downloads_unreachable";

/// The system voice has no engine, or no voice for the language: the page reads
/// with the browser's voice if it has one. Mirror of `SystemVoice.TTS_UNAVAILABLE`.
pub const TTS_UNAVAILABLE: &str = "tts_unavailable";

/// The system voice broke while reading. Mirror of `SystemVoice.TTS_FAILED`.
pub const TTS_FAILED: &str = "tts_failed";

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

/// The scope of a `request_permissions` call: the permissions the OPERATION that asks is about to
/// use, or `None` when the caller sent none (a web older than the scope), which keeps asking for
/// everything so the app and the web, shipped separately, cannot break each other.
#[derive(Debug, Serialize)]
struct RequestPermissionsArgs {
    permissions: Option<Vec<String>>,
}

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

/// A bluetooth print job on its way to Kotlin: the MAC of a bonded printer plus the
/// ALREADY-RENDERED ESC/POS bytes. base64 and not a JSON array of numbers, for the same reason as
/// `save_download`: a ticket is ~1.3× its size this way and ~4× the other, on a tablet.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BluetoothPrintArgs {
    mac: String,
    payload_base64: String,
}

impl BluetoothPrintArgs {
    fn new(mac: &str, payload: &[u8]) -> Self {
        use base64::Engine as _;
        Self {
            mac: mac.to_string(),
            payload_base64: base64::engine::general_purpose::STANDARD.encode(payload),
        }
    }
}

/// What `bluetoothPrint` answers with once the bytes are on the printer: nothing. Kotlin's
/// `invoke.resolve()` with no data reaches Rust as JSON `null`, so it is read and ignored whatever
/// its shape — reading it as [`Empty`] reported a ticket already on paper as `failed`.
type BluetoothPrintAnswer = serde::de::IgnoredAny;

/// A bonded printer as Kotlin announces it: `id` is `bluetooth:{MAC}` (the `printer_id` the
/// peripherals crate parses), `mac` the raw identity the registry keys on.
#[derive(Debug, Clone, Deserialize)]
pub struct BluetoothPrinter {
    pub id: String,
    pub name: String,
    pub mac: String,
}

/// The wire shape of `bluetooth_bonded_printers`.
#[derive(Debug, Deserialize)]
pub struct BluetoothPrinterList {
    pub printers: Vec<BluetoothPrinter>,
}

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
/// `null`, so the answer is read and ignored whatever its shape — reading it as [`Empty`] turned a
/// print screen that DID open into a failure.
type PrintHtmlAnswer = serde::de::IgnoredAny;

/// The question the app asks before a link links another hub, answered by an
/// `AlertDialog`. Mirror of `ErploraAndroidPlugin.askToOpenHub`; a test below checks the two never
/// drift apart. Only Rust calls it: it is in neither `build.rs` nor the default permissions, so no
/// page can show the question, let alone answer it.
pub const ASK_TO_OPEN_HUB_COMMAND: &str = "askToOpenHub";

/// The words of that question in one language: written by the app, shown by Kotlin as they come.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Wording {
    pub title: String,
    pub message: String,
    pub open: String,
    pub cancel: String,
}

/// Both languages travel: Kotlin picks the device's (Spanish unless it says otherwise).
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[derive(Debug, Serialize)]
struct AskToOpenHubArgs<'a> {
    en: &'a Wording,
    es: &'a Wording,
}

/// What `askToOpenHub` answers: `open` is «Open». An answer without it is a no.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[derive(Debug, Deserialize)]
struct AskToOpenHubAnswer {
    #[serde(default)]
    open: bool,
}

/// What the page asks of the listening service: `on`, and the words of the ongoing notification in
/// the app's language (the translations live with the page). Off, no words.
#[derive(Debug, Serialize)]
struct KeepListeningArgs {
    on: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    channel: Option<String>,
}

/// `keepListening` resolves with no data (`invoke.resolve()` → `null`): read it as anything.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
type KeepListeningAnswer = serde::de::IgnoredAny;

/// `leaveApp` resolves with no data (`invoke.resolve()` → `null`): read it as anything.
type LeaveAppAnswer = serde::de::IgnoredAny;

/// `openAppSettings` resolves with no data (`invoke.resolve()` → `null`): read it as anything.
#[cfg(target_os = "android")]
type OpenAppSettingsAnswer = serde::de::IgnoredAny;

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
/// Same trip as [`classify_publish_error`]: Tauri renders a plugin rejection as `[code] - message`
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

/// What the assistant asks the system voice to read: the text and its language
/// as a BCP 47 tag with its region (`es-ES`, `en-US`).
#[derive(Debug, Serialize)]
struct TtsSpeakArgs {
    text: String,
    language: String,
}

/// The language the page asks the system voice about.
#[derive(Debug, Serialize)]
struct TtsLanguageArgs {
    language: String,
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

/// The tap on a notice that STARTED the app: the notice's id, the JSON the notification plugin
/// stored it as, whose `extra.path` is the screen it leads to, and the screen the app wrote down
/// when it showed it — the plugin never stores that JSON, so `path` is what brings the screen back.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LaunchNoticeTap {
    pub id: i32,
    #[serde(default)]
    pub notification: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
}

/// The Kotlin command that writes down the screen a notice leads to, as it goes out.
/// Only Rust calls it: it is in neither `build.rs` nor the default permissions.
pub const REMEMBER_NOTICE_COMMAND: &str = "rememberNotice";

/// The Kotlin command Rust hands the channel through which Kotlin says a tap waits for
/// the page. Only Rust calls it: it is in neither `build.rs` nor the default permissions.
pub const WATCH_NOTICE_TAPS_COMMAND: &str = "watchNoticeTaps";

/// The notice's number and the screen it leads to, under the keys Kotlin reads.
#[derive(Debug, Serialize)]
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
struct RememberNoticeArgs<'a> {
    id: i32,
    path: Option<&'a str>,
}

/// `rememberNotice` resolves with no data (`invoke.resolve()` → `null`): read it as anything.
#[cfg(target_os = "android")]
type RememberNoticeAnswer = serde::de::IgnoredAny;

/// `watchNoticeTaps` resolves with no data (`invoke.resolve()` → `null`): read it as anything.
#[cfg(target_os = "android")]
type WatchNoticeTapsAnswer = serde::de::IgnoredAny;

/// The channel Kotlin calls back on when a tap waits for the page, under the key Kotlin reads.
#[cfg(target_os = "android")]
#[derive(Serialize)]
struct WatchNoticeTapsArgs {
    handler: tauri::ipc::Channel,
}

/// The wire shape of `takeNoticeTap`: `tap` absent or `null` when the app was not started by one.
#[derive(Debug, Default, Deserialize)]
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
struct NoticeTapAnswer {
    #[serde(default)]
    tap: Option<LaunchNoticeTap>,
}

#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "com.erplora.android";

/// The plugin's state: the handle of the Kotlin module on Android, nothing on desktop.
#[cfg(target_os = "android")]
pub struct ErploraAndroid<R: Runtime>(tauri::plugin::PluginHandle<R>);

#[cfg(not(target_os = "android"))]
pub struct ErploraAndroid<R: Runtime>(std::marker::PhantomData<fn() -> R>);

impl<R: Runtime> ErploraAndroid<R> {
    /// Sends the app to the background, as the system Back does on a root screen.
    pub fn leave_app(&self) -> Result<(), Error> {
        #[cfg(target_os = "android")]
        {
            return self
                .0
                .run_mobile_plugin::<LeaveAppAnswer>("leaveApp", Empty {})
                .map(|_| ())
                .map_err(|e| Error::PluginInvoke(e.to_string()));
        }
        #[cfg(not(target_os = "android"))]
        Ok(())
    }

    /// Opens this app's page in the device settings, the only place left to turn a
    /// permission on once Android stops showing its dialog. On desktop there is no such page.
    pub fn open_app_settings(&self) -> Result<(), Error> {
        #[cfg(target_os = "android")]
        {
            return self
                .0
                .run_mobile_plugin::<OpenAppSettingsAnswer>("openAppSettings", Empty {})
                .map(|_| ())
                .map_err(|e| Error::PluginInvoke(e.to_string()));
        }
        #[cfg(not(target_os = "android"))]
        Ok(())
    }

    /// Keeps the app running with the screen off (a foreground service and
    /// its ongoing notification) or lets Android reclaim it again. Every notice is born in the page,
    /// so this is what makes them arrive while nobody looks at the device. On desktop the app is not
    /// reclaimed.
    fn keep_listening(&self, args: KeepListeningArgs) -> Result<(), Error> {
        #[cfg(target_os = "android")]
        {
            return self
                .0
                .run_mobile_plugin::<KeepListeningAnswer>("keepListening", args)
                .map(|_| ())
                .map_err(|e| Error::PluginInvoke(e.to_string()));
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = args;
            Ok(())
        }
    }

    /// Whether the system voice (Android's `TextToSpeech`) can read the language.
    /// The first call makes the engine and waits for it to start. On desktop, never: Windows'
    /// WebView2 reads with the page's own `speechSynthesis`, which is where `false` sends the page.
    ///
    /// ⚠️ **Blocks** until the engine answers: call it off the async workers (`spawn_blocking`).
    fn tts_available(&self, args: TtsLanguageArgs) -> Result<bool, Error> {
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
    fn tts_speak(&self, args: TtsSpeakArgs) -> Result<(), Error> {
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
    fn tts_stop(&self) -> Result<(), Error> {
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

    /// The permissions granted right now. Always empty on desktop: there is nothing to grant.
    pub fn check_permissions(&self) -> Result<PermissionStatus, Error> {
        #[cfg(target_os = "android")]
        {
            return self
                .0
                .run_mobile_plugin("checkPermissions", Empty {})
                .map_err(|e| Error::PluginInvoke(e.to_string()));
        }
        #[cfg(not(target_os = "android"))]
        Ok(PermissionStatus::new())
    }

    /// Asks for what is missing, scoped to what the calling operation is about to use.
    /// `None` = no scope sent (an older web): the whole batch. Idempotent: if
    /// everything in scope is granted, no dialog.
    pub fn request_permissions(
        &self,
        permissions: Option<Vec<String>>,
    ) -> Result<PermissionStatus, Error> {
        #[cfg(target_os = "android")]
        {
            return self
                .0
                .run_mobile_plugin("requestPermissions", RequestPermissionsArgs { permissions })
                .map_err(|e| Error::PluginInvoke(e.to_string()));
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = permissions;
            Ok(PermissionStatus::new())
        }
    }

    /// The bonded devices that look like printers — Android's bonded list,
    /// filtered by `BluetoothSpp.looksLikePrinter`. On desktop, honestly empty: the platform is
    /// network-only by decision, and an error here would poison the network half of discovery.
    pub fn bluetooth_bonded_printers(&self) -> Result<Vec<BluetoothPrinter>, Error> {
        #[cfg(target_os = "android")]
        {
            return self
                .0
                .run_mobile_plugin::<BluetoothPrinterList>("bluetoothBondedPrinters", Empty {})
                .map(|list| list.printers)
                .map_err(|e| Error::PluginInvoke(e.to_string()));
        }
        #[cfg(not(target_os = "android"))]
        Ok(Vec::new())
    }

    /// Sends already-rendered ESC/POS bytes to the bonded printer at `mac` over RFCOMM.
    /// The rendering stays in Rust; Kotlin is transport only.
    ///
    /// ⚠️ **Blocks** — dispatched onto Android's main looper and waits for the socket work, so
    /// the calling command must be `#[tauri::command(async)]`, exactly like `save_download`.
    pub fn bluetooth_print(&self, mac: &str, payload: &[u8]) -> Result<(), Error> {
        #[cfg(target_os = "android")]
        {
            return self
                .0
                .run_mobile_plugin::<BluetoothPrintAnswer>(
                    "bluetoothPrint",
                    BluetoothPrintArgs::new(mac, payload),
                )
                .map(|_| ())
                .map_err(|e| Error::PluginInvoke(e.to_string()));
        }
        #[cfg(not(target_os = "android"))]
        {
            // An ERROR, never a silent no-op: a resolved print that put no paper out would tell
            // the till the ticket was printed. Unreachable through the shell, which routes
            // `bluetooth:` ids here only on Android — said out loud anyway.
            let _ = (mac, payload);
            Err(Error::PluginInvoke(
                "bluetooth printing is Android-only".into(),
            ))
        }
    }

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

    /// The tap on a notice that started the app, handed over once.
    ///
    /// The notification plugin reports that tap while it is still loading, before the page has a
    /// listener, and the event is lost; Kotlin keeps it instead. Off Android there is no such launch.
    pub fn take_notice_tap(&self) -> Result<Option<LaunchNoticeTap>, Error> {
        #[cfg(target_os = "android")]
        {
            return self
                .0
                .run_mobile_plugin::<NoticeTapAnswer>("takeNoticeTap", Empty {})
                .map(|answer| answer.tap)
                .map_err(|e| Error::PluginInvoke(e.to_string()));
        }
        #[cfg(not(target_os = "android"))]
        Ok(None)
    }

    /// Writes down the screen notice `id` leads to, as it goes out. The notification
    /// plugin brings a tap back with the notice's number only (it never stores the notice on the
    /// tap's intent), so this is what lets the tap open its screen, also when it starts the app.
    /// Off Android a tap carries its screen itself: there is nothing to write down.
    pub fn remember_notice(&self, id: i32, path: Option<&str>) -> Result<(), Error> {
        #[cfg(target_os = "android")]
        {
            return self
                .0
                .run_mobile_plugin::<RememberNoticeAnswer>(REMEMBER_NOTICE_COMMAND, RememberNoticeArgs { id, path })
                .map(|_| ())
                .map_err(|e| Error::PluginInvoke(e.to_string()));
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = (id, path);
            Ok(())
        }
    }

    /// `on_tap` runs every time a tap on a notice waits for a page that already
    /// listens: the notification plugin's own report of it says nothing the page can open. The
    /// shell tells the page, which claims the tap with `take_notice_tap`. Off Android there is no
    /// such tap to watch.
    pub fn watch_notice_taps<F: Fn() + Send + Sync + 'static>(&self, on_tap: F) -> Result<(), Error> {
        #[cfg(target_os = "android")]
        {
            use tauri::ipc::Channel;
            return self
                .0
                .run_mobile_plugin::<WatchNoticeTapsAnswer>(
                    WATCH_NOTICE_TAPS_COMMAND,
                    WatchNoticeTapsArgs {
                        handler: Channel::new(move |_| {
                            on_tap();
                            Ok(())
                        }),
                    },
                )
                .map(|_| ())
                .map_err(|e| Error::PluginInvoke(e.to_string()));
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = on_tap;
            Ok(())
        }
    }

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

    /// Asks the person «Open <hub> on this device?» in an `AlertDialog` before a link links another
    /// hub; `true` only for «Open». Both languages travel and the device's is shown.
    ///
    /// ⚠️ **Blocks** until the person answers: call it from a thread of its own.
    pub fn ask_to_open_hub(&self, en: &Wording, es: &Wording) -> Result<bool, Error> {
        #[cfg(target_os = "android")]
        {
            return self
                .0
                .run_mobile_plugin::<AskToOpenHubAnswer>(
                    ASK_TO_OPEN_HUB_COMMAND,
                    AskToOpenHubArgs { en, es },
                )
                .map(|answer| answer.open)
                .map_err(|e| Error::PluginInvoke(e.to_string()));
        }
        #[cfg(not(target_os = "android"))]
        {
            // Unreachable by construction: the desktop asks with its own dialog. A no all the same.
            let _ = (en, es);
            Err(Error::PluginInvoke("ask_to_open_hub is Android only".into()))
        }
    }
}

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

/// The page asks the app to keep listening for notices with the screen off, or to stop.
#[tauri::command]
async fn keep_listening<R: Runtime>(
    app: tauri::AppHandle<R>,
    on: bool,
    title: Option<String>,
    body: Option<String>,
    channel: Option<String>,
) -> Result<(), Error> {
    app.erplora_android().keep_listening(KeepListeningArgs { on, title, body, channel })
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
        keep_listening,
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
mod tests;
