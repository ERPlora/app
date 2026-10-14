//! Permisos de RUNTIME de Android para el shell de ERPlora.
//!
//! Declarar un permiso en el manifest no basta, y los dos que necesita el TPV fallan **en
//! silencio**:
//!
//! - sin `ACCESS_LOCAL_NETWORK` (API 37+) el barrido del puerto 9100 son 254 **timeouts**, así que
//!   el descubrimiento devuelve `[]` y parece que el local no tiene impresoras;
//! - sin `POST_NOTIFICATIONS` (API 33+) la notificación no aparece, y la comanda entra en cocina
//!   sin que nadie se entere.
//!
//! Verificado en el emulador API 37 antes de existir este plugin: `erplora_discover_printers`
//! devolvía una lista vacía sin la menor queja hasta conceder el permiso a mano con `adb`.
//!
//! En escritorio no hay nada que pedir: los comandos existen igual y responden «concedido», para
//! que la PWA pueda llamarlos sin ramificar por plataforma.

use serde::{Deserialize, Serialize};
use tauri::{
    ipc::Invoke,
    plugin::{Builder, TauriPlugin},
    Manager, Runtime,
};

/// Estado de un permiso tal y como lo ve la PWA: `{ "android.permission.X": true }`.
pub type PermissionStatus = std::collections::HashMap<String, bool>;

/// Local network access (API 37+). Without it the printer sweep is 254 silent timeouts.
///
/// Mirror of `PermissionPolicy.ACCESS_LOCAL_NETWORK` on the Kotlin side. The key of the map above
/// **is** the permission string, so Rust needs the same literal to read the answer — and a test
/// below checks the two never drift apart.
pub const ACCESS_LOCAL_NETWORK: &str = "android.permission.ACCESS_LOCAL_NETWORK";

/// System notifications (API 33+). Mirror of `PermissionPolicy.POST_NOTIFICATIONS`.
pub const POST_NOTIFICATIONS: &str = "android.permission.POST_NOTIFICATIONS";

/// Talking to bonded Bluetooth devices (API 31+): the SPP transport of ADR-0204 (hub#388).
/// Mirror of `PermissionPolicy.BLUETOOTH_CONNECT`. Denied, the bonded list comes back empty and
/// the RFCOMM connect throws — the same silent-failure family as the two above.
pub const BLUETOOTH_CONNECT: &str = "android.permission.BLUETOOTH_CONNECT";

/// The code a bluetooth operation is refused under when `BLUETOOTH_CONNECT` is denied (ADR-0204).
/// Mirror of `ErploraAndroidPlugin.BLUETOOTH_PERMISSION_DENIED` — same trip as
/// [`DOWNLOADS_UNREACHABLE`]: Kotlin rejects with a code, Tauri renders `[code] - message`, and
/// the shell recognises it out of the text.
pub const BLUETOOTH_PERMISSION_DENIED: &str = "bluetooth_permission_denied";

/// This device has no NFC reader at all (hub#988). Mirror of `NfcBadge.NFC_UNAVAILABLE`, and the
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

/// The code an Android with no public Downloads collection is refused under (hub#499).
///
/// Mirror of `DownloadPublisher.DOWNLOADS_UNREACHABLE`, and the same word `apps/tauri` and
/// `save-download.ts` already read: it is the ONE refusal the user can act on, so it must survive
/// the trip from Kotlin to the page intact. A test below checks the two sides never drift apart.
pub const DOWNLOADS_UNREACHABLE: &str = "downloads_unreachable";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    PluginInvoke(String),
    /// This Android is older than the public Downloads collection (API 29), so there is nowhere to
    /// put a file that the user could then open. The shell turns this into its own sentence.
    #[error("downloads_unreachable")]
    DownloadsUnreachable,
    /// No NFC reader on this device — a desktop, or a tablet sold without one (hub#988).
    #[error("nfc_unavailable")]
    NfcUnavailable,
    /// There is a reader and it is switched off. The only one of the three the user can fix.
    #[error("nfc_disabled")]
    NfcDisabled,
    /// The card randomises its id on every tap, so it can never be matched again after enrolment.
    #[error("nfc_random_uid")]
    NfcRandomUid,
}

impl Serialize for Error {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Empty {}

/// The scope of a `request_permissions` call (hub#758): the permissions the OPERATION that asks
/// is about to use, or `None` when the caller sent none (a web older than the scope), which keeps
/// the old ask-for-everything behavior so the two separately-shipped halves cannot break each
/// other.
#[derive(Debug, Serialize)]
struct RequestPermissionsArgs {
    permissions: Option<Vec<String>>,
}

/// What the shell asks the native side to publish, and where it staged the bytes (hub#499).
///
/// A **path** and not the bytes: the shell has already written the file into its own cache, so an
/// export crosses the JNI boundary once instead of being copied onto a tablet's heap a second time.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublishArgs {
    source_path: String,
    name: String,
}

/// A bluetooth print job on its way to Kotlin (ADR-0204): the MAC of a bonded printer plus the
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
/// its shape — reading it as [`Empty`] reported a ticket already on paper as `failed` (hub#2024,
/// the same trap as `printHtml` in hub#2008).
type BluetoothPrintAnswer = serde::de::IgnoredAny;

/// A bonded printer as Kotlin announces it: `id` is `bluetooth:{MAC}` (the printer_id contract of
/// ADR-0204), `mac` the raw identity the registry keys on.
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

/// The command that opens Android's print screen with an A4 document (hub#2008). Mirror of
/// `ErploraAndroidPlugin.printHtml`; a test below checks the two never drift apart.
pub const PRINT_HTML_COMMAND: &str = "printHtml";

/// An A4 document on its way to Kotlin (hub#2008): its html, already checked by the shell. A
/// string and not a path: an invoice is tens of KB, and the shell caps it well below what the
/// bridge carries.
#[derive(Debug, Serialize)]
struct PrintHtmlArgs {
    html: String,
}

/// What `printHtml` answers with. Kotlin's `invoke.resolve()` with no data reaches Rust as JSON
/// `null`, so the answer is read and ignored whatever its shape — reading it as [`Empty`] turned a
/// print screen that DID open into a failure (seen on the emulator, hub#2008).
type PrintHtmlAnswer = serde::de::IgnoredAny;

/// The question the app asks before a link links another hub (hub#2644), answered by an
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

/// hub#2307 — what the page asks of the listening service: `on`, and the words of the ongoing
/// notification in the app's language (ADR-0055: the catalogue lives with the page). Off, no words.
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

/// `keepListening` resolves with no data (`invoke.resolve()` → `null`): read it as anything (hub#2024).
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
type KeepListeningAnswer = serde::de::IgnoredAny;

/// `leaveApp` resolves with no data (`invoke.resolve()` → `null`): read it as anything (hub#2024).
type LeaveAppAnswer = serde::de::IgnoredAny;

/// `openAppSettings` resolves with no data (`invoke.resolve()` → `null`): read it as anything
/// (hub#2024).
#[cfg(target_os = "android")]
type OpenAppSettingsAnswer = serde::de::IgnoredAny;

/// How long reader mode may stay open on one call (hub#988). Kotlin clamps it: an argument nobody
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
/// be told to switch NFC on in its settings and sent looking for a toggle that is not there
/// (the hub#338 lesson, again).
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

/// The tap on a notice that STARTED the app (hub#2360): the notice's id and the JSON the
/// notification plugin stored it as, whose `extra.path` is the screen it leads to.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LaunchNoticeTap {
    pub id: i32,
    #[serde(default)]
    pub notification: Option<String>,
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

/// Estado del plugin: en Android guarda el handle del módulo Kotlin; en escritorio, nada.
#[cfg(target_os = "android")]
pub struct ErploraAndroid<R: Runtime>(tauri::plugin::PluginHandle<R>);

#[cfg(not(target_os = "android"))]
pub struct ErploraAndroid<R: Runtime>(std::marker::PhantomData<fn() -> R>);

impl<R: Runtime> ErploraAndroid<R> {
    /// hub#1906 — sends the app to the background, as the system Back does on a root screen.
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

    /// hub#1886 — opens this app's page in the device settings, the only place left to turn a
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

    /// hub#2307 — keeps the app running with the screen off (a foreground service and its ongoing
    /// notification) or lets Android reclaim it again. Every notice is born in the page, so this is
    /// what makes them arrive while nobody looks at the device. On desktop the app is not reclaimed.
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

    /// Permisos concedidos ahora mismo. En escritorio, siempre vacío: no hay nada que conceder.
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

    /// Asks for what is missing, scoped to what the calling operation is about to use (hub#758).
    /// `None` = no scope sent (an older web): the whole batch, as before. Idempotent: if
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

    /// The bonded devices that look like printers (ADR-0204, hub#388) — Android's bonded list,
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

    /// Sends already-rendered ESC/POS bytes to the bonded printer at `mac` over RFCOMM
    /// (ADR-0204). The rendering stays in Rust; Kotlin is transport only.
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
            // An ERROR, never a silent no-op: a resolved print that put no paper out is the
            // hub#475 failure all over again. Unreachable through the shell, which routes
            // `bluetooth:` ids here only on Android — said out loud anyway.
            let _ = (mac, payload);
            Err(Error::PluginInvoke(
                "bluetooth printing is Android-only (ADR-0204)".into(),
            ))
        }
    }

    /// Opens NFC reader mode for at most `timeout_ms` and answers with the badge of the card that
    /// was tapped (hub#988) — `None` when the window closed with nothing on the reader.
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

    /// hub#2360 — the tap on a notice that started the app, handed over once.
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

    /// Publishes the file at `source_path` into Android's **public** Downloads collection under
    /// `name`, and answers with the place to show the user (hub#499).
    ///
    /// This is how a business gets its own data off the tablet it keeps it on. Everywhere else the
    /// shell just writes to the Downloads folder; on Android there is no such folder to write to —
    /// what `download_dir()` resolves is app-scoped storage that Android 11 closed to every file
    /// manager, so the file would exist and be unreachable (hub#480, ADR-0259). `MediaStore` is the
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
    /// «Save as PDF» (hub#2008). Resolves once the screen has been asked for; what the user does
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
    /// hub (hub#2644); `true` only for «Open». Both languages travel and the device's is shown.
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

/// hub#1906 — leaves the app the way the system Back does on a root screen (the task goes to
/// the background; nothing is killed). The shell calls it when it holds the Back button and there
/// is nothing left to close nor to go back to. On desktop there is no such button: a no-op.
#[tauri::command]
async fn leave_app<R: Runtime>(app: tauri::AppHandle<R>) -> Result<(), Error> {
    app.erplora_android().leave_app()
}

/// hub#1886 — takes the owner to this app's page in the device settings. The shell only offers it
/// on a device that really has a permission refused, so on desktop it is never called: a no-op.
#[tauri::command]
async fn open_app_settings<R: Runtime>(app: tauri::AppHandle<R>) -> Result<(), Error> {
    app.erplora_android().open_app_settings()
}

/// hub#2307 — the page asks the app to keep listening for notices with the screen off, or to stop.
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

/// The plugin, with `gate` in front of every one of its commands (hub#2642). The capability grants
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
        keep_listening
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
mod tests {
    use super::*;

    /// hub#1906 — once the shell takes the Android Back button (Tauri's `onBackButtonPress`),
    /// Tauri no longer leaves the app on its own when the WebView has no history left: the shell
    /// has to ask for it, and Tauri's own `exit` has no ACL permission to grant. `leave_app` is that
    /// door. Four places have to agree or the press dies in silence: the build (which generates the
    /// permission), the invoke handler, the default permission set the hub capability grants, and
    /// the Kotlin command that actually leaves.
    const PLUGIN_KT: &str = include_str!("../android/src/main/java/com/erplora/android/ErploraAndroidPlugin.kt");
    const BUILD_RS: &str = include_str!("../build.rs");
    const LIB_RS: &str = include_str!("lib.rs");
    const DEFAULT_PERMISSIONS: &str = include_str!("../permissions/default.toml");

    #[test]
    fn leave_app_is_declared_wired_granted_and_implemented_hub1906() {
        assert!(BUILD_RS.contains("\"leave_app\""), "build.rs does not declare leave_app: no permission is generated");
        // Only the code above the tests: this very assertion spells the handler too.
        let production = LIB_RS.split("#[cfg(test)]").next().unwrap_or_default();
        assert!(
            production
                .split("generate_handler![")
                .nth(1)
                .and_then(|rest| rest.split(']').next())
                .is_some_and(|handler| handler.split(',').any(|c| c.trim() == "leave_app")),
            "leave_app is not wired to the invoke handler"
        );
        assert!(
            DEFAULT_PERMISSIONS.contains("\"allow-leave-app\""),
            "erplora-android:default does not grant allow-leave-app: the hub's capability would refuse it"
        );
        let kotlin = PLUGIN_KT.split("fun leaveApp(invoke: Invoke)").nth(1).expect("no Kotlin leaveApp command");
        let before = PLUGIN_KT.split("fun leaveApp(invoke: Invoke)").next().unwrap_or_default();
        assert!(before.trim_end().ends_with("@Command"), "Kotlin leaveApp is not a @Command");
        let body = kotlin.split("\n    }").next().unwrap_or_default();
        assert!(body.contains("moveTaskToBack(true)"), "leaveApp does not leave the way the system Back does");
        assert!(body.contains("invoke.resolve()"), "leaveApp never answers: the web would wait forever");
    }

    /// hub#1886 — once Android stops showing a permission dialog, the only way back is the app's
    /// page in the device settings, and the shell offers a button that takes the owner there.
    /// Four places have to agree or the tap dies in silence on a real device only: the build
    /// (which generates the permission), the invoke handler, the default permission set the hub
    /// capability grants, and the Kotlin command that opens the page.
    const PLUGIN_KT_SOURCE: &str =
        include_str!("../android/src/main/java/com/erplora/android/ErploraAndroidPlugin.kt");
    const BUILD_RS_SOURCE: &str = include_str!("../build.rs");
    const LIB_RS_SOURCE: &str = include_str!("lib.rs");
    const DEFAULT_PERMISSIONS_TOML: &str = include_str!("../permissions/default.toml");

    #[test]
    fn open_app_settings_is_declared_wired_granted_and_implemented_hub1886() {
        assert!(
            BUILD_RS_SOURCE.contains("\"open_app_settings\""),
            "build.rs does not declare open_app_settings: no permission is generated"
        );
        // Only the code above the tests: this very assertion spells the command too.
        let production = LIB_RS_SOURCE.split("#[cfg(test)]").next().unwrap_or_default();
        let handler = production
            .split("generate_handler![")
            .nth(1)
            .and_then(|rest| rest.split(']').next())
            .expect("no invoke handler");
        assert!(
            handler.split(',').any(|c| c.trim() == "open_app_settings"),
            "open_app_settings is not wired to the invoke handler"
        );
        assert!(
            DEFAULT_PERMISSIONS_TOML.contains("\"allow-open-app-settings\""),
            "erplora-android:default does not grant allow-open-app-settings: the hub's capability \
             would refuse it"
        );
        let signature = "fun openAppSettings(invoke: Invoke)";
        let before = PLUGIN_KT_SOURCE.split(signature).next().unwrap_or_default();
        assert!(before.trim_end().ends_with("@Command"), "Kotlin openAppSettings is not a @Command");
        let body = PLUGIN_KT_SOURCE
            .split(signature)
            .nth(1)
            .and_then(|rest| rest.split("\n    }").next())
            .expect("no Kotlin openAppSettings command");
        assert!(
            body.contains("Settings.ACTION_APPLICATION_DETAILS_SETTINGS"),
            "openAppSettings does not open the app's own page in the device settings"
        );
        assert!(
            body.contains("Uri.fromParts(\"package\", activity.packageName, null)"),
            "openAppSettings does not point the settings at THIS app"
        );
        assert!(body.contains("invoke.resolve()"), "openAppSettings never answers: the web would wait forever");
        assert!(
            body.contains("invoke.reject("),
            "openAppSettings swallows a device with no settings page: the web must hear it to fall back"
        );
    }

    // ── hub#2307: listening for notices with the screen off ──────────────────────────────────────
    //
    // The notices come out of the page (the bell's poll, the event socket), so they only arrive while
    // Android keeps the app running. A foreground service is what keeps it: six places have to agree
    // on it and none of them is seen by the compiler — the build (the permission), the handler, the
    // default set the hub's capability grants, the Kotlin command, the service class and the manifest
    // that declares it with the type and permissions Android 14 demands. Any one missing and the app
    // behaves exactly as before, with every other test green.

    const NOTICE_LISTENING_KT: &str =
        include_str!("../android/src/main/java/com/erplora/android/NoticeListening.kt");
    const PLUGIN_MANIFEST: &str = include_str!("../android/src/main/AndroidManifest.xml");

    #[test]
    fn keep_listening_is_declared_wired_granted_and_implemented_hub2307() {
        assert!(BUILD_RS.contains("\"keep_listening\""), "build.rs does not declare keep_listening");
        let production = LIB_RS.split("#[cfg(test)]").next().unwrap_or_default();
        let handler = production
            .split("generate_handler![")
            .nth(1)
            .and_then(|rest| rest.split(']').next())
            .expect("no invoke handler");
        assert!(
            handler.split(',').any(|c| c.trim() == "keep_listening"),
            "keep_listening is not wired to the invoke handler"
        );
        assert!(
            production.contains("run_mobile_plugin::<KeepListeningAnswer>(\"keepListening\", args)"),
            "Rust does not reach Kotlin under the name Kotlin answers to"
        );
        assert!(
            DEFAULT_PERMISSIONS.contains("\"allow-keep-listening\""),
            "erplora-android:default does not grant allow-keep-listening: the hub's capability would refuse it"
        );
        let signature = "fun keepListening(invoke: Invoke)";
        let before = PLUGIN_KT.split(signature).next().unwrap_or_default();
        assert!(before.trim_end().ends_with("@Command"), "Kotlin keepListening is not a @Command");
        let body = PLUGIN_KT
            .split(signature)
            .nth(1)
            .and_then(|rest| rest.split("\n    }").next())
            .expect("no Kotlin keepListening command");
        assert!(body.contains("NoticeListeningService.start("), "keepListening never starts the service");
        assert!(body.contains("NoticeListeningService.stop("), "keepListening never stops the service");
        assert!(body.contains("invoke.resolve()"), "keepListening never answers: the web would wait forever");
        assert!(body.contains("invoke.reject("), "keepListening swallows a refusal the page must hear");
    }

    #[test]
    fn the_listening_request_crosses_to_kotlin_under_the_keys_kotlin_reads() {
        let on = serde_json::to_value(KeepListeningArgs {
            on: true,
            title: Some("t".into()),
            body: Some("b".into()),
            channel: Some("c".into()),
        })
        .expect("serializes");
        assert_eq!(on, serde_json::json!({ "on": true, "title": "t", "body": "b", "channel": "c" }));
        let off = serde_json::to_value(KeepListeningArgs { on: false, title: None, body: None, channel: None })
            .expect("serializes");
        assert_eq!(off, serde_json::json!({ "on": false }));
        for key in ["\"on\"", "\"title\"", "\"body\"", "\"channel\""] {
            assert!(PLUGIN_KT.contains(key), "Kotlin does not read {key} from the request");
        }
    }

    #[test]
    fn the_app_is_not_kept_listening_once_its_page_is_gone_hub2307() {
        // The service keeps the PROCESS; the page is what listens. With the activity destroyed or the
        // app swiped away, a notification saying «listening» would be a promise nothing keeps.
        let on_destroy = PLUGIN_KT
            .split("override fun onDestroy(activity: AppCompatActivity)")
            .nth(1)
            .and_then(|rest| rest.split("\n    }").next())
            .expect("the plugin does not stop anything when the activity is destroyed");
        assert!(on_destroy.contains("NoticeListeningService.stop("), "onDestroy leaves the service running");
        let task_removed = NOTICE_LISTENING_KT
            .split("override fun onTaskRemoved(")
            .nth(1)
            .and_then(|rest| rest.split("\n    }").next())
            .expect("the service does not react to the app being swiped away");
        assert!(task_removed.contains("stopSelf()"), "swiping the app away leaves the service running");
        assert!(
            NOTICE_LISTENING_KT.contains("return NoticeListening.START_MODE"),
            "onStartCommand does not answer with the pinned start mode"
        );
    }

    #[test]
    fn the_page_is_kept_running_exactly_while_the_app_listens_hub2307() {
        // The service keeps the process, `PageKeeper` keeps the page: without it Chromium freezes
        // the hidden page at 60 s and no notice is born, with the service and its notification in
        // place (measured on the device). What it does has its own test in Kotlin
        // (`PageKeeperTest`); what only lives in the plugin is WHEN it is told — installed with the
        // WebView, on once the service started, off with every way the listening ends.
        let kotlin_fn = |signature: &str| {
            PLUGIN_KT
                .split(signature)
                .nth(1)
                .and_then(|rest| rest.split("\n    }").next())
                .unwrap_or_else(|| panic!("the plugin has no `{signature}`"))
        };
        let load = kotlin_fn("override fun load(webView: WebView)");
        assert!(load.contains("PageKeeper(webView)"), "no keeper is built for the WebView");
        assert!(load.contains("keeper.install(root, owner)"), "the keeper never hears the app leave the screen");
        assert!(load.contains("pageKeeper = keeper"), "the commands cannot reach the keeper");

        let command = kotlin_fn("fun keepListening(invoke: Invoke)");
        let (off, on) = command
            .split_once("NoticeListening.textsOf(")
            .expect("keepListening does not read the words of the notification");
        assert!(off.contains("pageKeeper?.setListening(false)"), "told to stop, the page is still kept running");
        let started = on
            .split_once("NoticeListeningService.start(")
            .map(|(_, after)| after)
            .and_then(|after| after.split("catch").next())
            .expect("keepListening never starts the service");
        assert!(
            started.contains("pageKeeper?.setListening(true)"),
            "the service starts and the page still freezes: the keeper is never told to listen"
        );
        assert!(
            !on.split("NoticeListeningService.start(").next().unwrap_or_default().contains("setListening(true)"),
            "the page is kept running before the service is known to have started"
        );

        let on_destroy = kotlin_fn("override fun onDestroy(activity: AppCompatActivity)");
        assert!(
            on_destroy.contains("pageKeeper?.setListening(false)"),
            "the activity is gone and its page is still kept running"
        );
    }

    #[test]
    fn the_listening_service_is_declared_where_no_generator_can_drop_it_hub2307() {
        let manifest = without_comments(PLUGIN_MANIFEST);
        let declared = declared_permissions(PLUGIN_MANIFEST);
        for permission in [
            "android.permission.FOREGROUND_SERVICE",
            "android.permission.FOREGROUND_SERVICE_SPECIAL_USE",
        ] {
            assert!(declared.iter().any(|p| p == permission), "{permission} is not declared: startForeground throws");
        }
        let service = manifest
            .split("<service")
            .nth(1)
            .and_then(|rest| rest.split("</service>").next())
            .expect("no <service> in the plugin manifest");
        assert!(
            service.contains("android:name=\"com.erplora.android.NoticeListeningService\""),
            "the declared service is not NoticeListeningService"
        );
        assert!(
            service.contains("android:foregroundServiceType=\"specialUse\""),
            "the service does not declare the type Android 14 demands"
        );
        assert!(service.contains("android:exported=\"false\""), "another app could start or stop the service");
        assert!(
            service.contains("android.app.PROPERTY_SPECIAL_USE_FGS_SUBTYPE"),
            "a specialUse service without its subtype is refused by Google Play's review"
        );
    }

    #[test]
    fn el_estado_serializa_como_un_mapa_permiso_a_booleano() {
        // Es el contrato que consume la PWA: `{ "android.permission.X": true }`.
        let mut estado = PermissionStatus::new();
        estado.insert("android.permission.POST_NOTIFICATIONS".into(), true);
        let json = serde_json::to_string(&estado).unwrap();
        assert!(json.contains("\"android.permission.POST_NOTIFICATIONS\":true"));
    }

    /// The Kotlin policy is the only place that can actually ASK for these permissions, and the
    /// map it returns is keyed by the raw string. If the two sides ever spelled one differently,
    /// Rust would read `None` for a permission Android had denied and the till would go back to
    /// reporting zero printers with a straight face (hub#338).
    const PERMISSION_POLICY_KT: &str =
        include_str!("../android/src/main/java/com/erplora/android/PermissionPolicy.kt");

    #[test]
    fn rust_and_kotlin_spell_the_permissions_the_same_way() {
        for permission in [ACCESS_LOCAL_NETWORK, POST_NOTIFICATIONS, BLUETOOTH_CONNECT] {
            assert!(
                PERMISSION_POLICY_KT.contains(permission),
                "{permission} is not in PermissionPolicy.kt — the status map would never mention it"
            );
        }
    }

    /// Every `android.permission.*` the Kotlin policy names, read out of its source.
    ///
    /// Read rather than listed so the guards below cannot drift from the one place that decides
    /// what the shell ASKS for: a permission added there and forgotten everywhere else is the bug
    /// this file is about.
    fn permissions_the_plugin_can_ask_for() -> Vec<&'static str> {
        PERMISSION_POLICY_KT
            .match_indices("\"android.permission.")
            .filter_map(|(at, _)| PERMISSION_POLICY_KT[at + 1..].split('"').next())
            .collect()
    }

    /// Everything outside XML comments. Commenting a tag out is the easiest way to "temporarily"
    /// drop a permission, and the edit nobody remembers to undo.
    fn without_comments(xml: &str) -> String {
        let mut out = String::new();
        let mut rest = xml;
        while let Some(start) = rest.find("<!--") {
            out.push_str(&rest[..start]);
            let Some(end) = rest[start..].find("-->") else { return out };
            rest = &rest[start + end + "-->".len()..];
        }
        out.push_str(rest);
        out
    }

    /// Permissions actually DECLARED by a manifest, read out of its live `<uses-permission>` tags.
    ///
    /// Not a text search: manifests here carry long comments naming the permissions and explaining
    /// why they are there, so `contains()` would keep passing on the prose after the tag itself was
    /// deleted — a guard that goes green on the wreckage is worse than no guard.
    fn declared_permissions(manifest: &str) -> Vec<String> {
        without_comments(manifest)
            .split("<uses-permission")
            .skip(1)
            .filter_map(|tag| {
                tag.split("android:name=\"")
                    .nth(1)
                    .and_then(|value| value.split('"').next())
                    .map(str::to_owned)
            })
            .collect()
    }

    #[test]
    fn a_commented_out_permission_does_not_count_as_declared() {
        // How a permission usually disappears: someone comments it out "for a minute". The prose
        // around it still names it, so a text search would call it declared and let the guard
        // below wave the regression through.
        let manifest = r#"<manifest>
            <uses-permission android:name="android.permission.INTERNET" />
            <!-- <uses-permission android:name="android.permission.ACCESS_LOCAL_NETWORK" /> -->
        </manifest>"#;

        assert_eq!(declared_permissions(manifest), ["android.permission.INTERNET"]);
    }

    /// Asking for a permission the merged manifest never declared is not a dialog the user can
    /// say yes to: Android answers DENIED at once, forever, and shows nothing. The till would
    /// then tell the user to grant local network access in the system settings (hub#338) and send
    /// them looking for a toggle that does not exist.
    ///
    /// The plugin declares them in its OWN Android library manifest, which the manifest merger
    /// folds into the app at build time. That is what makes the declaration survive
    /// `cargo tauri android init` — the generator rewrites `apps/tauri/src-tauri/gen/android`, it
    /// does not reach into `crates/` (hub#337).
    #[test]
    fn the_plugin_declares_every_permission_it_can_ask_for() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/android/src/main/AndroidManifest.xml");
        let manifest = std::fs::read_to_string(path).unwrap_or_else(|e| {
            panic!(
                "{path}: {e}\n\
                 The plugin ships no Android manifest, so these permissions exist only in the \
                 GENERATED gen/android project. Lose that file and `cargo tauri android init` \
                 writes Tauri's template back — INTERNET and nothing else — and the till stops \
                 finding printers with a perfectly green build."
            )
        });

        let declared = declared_permissions(&manifest);
        let asked_for = permissions_the_plugin_can_ask_for();
        assert!(!asked_for.is_empty(), "PermissionPolicy.kt names no permission at all");
        for permission in asked_for {
            assert!(
                declared.iter().any(|d| d == permission),
                "PermissionPolicy asks for {permission}, but no manifest of this plugin declares \
                 it: Android would answer that request DENIED without ever showing a dialog.\n\
                 Declared right now: {declared:?}"
            );
        }
    }

    /// The scope of a permission request has to reach Kotlin under the key Kotlin reads
    /// (hub#758). If either side renamed it, the argument would silently vanish and the plugin
    /// would fall back to requesting its whole batch — the exact out-of-context notifications
    /// dialog this contract exists to end, with no error anywhere.
    #[test]
    fn the_permission_scope_travels_under_the_key_kotlin_reads() {
        let args = RequestPermissionsArgs {
            permissions: Some(vec![ACCESS_LOCAL_NETWORK.to_string()]),
        };
        let json = serde_json::to_value(&args).expect("serializable");
        assert_eq!(json["permissions"][0], ACCESS_LOCAL_NETWORK);

        const PLUGIN_KT: &str =
            include_str!("../android/src/main/java/com/erplora/android/ErploraAndroidPlugin.kt");
        assert!(
            PLUGIN_KT.contains("optJSONArray(\"permissions\")"),
            "ErploraAndroidPlugin.kt no longer reads the \"permissions\" scope: every request \
             would go back to asking for the whole batch (hub#758)"
        );
    }

    /// An absent scope must serialize as an ABSENT key, not as `null`: Kotlin's `optJSONArray`
    /// treats both as "no scope", but the wire contract is "an old web sends nothing", and
    /// pinning it keeps the fallback path honest.
    #[test]
    fn no_scope_still_serializes_and_means_the_whole_batch() {
        let args = RequestPermissionsArgs { permissions: None };
        let json = serde_json::to_value(&args).expect("serializable");
        assert!(json["permissions"].is_null());
    }

    // ── Bluetooth Classic SPP (ADR-0204, hub#388) ────────────────────────────────────────────
    //
    // The transport lives in Kotlin (`BluetoothSpp.kt`); what Rust owns is the CONTRACT across
    // the JNI boundary — command names, argument keys, the shape of the answer, and the refusal
    // code. Every one of those is a bare literal on both sides, so each gets a drift guard: a
    // typo in either one is invisible until a real till stops finding its bluetooth printer.

    const ERPLORA_ANDROID_PLUGIN_KT: &str =
        include_str!("../android/src/main/java/com/erplora/android/ErploraAndroidPlugin.kt");

    #[test]
    fn the_print_payload_crosses_to_kotlin_under_the_keys_kotlin_reads() {
        // ESC/POS init (`ESC @`) — two bytes, so the base64 is checkable by eye.
        let args = BluetoothPrintArgs::new("AA:BB:CC:DD:EE:FF", &[0x1b, 0x40]);
        let json = serde_json::to_value(&args).expect("serializable");
        assert_eq!(json["mac"], "AA:BB:CC:DD:EE:FF");
        assert_eq!(json["payloadBase64"], "G0A=");

        for key in ["getString(\"mac\"", "getString(\"payloadBase64\""] {
            assert!(
                ERPLORA_ANDROID_PLUGIN_KT.contains(key),
                "ErploraAndroidPlugin.kt no longer reads {key}) — the print job would cross the \
                 boundary and vanish"
            );
        }
    }

    #[test]
    fn a_bonded_printer_list_deserializes_from_what_kotlin_sends() {
        let list: BluetoothPrinterList = serde_json::from_value(serde_json::json!({
            "printers": [
                { "id": "bluetooth:AA:BB:CC:DD:EE:FF", "name": "Kitchen", "mac": "AA:BB:CC:DD:EE:FF" }
            ]
        }))
        .expect("the wire shape Kotlin builds");
        assert_eq!(list.printers.len(), 1);
        assert_eq!(list.printers[0].id, "bluetooth:AA:BB:CC:DD:EE:FF");
        assert_eq!(list.printers[0].mac, "AA:BB:CC:DD:EE:FF");
    }

    #[test]
    fn rust_and_kotlin_spell_the_bluetooth_refusal_and_commands_the_same_way() {
        for literal in [BLUETOOTH_PERMISSION_DENIED, "bluetoothBondedPrinters", "bluetoothPrint"] {
            assert!(
                ERPLORA_ANDROID_PLUGIN_KT.contains(literal),
                "{literal} is not in ErploraAndroidPlugin.kt — the mobile call would answer \
                 `command not found` (or the refusal would lose its name) on a real device only"
            );
        }
    }

    #[test]
    fn desktop_stays_network_only_by_construction() {
        // ADR-0204: only Android produces or accepts `bluetooth:{mac}`. On desktop the bonded
        // list is honestly empty and a bluetooth print is an ERROR, never a silent no-op — a
        // resolved print that put no paper out is the hub#475 failure all over again.
        let desktop = ErploraAndroid::<tauri::Wry>(std::marker::PhantomData);
        assert_eq!(
            desktop
                .bluetooth_bonded_printers()
                .expect("an empty list, not an error")
                .len(),
            0
        );
        assert!(desktop.bluetooth_print("AA:BB:CC:DD:EE:FF", &[0x1b]).is_err());
    }

    // ── Reading a badge off the device's own NFC reader (hub#988) ────────────────────────────
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
        // settings screen for a tablet that has no reader to enable (the hub#338 lesson).
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

    #[test]
    fn el_error_llega_al_frontend_como_texto_plano() {
        // Mismo patrón que el resto del shell: la promesa se rechaza con un mensaje legible.
        let e = Error::PluginInvoke("sin actividad".into());
        assert_eq!(serde_json::to_string(&e).unwrap(), "\"sin actividad\"");
    }

    // ── Publishing a file into the public Downloads collection (hub#499) ─────────────────────────

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
        // `apps/tauri` maps this variant onto `ShellError::DownloadsUnreachable`, which serializes
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

    /// Kotlin source with its comments taken out.
    ///
    /// The guard below went red on its own prose the first time it ran: `DownloadPublisher` EXPLAINS
    /// why it does not use `WRITE_EXTERNAL_STORAGE`, and a plain `contains` cannot tell an
    /// explanation from a call. Same lesson as [`without_comments`] one screen up — a guard has to
    /// read the code, or it reads the documentation about the code.
    fn without_kotlin_comments(source: &str) -> String {
        let mut out = String::new();
        let mut rest = source;
        loop {
            let block = rest.find("/*");
            let line = rest.find("//");
            let (start, close, skip) = match (block, line) {
                (Some(b), Some(l)) if b < l => (b, "*/", 2),
                (Some(b), None) => (b, "*/", 2),
                (_, Some(l)) => (l, "\n", 0),
                (None, None) => break,
            };
            out.push_str(&rest[..start]);
            let Some(end) = rest[start..].find(close) else {
                break;
            };
            rest = &rest[start + end + skip..];
        }
        out.push_str(rest);
        out
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

    #[test]
    fn the_comment_stripper_keeps_the_code_and_drops_the_prose() {
        let source = "// asks for WRITE_EXTERNAL_STORAGE one day\nval a = 1\n/* WRITE_EXTERNAL_STORAGE */\nval b = 2";
        let code = without_kotlin_comments(source);
        assert!(!code.contains("WRITE_EXTERNAL_STORAGE"), "{code:?}");
        assert!(
            code.contains("val a = 1") && code.contains("val b = 2"),
            "{code:?}"
        );
    }

    // ── The A4 document through Android's own print service (hub#2008) ───────────────────────
    //
    // The WebView that prints lives in Kotlin (`HtmlPrinter.kt`); Rust owns the command name and
    // the argument key, both bare literals on the two sides.

    const HTML_PRINTER_KT: &str =
        include_str!("../android/src/main/java/com/erplora/android/HtmlPrinter.kt");

    // ── hub#2644: the question before a link links another hub ─────────────────────────────

    fn question_in(language: &str) -> Wording {
        Wording {
            title: format!("title-{language}"),
            message: format!("message-{language}"),
            open: format!("open-{language}"),
            cancel: format!("cancel-{language}"),
        }
    }

    #[test]
    fn the_hub_question_crosses_to_kotlin_in_both_languages_under_the_keys_kotlin_reads() {
        let (en, es) = (question_in("en"), question_in("es"));
        let json = serde_json::to_value(AskToOpenHubArgs { en: &en, es: &es }).expect("serializable");
        for language in ["en", "es"] {
            for key in ["title", "message", "open", "cancel"] {
                assert_eq!(json[language][key], format!("{key}-{language}"), "{language}.{key}");
            }
        }
        let code = without_kotlin_comments(ERPLORA_ANDROID_PLUGIN_KT);
        let mut literals = vec![
            format!("fun {ASK_TO_OPEN_HUB_COMMAND}(invoke: Invoke)"),
            "put(\"open\"".to_string(),
        ];
        literals.extend(["title", "message", "open", "cancel"].map(|key| format!("getString(\"{key}\"")));
        for literal in literals {
            assert!(
                code.contains(&literal),
                "{literal} is not in ErploraAndroidPlugin.kt — a link to another hub would never be asked \
                 about on a real device (hub#2644)"
            );
        }
        let languages = without_kotlin_comments(HUB_QUESTION_KT);
        for literal in ["\"es\"", "\"en\""] {
            assert!(languages.contains(literal), "{literal} is not in HubQuestion.kt");
        }
    }

    const HUB_QUESTION_KT: &str =
        include_str!("../android/src/main/java/com/erplora/android/HubQuestion.kt");

    #[test]
    fn only_an_explicit_open_opens() {
        let read = |answer| serde_json::from_value::<AskToOpenHubAnswer>(answer).map(|a| a.open);
        assert_eq!(read(serde_json::json!({ "open": true })).ok(), Some(true));
        assert_eq!(read(serde_json::json!({ "open": false })).ok(), Some(false));
        assert_eq!(read(serde_json::json!({})).ok(), Some(false), "an answer without `open` linked the hub");
    }

    #[test]
    fn no_page_can_show_or_answer_the_hub_question() {
        // Only Rust asks: a command in `build.rs` gets an `allow-*` permission a capability could
        // grant, and the page is exactly who must not be able to answer (hub#2644).
        let build = include_str!("../build.rs");
        let permissions = include_str!("../permissions/default.toml");
        for name in ["ask_to_open_hub", "ask-to-open-hub", ASK_TO_OPEN_HUB_COMMAND] {
            assert!(!build.contains(name), "build.rs declares {name}");
            assert!(!permissions.contains(name), "the default permissions grant {name}");
        }
    }

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
        // The same frontier as the desktop print window's CSP (hub#2006): the html comes from a
        // remote page, so the WebView that renders it runs no script and exposes no bridge.
        let code = without_kotlin_comments(HTML_PRINTER_KT);
        assert!(code.contains("javaScriptEnabled = false"), "{code}");
        assert!(!code.contains("javaScriptEnabled = true"), "{code}");
        assert!(!code.contains("addJavascriptInterface"), "{code}");
    }

    #[test]
    fn desktop_refuses_to_print_through_the_android_plugin() {
        // Unreachable by construction (the shell only routes here on Android), and a refusal
        // anyway: a resolved call would read as a dialog that opened (hub#475).
        let desktop = ErploraAndroid::<tauri::Wry>(std::marker::PhantomData);
        assert!(desktop.print_html("<p>F-1</p>").is_err());
    }

    // ── Kotlin answers with no data (hub#2024) ──────────────────────────────────────────────
    //
    // `invoke.resolve()` with no data reaches Rust as JSON `null`. Read as a struct, that `null`
    // is a deserialize error, so a call that DID its job came back as a failure: the ticket was
    // on paper and the print host reported it `failed`.

    #[test]
    fn a_bluetooth_answer_with_no_data_is_a_ticket_on_paper() {
        for answer in [serde_json::Value::Null, serde_json::json!({})] {
            assert!(
                serde_json::from_value::<BluetoothPrintAnswer>(answer.clone()).is_ok(),
                "{answer} must read as a printed ticket"
            );
        }
    }

    /// The Rust source outside this test module.
    fn production_rust() -> &'static str {
        const LIB_RS: &str = include_str!("lib.rs");
        LIB_RS
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("lib.rs has code before its tests")
    }

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
        let answers = rust_answer_types(rust);
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
                 its job would come back as a failure (hub#2024)"
            );
        }
    }

    // ── hub#2360: the tap that started the app ───────────────────────────────────────────────────
    //
    // The notification plugin reports it while it is still loading, before the page listens, and the
    // event is lost. Kotlin keeps it and the shell asks for it once — three places that have to agree
    // on one command name and one answer shape, none of which the compiler sees.

    #[test]
    fn the_kept_launch_tap_reads_with_and_without_a_tap() {
        let kept: NoticeTapAnswer =
            serde_json::from_str(r#"{"tap":{"id":7,"notification":"{\"extra\":{\"path\":\"/m/kds\"}}"}}"#)
                .expect("a kept tap reads");
        assert_eq!(
            kept.tap,
            Some(LaunchNoticeTap { id: 7, notification: Some(r#"{"extra":{"path":"/m/kds"}}"#.into()) })
        );
        let none: NoticeTapAnswer = serde_json::from_str(r#"{"tap":null}"#).expect("no tap reads");
        assert_eq!(none.tap, None);
        let bare: NoticeTapAnswer = serde_json::from_str("{}").expect("an empty answer reads");
        assert_eq!(bare.tap, None);
    }

    #[test]
    fn kotlin_keeps_the_launch_tap_and_hands_it_over_once_hub2360() {
        const NOTICE_TAPS_KT: &str = include_str!("../android/src/main/java/com/erplora/android/NoticeTaps.kt");
        let production = LIB_RS.split("#[cfg(test)]").next().unwrap_or_default();
        assert!(
            production.contains("run_mobile_plugin::<NoticeTapAnswer>(\"takeNoticeTap\", Empty {})"),
            "Rust no longer asks Kotlin for the kept tap by the name Kotlin answers to"
        );
        let before = PLUGIN_KT.split("fun takeNoticeTap(invoke: Invoke)").next().unwrap_or_default();
        assert!(before.trim_end().ends_with("@Command"), "Kotlin takeNoticeTap is not a @Command");
        let command = PLUGIN_KT.split("fun takeNoticeTap(invoke: Invoke)").nth(1).expect("no Kotlin takeNoticeTap");
        let body = command.split("\n    }").next().unwrap_or_default();
        assert!(body.contains("NoticeTaps.take()"), "takeNoticeTap does not hand over the tap the box kept");
        let load = PLUGIN_KT.split("override fun load(webView: WebView)").nth(1).expect("Kotlin does not look at the launch");
        let load = load.split("\n    }").next().unwrap_or_default();
        assert!(
            load.contains("NoticeTaps.pageLoading(activity, activity.intent)"),
            "load does not hand the box the intent that started the app"
        );
        // The memory has to outlive the process: back through the icon after the system killed it,
        // the task's intent is the old tap again (rv-2411).
        assert!(
            NOTICE_TAPS_KT.contains("NoticeTapBox(PreferencesMemory(context.applicationContext))"),
            "the box no longer remembers the kept tap outside the process"
        );
        assert!(
            NOTICE_TAPS_KT.contains("prefs.edit().putString(LAST_KEPT, key).apply()"),
            "the kept tap is not written to the app's preferences"
        );
    }
}

