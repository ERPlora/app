//! ERPlora shell: a **thin** native client around remote pages.
//!
//! The window loads REMOTE ORIGINS: the **SaaS onboarding** (`{saas}/shell/`) on the first start
//! and, once captured, the **hub PWA** (`hub_url`). The capture contract is the `?shell=1` marker:
//! the SaaS redirects to the hub with it (`/shell/open/<id>/` → 302 to `{hub}/?shell=1`) and the
//! shell persists the ORIGIN of that navigation as `hub.url`; later starts load the hub directly.
//! `forget_hub` (for example after a 410 `hub_not_found`) deletes the capture and sends the window
//! back to the onboarding.
//!
//! What is NOT here: an embedded runtime, a local database, an entitlement gate or a machine token
//! in a keychain. The hub enforces entitlements on the server. `invoke` is ONLY for what is native
//! (`device_context`, `forget_hub`) and for HARDWARE (`erplora_*` → `erplora-peripherals`: the
//! shell IS the hardware bridge).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;

/// What the window shows when the network dies under it.
mod connectivity;
use connectivity::{ShellNav, spawn_connectivity_guard};
/// Only the linked hub drives the device.
mod hub_link;
/// A link to another hub waits for the person's word.
mod link_consent;
mod navigation;
mod notice_tap;
pub use navigation::{NavigationVerdict, navigation_verdict};
mod native_print;
pub use native_print::{
    check_print_document, native_print_supported, print_document_id, print_document_url, print_window_label,
    print_window_may_navigate, PrintDocuments, MAX_PRINT_DOCUMENT_BYTES, PRINT_DOCUMENT_CSP,
    PRINT_SCHEME,
};
#[cfg(desktop)]
pub use native_print::open_print_window;

/// Stable per-installation device id, sent as `X-Device-Id` at login (one session per device).
const DEVICE_ID_FILE: &str = "device.id";
/// Persisted origin of the hub PWA captured through `?shell=1`.
const HUB_URL_FILE: &str = "hub.url";
/// Persistent registry of hardware devices (same format as the standalone bridge).
const DEVICES_FILE: &str = "devices.json";

/// Base URL of the SaaS (onboarding). Baked-in default; only a fork or a self-hosted install
/// sets it.
pub const ENV_SAAS_URL: &str = "ERPLORA_SAAS_URL";
/// Override of the FULL start URL (development only: point at a local SaaS or a dev PWA).
pub const ENV_SHELL_URL: &str = "ERPLORA_SHELL_URL";
const DEFAULT_SAAS_URL: &str = "https://erplora.com";

/// Shell error, serialized to the frontend as a string.
#[derive(Debug, thiserror::Error)]
pub enum ShellError {
    #[error("io error: {0}")]
    Io(String),
    /// The address is not one this app will hand to a browser ([`external_browser_url`]).
    #[error("external_url_refused")]
    ExternalUrlRefused,
    /// There is a browser to ask for and it said no — or there is none at all (an Android with no
    /// browser installed answers `ActivityNotFoundException`). Either way the trip did not happen,
    /// and the page has to say so instead of pretending it did.
    #[error("external_url_unavailable: {0}")]
    ExternalUrlUnavailable(String),
    /// This device has no Downloads folder the USER can open, so there is nowhere to save a file
    /// that would count as saved ([`downloads_dir_is_reachable`]). The page turns this
    /// one into its own sentence: it is the only refusal the user can act on.
    #[error("downloads_unreachable")]
    DownloadsUnreachable,
    /// What the page sent is not a file name ([`download_file_name`]) — or the file could not be
    /// written under any name. Either way nothing was saved.
    #[error("download_refused")]
    DownloadRefused,
    /// This device has no NFC reader — a desktop, or a tablet sold without one. The page
    /// stops offering the tap and stops polling; the badge keeps arriving through the USB reader.
    #[error("nfc_unavailable")]
    NfcUnavailable,
    /// There IS a reader and it is switched off. The only one of the three the user can fix, so it
    /// is the only one that gets a sentence pointing at the settings.
    #[error("nfc_disabled")]
    NfcDisabled,
    /// The card answers with a fresh id on every tap, so it can never be matched after enrolment.
    /// Said out loud rather than enrolled: the failure would otherwise surface as an employee
    /// locked out by a card that demonstrably worked the day it was set up.
    #[error("nfc_random_uid")]
    NfcRandomUid,
    /// The page sent no document to print, or one too big to hold.
    #[error("print_document_refused")]
    PrintDocumentRefused,
    /// This platform has no system print dialog for a webview (iOS). The print door
    /// takes its usual route instead.
    #[error("native_print_unsupported")]
    NativePrintUnsupported,
    /// The print window could not be opened.
    #[error("native_print_failed: {0}")]
    NativePrintFailed(String),
}

impl Serialize for ShellError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

// ── Device identity (X-Device-Id) ────────────────────────────────────────────────────────────────

/// Device identity the frontend sends at login (one session per device), as `X-Client-Type` +
/// `X-Device-Id` + `X-Device-Platform`.
#[derive(Debug, Clone, Serialize)]
pub struct DeviceContext {
    pub id: String,
    pub client_type: String,
    pub platform: String,
    /// Where THIS binary came from: `play`, `msstore` or `direct`.
    ///
    /// Only the shell can tell. The web is served by the hub, so a flag of its own would be the
    /// same for every installation and could not tell a Play copy from one installed by hand.
    /// Here it is baked in at compile time: the store jobs of the release workflow export
    /// `ERPLORA_DISTRIBUTION`, and everything else (Windows installer, `.deb`, AppImage) stays
    /// `direct`, which is exactly the copy that must keep announcing new versions.
    pub distribution: String,
}

/// The channel this binary came through. Each platform resolves it differently **because CI
/// builds them differently**:
///
/// - **Android**: the AAB is its own build and only goes to Play, so the mark is baked in at
///   compile time (`ERPLORA_DISTRIBUTION=play` in that job). `option_env!` rather than `env!`:
///   without the variable the build does not fail, it declares `direct`.
/// - **Windows**: the MSIX is packaged **from the same `.exe`** as the regular installer (the
///   workflow does that on purpose so the app is built only once), so a compile-time flag would
///   be the same for both channels. It has to be checked at run time: a packaged app runs from
///   `WindowsApps`, one installed through the `.exe`/`.msi` does not.
///
/// The bias of the errors is deliberate. Erring towards `direct` leaves a store copy with one
/// notice too many (ugly, and review catches it); erring towards `msstore` would leave a counter
/// till never learning that there is a new version, which is the problem this exists to avoid.
/// When in doubt, `direct`.
fn distribution_channel() -> &'static str {
    if let Some(forced) = option_env!("ERPLORA_DISTRIBUTION") {
        return match forced {
            "play" => "play",
            "msstore" => "msstore",
            _ => "direct",
        };
    }
    #[cfg(target_os = "windows")]
    {
        // `GetCurrentPackageFullName` would be the canonical check, but it needs the `windows`
        // crate for a single call. The path is the same observable fact: the MSIX runtime mounts
        // the app under `%ProgramFiles%\WindowsApps\<identity>`, where no installer ever lands.
        if std::env::current_exe()
            .ok()
            .map(|p| p.to_string_lossy().to_lowercase().contains("windowsapps"))
            .unwrap_or(false)
        {
            return "msstore";
        }
    }
    "direct"
}

/// Reads (or creates and persists) a stable per-installation device id in `app_data_dir`.
/// It survives webview cache wipes: it identifies **this** installation of the shell.
fn ensure_device_id(cache_dir: &Path) -> Result<String, ShellError> {
    let path = cache_dir.join(DEVICE_ID_FILE);
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    std::fs::create_dir_all(cache_dir).map_err(|e| ShellError::Io(e.to_string()))?;
    std::fs::write(&path, &id).map_err(|e| ShellError::Io(e.to_string()))?;
    Ok(id)
}

/// Returns the device identity the page sends at login.
#[tauri::command]
fn device_context(app: tauri::AppHandle) -> Result<DeviceContext, ShellError> {
    use tauri::Manager;
    let cache_dir: PathBuf = app
        .path()
        .app_data_dir()
        .map_err(|e| ShellError::Io(e.to_string()))?;
    let id = ensure_device_id(&cache_dir)?;
    // Without the `android` branch the shell announced itself as `hub-desktop`/`desktop` FROM A
    // PHONE (seen on the emulator, API 37), so the server could not tell a tablet from a counter
    // till, neither in the session list nor in the one-session-per-device rule.
    let platform = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "android") {
        "android"
    } else if cfg!(target_os = "ios") {
        "ios"
    } else {
        "desktop"
    };
    let client_type = if cfg!(any(target_os = "android", target_os = "ios")) {
        "hub-mobile"
    } else {
        "hub-desktop"
    };
    Ok(DeviceContext {
        id,
        // The server's client taxonomy; the concrete platform travels in X-Device-Platform.
        client_type: client_type.to_string(),
        platform: platform.to_string(),
        distribution: distribution_channel().to_string(),
    })
}

// ── Deep link `erplora://` ───────────────────────────────────────────────────────────────────────
//
// A link is the one thing that lets a page OUTSIDE the app steer the app, so it is a trust
// boundary and not a convenience: whoever writes the link picks the destination, and the
// destination is the window that owns the hardware handlers. The app therefore never opens what
// the link says — it RESOLVES it, and only a hub of ours resolves at all. Anything else returns
// `None`, and `None` means "leave the window where it is": the browser that issued the link is the
// one that owns the fallback.

/// Scheme of the installable app's deep link, registered against the single app identity
/// `com.erplora.app`.
pub const DEEP_LINK_SCHEME: &str = "erplora";

/// The only action of the grammar: `erplora://hub/<host>`.
const DEEP_LINK_HUB_ACTION: &str = "hub";

/// Registrable domain every ERPlora hub lives under (`{slug}.{aura}.erplora.com`).
const HUB_DOMAIN_SUFFIX: &str = ".erplora.com";

/// The apex itself — the SaaS: marketing, the dashboard, billing and the module checkout. It is
/// [`HUB_DOMAIN_SUFFIX`] without the leading dot that makes that one a suffix match, and it is
/// deliberately NOT a hub: only [`external_browser_url`] accepts it.
const ERPLORA_DOMAIN: &str = "erplora.com";

/// Stripe's hosted checkout, matched by EXACT host. The assistant's paid upgrade is its own Stripe
/// subscription and the SaaS answers it with this page directly — there is no erplora.com page in
/// between to send the browser to. Without it the assistant's «See plans» could only fail inside
/// the installed app. No other Stripe host, and no suffix match: the dashboard or a look-alike is
/// not a checkout the till starts.
const STRIPE_CHECKOUT_HOST: &str = "checkout.stripe.com";

/// The two sites of the AEAT's QR check: the real one and its test one (`QR_HOSTS` of
/// `apps/verifactu/chain.py` in the Hub). The Hub links a record to the site it was born on.
const AEAT_QR_HOSTS: [&str; 2] = ["www2.agenciatributaria.gob.es", "prewww2.aeat.es"];

/// The page that checks a VeriFactu QR on [`AEAT_QR_HOSTS`] (`QR_PATH` in the same Hub file).
const AEAT_QR_PATH: &str = "/wlpl/TIKE-CONT/ValidarQR";

/// Is this the AEAT's QR check (app#20)? Host AND path, unlike [`STRIPE_CHECKOUT_HOST`]: the AEAT
/// serves far more than the check under those two domains, and this door only has to lead to it.
///
/// No port either: `port()` is `None` when the address spells the 443 that https implies, so anything
/// else is another service of that machine. Judged on the parsed URL, where the dots are resolved and
/// percent-escapes stay as written, so `…/ValidarQR/../Otra` and `…/ValidarQR%2F..%2FOtra` both miss.
fn is_aeat_qr_check(url: &tauri::Url) -> bool {
    url.host_str().is_some_and(|host| AEAT_QR_HOSTS.contains(&host))
        && url.port().is_none()
        && url.path() == AEAT_QR_PATH
}

/// Is this host a hub of ours — `<label>[.<label>…].erplora.com`?
///
/// The apex has no leading dot to strip, which is exactly why it is excluded: `erplora.com` also
/// serves marketing, billing and a third-party checkout, and is deliberately kept out of the
/// hardware capability. Auras are matched by WILDCARD and never enumerated: hubs live at
/// `{slug}.{aura}.erplora.com`, so a list would turn "open a new aura" into "ship a new app to
/// every till".
///
/// Expects an already-lowercased host — for `https` the URL parser guarantees it, and
/// [`hub_url_for_host`] lowercases the opaque host of a deep link before asking.
fn is_hub_domain(host: &str) -> bool {
    match host.strip_suffix(HUB_DOMAIN_SUFFIX) {
        Some(subdomain) => !subdomain.is_empty() && subdomain.split('.').all(is_dns_label),
        None => false,
    }
}

/// Is this host THIS machine? Loopback is the development exemption: nothing outside the machine
/// can serve it, so nobody else can steer a window that points there.
fn is_loopback_host(host: &str) -> bool {
    // `[::1]` with brackets is how `Url::host_str` serializes the IPv6 loopback.
    matches!(host, "127.0.0.1" | "localhost" | "[::1]")
}

/// Is this a plain DNS label? Deliberately narrower than the RFC: lowercase ASCII only, so a
/// percent-escape, a homograph or a stray `@` can never pass for a label.
fn is_dns_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The trust boundary: which destinations the app is willing to open for a link.
///
/// Mirrors `remote.urls` of the hardware capability (`https://*.erplora.com/*` plus loopback for
/// development) on purpose. A destination the capabilities would not trust must not be reachable
/// through a link either — and the reverse would be just as bad: opening a page that cannot
/// `invoke` leaves a till that looks fine and cannot print.
///
/// The production half of the rule lives in [`is_hub_domain`], shared with what the shell is
/// willing to REMEMBER ([`trusted_hub_origin`]): one predicate, so the two cannot drift apart.
fn hub_url_for_host(raw_host: &str) -> Option<String> {
    let host = raw_host.trim().to_ascii_lowercase();

    // Development: the hub is plain http on loopback. Not reachable from another machine, so a
    // third party cannot steer it — but the port must be digits and nothing else, or
    // `127.0.0.1:8787@evil.com` would walk straight through.
    for prefix in ["127.0.0.1:", "localhost:"] {
        if let Some(port) = host.strip_prefix(prefix) {
            if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            return Some(format!("http://{host}/?shell=1"));
        }
    }

    // Production: the same rule the capture applies to what it is willing to remember.
    if !is_hub_domain(&host) {
        return None;
    }
    Some(format!("https://{host}/?shell=1"))
}

/// Resolves a deep link into the URL the window must navigate to. `None` = not a link of ours, or
/// not a destination of ours → **do not navigate**.
///
/// The destination is rebuilt from the host alone, so query and fragment are ignored by
/// construction and no parameter can smuggle a second URL in. The `?shell=1` marker is the capture
/// contract: navigating with it is what makes the shell remember this hub as `hub.url`, so a link
/// opens the app AND leaves it pointing at the right hub next time.
pub fn resolve_deep_link(raw: &str) -> Option<String> {
    let url: tauri::Url = raw.trim().parse().ok()?;
    if url.scheme() != DEEP_LINK_SCHEME {
        return None;
    }
    // Case-insensitive on purpose: for a non-special scheme the host is an OPAQUE host, so the URL
    // parser does NOT lowercase it the way it would for `https`. A link typed or reflowed in
    // uppercase by a mail client is still the same link.
    if !url.host_str()?.eq_ignore_ascii_case(DEEP_LINK_HUB_ACTION) {
        return None;
    }
    // Credentials or a port on the action are not part of the grammar. They carry no meaning here,
    // so accepting them would only add shapes of the same link that have to be reasoned about.
    if !url.username().is_empty() || url.password().is_some() || url.port().is_some() {
        return None;
    }
    let mut segments = url.path_segments()?;
    let target = segments.next()?;
    // Exactly one segment. A single empty tail is the trailing slash of `…/erplora.com/`.
    if segments.any(|s| !s.is_empty()) {
        return None;
    }
    hub_url_for_host(target)
}

/// Picks the deep link out of the process arguments — the cold start path on Windows and Linux,
/// where the OS launches the executable with the URL appended instead of delivering an event.
///
/// Every argument goes through [`resolve_deep_link`], so a hostile one is refused here too: this
/// list is attacker-reachable, since whatever the browser hands the registered handler lands in it
/// verbatim.
pub fn deep_link_from_args<I: IntoIterator<Item = String>>(args: I) -> Option<String> {
    args.into_iter().find_map(|arg| resolve_deep_link(&arg))
}

// ── The way OUT: the user's own browser ──────────────────────────────────────────────────────────

/// The address `openExternal` may hand to the SYSTEM browser, normalized — or `None`.
///
/// The web app has one door out of the till, and behind it are the buttons that CHARGE: the module
/// checkout, which lives on the SaaS website *because Google Play does not allow paying for digital
/// goods inside the app*, the plans page, the billing portal, the plan-limit upsell. In the browser
/// that door is a new tab; inside the installed app `window.open` opens nothing at all — no plugin
/// is exposed to the page and the webview spawns no window.
///
/// Letting the page ask for a browser is a frontier of its own: an unchecked opener turns the till
/// into a launcher for any address the page names — a phishing page wearing the trust of an
/// installed app, a `file://` path handed to whatever the desktop associates with it, or a custom
/// scheme that starts another program. So the destination is checked here, just as the hardware
/// capability checks who may drive the hardware and [`resolve_deep_link`] what a link may open.
///
/// **The apex is IN, and that is the difference from the other two boundaries.** [`is_hub_domain`]
/// and [`trusted_hub_origin`] refuse `erplora.com` on purpose, because they answer "may this page
/// open the cash drawer / become this till's home?". This one answers "may the user's browser be
/// sent here?" — and the apex is the SaaS, where those paid destinations live. A visit grants
/// nothing: the page opens in the browser's own sandbox, with no `invoke` and no way back in.
pub fn external_browser_url(raw: &str) -> Option<String> {
    // The WHATWG parser already strips leading and trailing spaces, so `trim` changes no outcome.
    // It stays so that the code reads "trim, then parse" without relying on that clause of the URL
    // spec.
    let url: tauri::Url = raw.trim().parse().ok()?;

    // Credentials are the oldest confusion trick there is, and the dangerous half is the one the
    // host check cannot catch: `https://support%40evil.example@erplora.com/` really is our host,
    // and the browser would show a page nobody at ERPlora wrote under an address that reads like
    // support.
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }

    let host = url.host_str()?;
    let allowed = match url.scheme() {
        // Everything we serve: the SaaS at the apex plus every hub, auras by wildcard.
        // Plus Stripe's hosted checkout, where the assistant's upgrade is paid, and the
        // AEAT's public check of a VeriFactu QR (app#20): only a link, nothing is sent to the AEAT.
        "https" => {
            host == ERPLORA_DOMAIN
                || host == STRIPE_CHECKOUT_HOST
                || is_hub_domain(host)
                || is_aeat_qr_check(&url)
        }
        // Development against a local SaaS (`VITE_CLOUD_API_URL=http://127.0.0.1:8001`). Nothing
        // outside this machine can serve loopback, so nobody else can steer it.
        "http" => is_loopback_host(host),
        // Nothing else. `file:`, `javascript:` and custom schemes are not "a web address the user
        // wanted": they are instructions to the operating system.
        _ => false,
    };

    allowed.then(|| url.to_string())
}

// ── The way DOWN: a file the user can find afterwards ────────────────────────────────────────────

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

// ── Shell state: capturing and persisting hub_url ────────────────────────────────────────────────

/// Normalizes the SaaS base URL: no surrounding spaces and no trailing `/`.
fn normalize_base(base: &str) -> String {
    base.trim().trim_end_matches('/').to_string()
}

/// URL of the SaaS onboarding that the first start loads.
fn onboarding_url(base: &str) -> String {
    format!("{}/shell/", normalize_base(base))
}

/// SaaS base URL: the [`ENV_SAAS_URL`] environment variable or the baked-in default.
fn saas_base_url() -> String {
    normalize_base(
        &std::env::var(ENV_SAAS_URL)
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_SAAS_URL.to_string()),
    )
}

/// The ORIGIN this installation may enter "app mode" on, or `None` if it may not.
///
/// The trust boundary of `hub.url`, and the reason it is a boundary at all: whatever ends up in
/// that file is what the window opens FULL-SCREEN, chrome-less and titled "ERPlora" on every cold
/// start from then on. So the destination is checked, not merely parsed — a hub of ours, or the
/// loopback of this machine, and nothing else.
///
/// It is deliberately the SAME rule as [`hub_url_for_host`] (what a deep link may open) and as
/// `capabilities/*.json` (what may drive the hardware); `tests/remote_acl.rs`
/// asserts the capture and the capabilities agree origin by origin. They have to: remembering an
/// origin the capabilities refuse produces a till that looks fine and cannot print, because every
/// `invoke` from it — down to `device_context` — is rejected by Tauri's ACL.
///
/// `https` everywhere, plus plain `http` on loopback for development. There is no opaque-origin
/// case left to reject: only `http`/`https` get here, and both are special schemes whose origin is
/// always a tuple.
fn trusted_hub_origin(url: &tauri::Url) -> Option<String> {
    // The `None` branch is unreachable: the scheme gate below only lets `http`/`https` through, and
    // neither parses without a host. The `?` stays so the function is total on its own terms
    // instead of depending on the order of the two checks.
    let host = url.host_str()?;
    let allowed = match url.scheme() {
        "https" => is_hub_domain(host) || is_loopback_host(host),
        // A deployed hub is always https; plain http is the development exemption.
        "http" => is_loopback_host(host),
        _ => false,
    };
    if !allowed {
        return None;
    }
    Some(url.origin().ascii_serialization())
}

/// Capture contract: when the navigation carries the `?shell=1` marker **and** the destination is
/// one of ours ([`trusted_hub_origin`]), returns the ORIGIN to persist as `hub_url`.
///
/// The marker says "remember me", not "trust me": it rides on the URL being navigated to, and
/// `on_navigation` only refuses pages of the SaaS, and only in the Play copy
/// ([`navigation_verdict`]) — so without the second filter one link is enough to leave the
/// till booting on somebody else's page, for good.
pub fn shell_capture_origin(url: &tauri::Url) -> Option<String> {
    let has_marker = url.query_pairs().any(|(k, v)| k == "shell" && v == "1");
    if !has_marker {
        return None;
    }
    let captured = trusted_hub_origin(url);
    if captured.is_none() {
        log::warn!(
            "shell: {} asks to be remembered and is not one of our hubs; not captured",
            url.origin().ascii_serialization()
        );
    }
    captured
}

/// Reads the persisted `hub.url`, reduced to its ORIGIN and passed through [`trusted_hub_origin`].
///
/// It is revalidated on READ, not only on write, because guarding the capture only protects the
/// tills that have not captured anything yet: one that already ran a build without the filter
/// carries the foreign origin in the file, and an update that trusts it again does not fix exactly
/// the case that matters. It is also a text file in the user's data directory: nothing stops
/// anyone from editing it.
fn load_hub_url(cache_dir: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(cache_dir.join(HUB_URL_FILE)).ok()?;
    // No `is_empty` guard: an empty (or blank) file does not parse as a URL, so the `?` below is
    // the same rejection written once instead of twice.
    //
    // The WHATWG parser strips leading and trailing C0 and space itself, so the `trim` changes no
    // outcome today. It stays because `hub.url` is OUR file format: normalizing it here says so,
    // instead of leaning on a parsing detail of a dependency to make our own writes readable.
    let trimmed = raw.trim();
    let parsed: tauri::Url = trimmed.parse().ok()?;
    let trusted = trusted_hub_origin(&parsed);
    if trusted.is_none() {
        log::warn!(
            "shell: the remembered hub ({trimmed}) is not one of ours; back to the onboarding"
        );
    }
    trusted
}

/// Persists the captured origin as `hub.url` (creating `cache_dir` if it does not exist).
fn persist_hub_url(cache_dir: &Path, origin: &str) -> Result<(), ShellError> {
    std::fs::create_dir_all(cache_dir).map_err(|e| ShellError::Io(e.to_string()))?;
    std::fs::write(cache_dir.join(HUB_URL_FILE), origin).map_err(|e| ShellError::Io(e.to_string()))
}

/// Forgets the captured hub (best effort: no error if there was none).
fn clear_hub_url(cache_dir: &Path) {
    let _ = std::fs::remove_file(cache_dir.join(HUB_URL_FILE));
}

/// What the remembered hub answers when it is asked at start-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HubProbe {
    /// It answered with this HTTP status.
    Status(u16),
    /// It could not be reached: no network, DNS down, timeout.
    Unreachable,
}

/// Must the remembered `hub.url` be forgotten?
///
/// **Only** if the hub no longer exists. The decision is asymmetric on purpose: forgetting too
/// eagerly deletes the user's hub and sends them through the onboarding again, while forgetting
/// too little only leaves an ugly screen that fixes itself as soon as the hub is back. When in
/// doubt, it is kept.
///
/// It exists because [`forget_hub`] **does not reach this case**: the frontend calls it on a 410
/// from the server, and if the hub was deleted the PWA never loads at all — the 404 is served by
/// the edge. Without this the app opens on «404 page not found» for good, with no way out through
/// the UI. Seen on a Mac with a hub that a production purge had removed.
fn should_forget_hub(probe: HubProbe) -> bool {
    match probe {
        // The hub is not there. 410 is also the server's explicit `hub_not_found` contract.
        HubProbe::Status(404) | HubProbe::Status(410) => true,
        // Everything else (alive, redirecting to the login, unauthenticated, down or unreachable)
        // is a hub that DOES exist.
        _ => false,
    }
}

/// The window's start URL, by precedence: the start-up deep link → the development override
/// ([`ENV_SHELL_URL`]) → the persisted `hub.url` (app mode) → the SaaS onboarding. Pure so that it
/// can be tested.
///
/// The deep link goes **first** because it is what the user is asking for NOW: the override is
/// static configuration and the persisted hub is last time's. If either of them won, pressing
/// «Open Terminal» on hub B from a till that remembers hub A would open A — the wrong business —
/// without a single error to give it away.
fn initial_url_for(
    deep_link: Option<&str>,
    override_url: Option<&str>,
    persisted: Option<&str>,
    saas_base: &str,
) -> String {
    if let Some(target) = deep_link {
        return target.to_string();
    }
    if let Some(dev) = override_url {
        return dev.to_string();
    }
    if let Some(origin) = persisted {
        return format!("{}/", normalize_base(origin));
    }
    onboarding_url(saas_base)
}

/// Which pages may choose a hub (the entries of [`hub_link::HubLink`]) and which hub the window
/// starts linked to, by the precedence of [`initial_url_for`].
///
/// The development override is an entry (it may boot at a local SaaS that chooses the hub) and is
/// linked only when it is a hub of ours that is not the SaaS itself (a local PWA).
fn boot_link(
    override_url: Option<&str>,
    persisted: Option<&str>,
    saas_base: &str,
) -> (Vec<String>, Option<String>) {
    let origin = |raw: &str| raw.parse::<tauri::Url>().ok().map(|u| u.origin().ascii_serialization());
    let hub = |raw: &str| raw.parse::<tauri::Url>().ok().as_ref().and_then(trusted_hub_origin);
    let saas = origin(saas_base);
    let mut entries: Vec<String> = saas.iter().cloned().collect();
    if let Some(dev) = override_url.and_then(origin) {
        if !entries.contains(&dev) {
            entries.push(dev);
        }
    }
    let linked = match override_url {
        Some(dev) => hub(dev).filter(|dev| Some(dev) != saas.as_ref()),
        None => persisted.map(str::to_string),
    };
    (entries, linked)
}

/// What a cold start does with the link it was launched by.
#[derive(Debug, PartialEq, Eq)]
enum ColdStartLink {
    /// Launched without one.
    None,
    /// A link to the hub the device is linked to: the window boots on it, as it always did.
    Open(String),
    /// A link to any other hub: the window boots where it would without a link, and the person is
    /// asked ([`open_hub_from_link`]) before anything is linked.
    Ask(String),
}

fn cold_start_link(deep_link: Option<&str>, linked: Option<&str>) -> ColdStartLink {
    let Some(target) = deep_link else {
        return ColdStartLink::None;
    };
    let hub = target.parse::<tauri::Url>().ok().as_ref().and_then(trusted_hub_origin);
    match hub {
        Some(hub) if linked == Some(hub.as_str()) => ColdStartLink::Open(target.to_string()),
        _ => ColdStartLink::Ask(target.to_string()),
    }
}

/// Where the window goes after forgetting the hub.
///
/// Two callers, two intents. The 410 path (`choose = false`) keeps the plain onboarding: the hub
/// is gone, the SaaS routes as it sees fit — and with a single hub that means straight back in,
/// which is right there. The USER path («switch business», `choose = true`) needs the opposite:
/// `?choose=1` is the SaaS's own affordance for forcing the hub list even when a lone hub would
/// auto-redirect — without it, the owner with two businesses and one tablet bounces right back
/// into the hub they were trying to leave.
fn forget_destination(base: &str, choose: bool) -> String {
    if choose {
        format!("{}?choose=1", onboarding_url(base))
    } else {
        onboarding_url(base)
    }
}

/// Forgets the captured hub and sends the window back to the SaaS onboarding. Two callers: the
/// frontend on a 410 `hub_not_found` (no `choose` → the plain onboarding) and the topbar's «switch
/// business» control (`choose: true` → `?choose=1`, the SaaS hub picker).
/// KEEPS `device.id`, the stable anchor of this installation.
/// The navigation is best effort (no window is not an error).
#[tauri::command]
fn forget_hub(app: tauri::AppHandle, choose: Option<bool>) -> Result<(), ShellError> {
    use tauri::Manager;
    let cache_dir: PathBuf = app
        .path()
        .app_data_dir()
        .map_err(|e| ShellError::Io(e.to_string()))?;
    clear_hub_url(&cache_dir);
    forget_linked_hub(&app);
    if let Some(window) = app.get_webview_window("main") {
        if let Ok(url) =
            forget_destination(&saas_base_url(), choose.unwrap_or(false)).parse::<tauri::Url>()
        {
            let _ = window.navigate(url);
        }
    }
    Ok(())
}

/// `open_external_url` — hands `url` to the user's OWN browser, leaving the till where it is.
///
/// This is what the web app's `openExternal` calls inside the installed app. It is a separate
/// program, not a tab of ours, and that is the point on two counts: the till stays exactly as the
/// user left it while they pay, and the payment happens **outside** the app, which is what lets the
/// Android build ship (Play does not allow digital goods to be paid for inside the app).
///
/// A refusal is returned, never swallowed: the page turns it into something the user can read. A
/// button that does nothing when pressed is the defect this command exists to end.
#[tauri::command]
fn open_external_url(app: tauri::AppHandle, url: String) -> Result<(), ShellError> {
    use tauri_plugin_opener::OpenerExt;

    let Some(target) = external_browser_url(&url) else {
        log::warn!("shell: refused to open {url} externally — not an address of ours");
        return Err(ShellError::ExternalUrlRefused);
    };
    // `None` for `with`, deliberately: the plugin's `"inAppBrowser"` opens an Android Custom Tab,
    // which is chrome our app hosts. What is wanted here is the browser as its own app.
    app.opener()
        .open_url(target, None::<&str>)
        .map_err(|e| ShellError::ExternalUrlUnavailable(e.to_string()))
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
fn save_download(
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

/// `print_document` — the system print dialog for an A4 document the page holds.
///
/// The page keeps the html, the shell keeps the dialog: on the desktop a window of its own shows
/// the document and the OS prints it, with its printer list and «Save as PDF» ([`native_print`]);
/// on Android the plugin renders it in a WebView of its own and opens the system print service
/// (`PrintManager`). Where there is no dialog it answers `native_print_unsupported` and
/// the print door takes its usual route — a refusal, never a pretended page.
///
/// ⚠️ `(async)` is load-bearing: creating a window from a synchronous command deadlocks on Windows.
#[tauri::command(async)]
fn print_document(app: tauri::AppHandle, html: String) -> Result<(), ShellError> {
    if !native_print_supported(std::env::consts::OS) {
        return Err(ShellError::NativePrintUnsupported);
    }
    #[cfg(desktop)]
    {
        native_print::open_print_window(&app, html)
    }
    #[cfg(target_os = "android")]
    {
        use tauri_plugin_erplora_android::ErploraAndroidExt;
        check_print_document(&html)?;
        app.erplora_android()
            .print_html(&html)
            .map_err(|e| ShellError::NativePrintFailed(e.to_string()))
    }
    #[cfg(not(any(desktop, target_os = "android")))]
    {
        let _ = (&app, &html);
        Err(ShellError::NativePrintUnsupported)
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

/// Asks in the background whether the remembered hub still exists and, if it does not, forgets it
/// and sends the window back to the onboarding.
///
/// In the background on purpose: the window is already open and showing the hub, so in the normal
/// case (the hub exists) this goes unnoticed. In the bad case the user sees the 404 for an instant
/// and ends up on the onboarding, which is where they can get out from. Blocking the start to
/// avoid that flicker would penalize **every** start for a rare case.
///
/// A `HEAD` is enough and does not download the whole PWA. The timeout can be short: if the hub
/// does not answer in time it is kept, which is the safe decision ([`should_forget_hub`]).
///
/// It runs on Tauri's **async** runtime with reqwest's async client, NOT on `std::thread` +
/// `reqwest::blocking`. Measured on the API 37 emulator: with the blocking client the request
/// **never left** the device on Android (not a single `HEAD` reached the server), so the rescue did
/// not exist on exactly the platform where the user cannot clear the app's data by hand. On macOS
/// it did work, which is what made it easy to take for granted.
fn spawn_hub_liveness_check(app: tauri::AppHandle, cache_dir: PathBuf, origin: String) {
    tauri::async_runtime::spawn(async move {
        use tauri::Manager;

        let url = format!("{}/", normalize_base(&origin));
        let probe = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(6))
            .build()
        {
            Ok(c) => match c.head(&url).send().await {
                Ok(r) => HubProbe::Status(r.status().as_u16()),
                Err(e) => {
                    log::warn!("shell: could not reach the remembered hub ({origin}): {e}");
                    HubProbe::Unreachable
                }
            },
            Err(e) => {
                log::warn!("shell: could not build the HTTP client: {e}");
                HubProbe::Unreachable
            }
        };
        if !should_forget_hub(probe) {
            return;
        }

        log::info!("shell: the remembered hub ({origin}) is gone ({probe:?}); back to the onboarding");
        clear_hub_url(&cache_dir);
        forget_linked_hub(&app);
        if let Some(window) = app.get_webview_window("main") {
            if let Ok(url) = onboarding_url(&saas_base_url()).parse::<tauri::Url>() {
                let _ = window.navigate(url);
            }
        }
    });
}

/// The device stops answering the hub it was linked to.
fn forget_linked_hub<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri::Manager;
    if let Some(link) = app.try_state::<hub_link::HubLink>() {
        link.forget();
    }
}

/// A link the system handed the app.
///
/// A link to the linked hub just takes the window there. A link to any other hub would hand it the
/// printer, the drawer and the card reader and make it the business the app boots on, and anyone
/// can send one — an email, a web page, a message — so the person in front of the device is asked
/// first, by the system's own dialog: «Open» links it and goes there, anything else leaves the
/// device as it was. With nobody to ask, the link is refused, never followed.
///
/// `target` comes out of [`resolve_deep_link`], already a hub of ours; it is checked again here so
/// this function does not lean on its caller for what it links.
fn open_hub_from_link<R: tauri::Runtime>(app: &tauri::AppHandle<R>, target: &str) {
    use tauri::Manager;
    let Some(hub) = target.parse::<tauri::Url>().ok().as_ref().and_then(trusted_hub_origin) else {
        log::warn!("shell: link ignored, it does not lead to a hub of ours ({target})");
        return;
    };
    let Some(link) = app.try_state::<hub_link::HubLink>() else {
        log::warn!("shell: link ignored, the device does not know its hub yet ({target})");
        return;
    };
    let linked = link.linked();
    if linked.as_deref() == Some(hub.as_str()) {
        navigate_main_window(app, target);
        return;
    }
    let Some(asker) = app.try_state::<link_consent::AskBeforeLinking>() else {
        log::warn!("shell: link ignored, nobody to ask before linking {hub}");
        return;
    };
    let question = link_consent::Question {
        to: link_consent::host_of(&hub),
        from: linked.as_deref().map(link_consent::host_of),
    };
    let handle = app.clone();
    let target = target.to_string();
    asker.ask(
        question,
        Box::new(move |open| {
            if !open {
                log::info!("shell: the person kept this device on its business; {hub} was not linked");
                return;
            }
            if let Some(link) = handle.try_state::<hub_link::HubLink>() {
                link.link(&hub);
            }
            navigate_main_window(&handle, &target);
        }),
    );
}

/// Takes the main window to `url`. Best effort on purpose: without a window (or with a URL that
/// does not parse) there is nothing to steer, and a link must not bring the app down.
fn navigate_main_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>, url: &str) {
    use tauri::Manager;
    let Some(window) = app.get_webview_window("main") else {
        log::warn!("shell: a link arrived but there is no window to steer yet ({url})");
        return;
    };
    match url.parse::<tauri::Url>() {
        Ok(parsed) => {
            if let Err(e) = window.navigate(parsed) {
                log::warn!("shell: could not open the link ({url}): {e}");
            }
        }
        Err(e) => log::warn!("shell: unreadable link destination ({url}): {e}"),
    }
}

/// The retry button of the offline page.
///
/// A plain `location.reload()` on the bundled fallback page would reload THE FALLBACK PAGE: the
/// only control on the only screen the user can reach would lead back to itself.
/// Retry has to happen on this side, because this is the side that can ask the network and move
/// the window. Answers whether the target is reachable so the page can say "still nothing" instead
/// of pretending it did something.
///
/// Granted to the bundled page ONLY (`capabilities/degraded.json`, `local: true`): a command that
/// navigates the main window is not something a remote origin should be able to call.
#[tauri::command]
async fn shell_retry(
    window: tauri::WebviewWindow,
    nav: tauri::State<'_, Arc<ShellNav>>,
) -> Result<bool, ShellError> {
    let reachability = connectivity::probe_and_apply(&window, nav.inner()).await;
    Ok(reachability == connectivity::Reachability::Online)
}

/// Creates the main window on [`initial_url_for`] and registers the `on_navigation` handler that
/// captures `?shell=1` and persists the origin as `hub.url` (a single write per change).
fn open_main_window(app: &tauri::App, cache_dir: Option<PathBuf>) -> tauri::Result<()> {
    use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

    let persisted = cache_dir.as_deref().and_then(load_hub_url);
    let override_url = std::env::var(ENV_SHELL_URL)
        .ok()
        .filter(|v| !v.trim().is_empty());
    // COLD start by deep link: on Windows and Linux the system answers a link by launching the
    // executable again with the URL as an argument, so this is where it arrives. On macOS, iOS and
    // Android it arrives as an event and `on_open_url` picks it up (see `run`).
    let deep_link = deep_link_from_args(std::env::args());
    // The second gate in front of the commands: only the linked hub drives the device.
    let (entries, linked) = boot_link(
        override_url.as_deref(),
        persisted.as_deref(),
        &saas_base_url(),
    );
    // Only a link to the linked hub boots on it; one to another hub asks first.
    let cold = cold_start_link(deep_link.as_deref(), linked.as_deref());
    let opens = match &cold {
        ColdStartLink::Open(target) => Some(target.clone()),
        ColdStartLink::None | ColdStartLink::Ask(_) => None,
    };
    let initial = initial_url_for(
        opens.as_deref(),
        override_url.as_deref(),
        persisted.as_deref(),
        &saas_base_url(),
    );
    let url = match initial.parse::<tauri::Url>() {
        Ok(u) => WebviewUrl::External(u),
        Err(e) => {
            // Degraded: the bundled static page (shell-dist). It should not happen except with a
            // malformed ERPLORA_SHELL_URL/ERPLORA_SAAS_URL.
            eprintln!("shell: invalid start URL ({initial}): {e}; loading the degraded page");
            WebviewUrl::App("index.html".into())
        }
    };

    // When the window boots on a remembered hub, it has to be checked that the hub still exists —
    // but AFTER opening the window, not before: blocking a till's start on a network request would
    // be worse than the screen this tries to avoid.
    //
    // Not when the window boots on a link's hub: this check ends up navigating to the onboarding,
    // and would take away exactly what the person just asked for. A link to ANOTHER hub boots on the
    // remembered one, so that one is checked as usual while the person is asked.
    if override_url.is_none() && opens.is_none() {
        if let (Some(dir), Some(origin)) = (cache_dir.clone(), persisted.clone()) {
            spawn_hub_liveness_check(app.handle().clone(), dir, origin);
        }
    }

    // Where the window is MEANT to be, so the connectivity guard can put it back there. Seeded
    // with the URL the window is about to load; `on_navigation` keeps it current afterwards.
    let initial_target = match &url {
        WebviewUrl::External(u) => u.clone(),
        // The degraded arm above: the env URL is unusable, so the address to come back to is the
        // one the app is meant to boot at. Baked literal, not `saas_base_url()`, because what put
        // us in this arm is precisely an env var that does not parse.
        _ => DEFAULT_SAAS_URL
            .parse()
            .map_err(tauri::Error::InvalidUrl)?,
    };
    let hub_link = hub_link::HubLink::new(entries, Some(&initial_target));
    if let Some(origin) = linked {
        hub_link.link(&origin);
    }
    app.manage(hub_link);
    let nav_state = Arc::new(ShellNav::new(initial_target));
    app.manage(nav_state.clone());

    let last = std::sync::Mutex::new(persisted);
    let watched = nav_state.clone();
    let saas_base = saas_base_url();
    let refusals = app.handle().clone();
    let window = WebviewWindowBuilder::new(app, "main", url)
        .title("ERPlora")
        .inner_size(1280.0, 800.0)
        .min_inner_size(960.0, 600.0)
        // The hardware waits for the page to have LOADED: the window's URL flips at the
        // start of a navigation, while the page that started it still runs until the new one commits.
        .on_page_load(|window, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                if let Some(link) = window.try_state::<hub_link::HubLink>() {
                    link.landed(payload.url());
                }
            }
        })
        .on_navigation(move |nav| {
            // The Play copy follows only the SaaS pages that cannot take money. First,
            // so a refused page is neither remembered as the hub nor watched by the guard below.
            let verdict = navigation_verdict(distribution_channel(), &saas_base, nav);
            if verdict != NavigationVerdict::Allow {
                refuse_navigation(&refusals, verdict, &saas_base, nav);
                return false;
            }
            // `?shell=1` links a hub only when the SaaS chose it, or it is the linked one.
            let captured = refusals.try_state::<hub_link::HubLink>().and_then(|link| link.follow(nav));
            if let (Some(dir), Some(origin)) = (cache_dir.as_deref(), captured) {
                if let Ok(mut guard) = last.lock() {
                    if guard.as_deref() != Some(origin.as_str()) {
                        match persist_hub_url(dir, &origin) {
                            Ok(()) => *guard = Some(origin),
                            Err(e) => eprintln!("shell: could not persist hub.url: {e}"),
                        }
                    }
                }
            }
            // Wherever the webview goes on its own — a link, a redirect, the login chain — that is
            // the page the guard has to keep alive. Our own bundled page is filtered
            // out inside `remote_target`, or the offline screen would end up watching itself.
            if let Some(target) = connectivity::remote_target(nav) {
                watched.set_target(target);
            }
            true
        })
        .build()?;

    // From here on, a load that never lands has an answer.
    spawn_connectivity_guard(window, nav_state);
    // With the window up, a link to another hub gets its question.
    match cold {
        ColdStartLink::Ask(target) => open_hub_from_link(app.handle(), &target),
        ColdStartLink::None | ColdStartLink::Open(_) => {}
    }
    Ok(())
}

/// The notice for a page the Play copy refused: English source plus its `es` translation. It names
/// no other place to go on purpose — sending the person to finish on the website is the very
/// communication Google Play forbids.
const REFUSAL_NOTICE_EN: &str = "This page is not available in the app.";
const REFUSAL_NOTICE_ES: &str = "Esta página no está disponible en la aplicación.";

/// The script that shows the refusal notice on the page the window stayed on.
///
/// The refused page is the SaaS's, not ours, so there is no i18n runtime to lean on: the language is
/// picked the way the bundled offline page picks it — Spanish unless the device says otherwise. An
/// `alert` because it is the one dialog every webview already paints natively (wry's Android client
/// answers it with an `AlertDialog`). The texts reach the page as JSON string literals, never as
/// code.
fn refusal_notice_script() -> String {
    let es = serde_json::Value::from(REFUSAL_NOTICE_ES);
    let en = serde_json::Value::from(REFUSAL_NOTICE_EN);
    format!(
        "alert(String(navigator.language || \"es\").toLowerCase().indexOf(\"es\") === 0 ? {es} : {en});"
    )
}

/// What the window does with a page `on_navigation` refused: the SaaS home page takes it to the
/// app's own start; any other page leaves it where it was, with the notice.
fn answer_refusal<R: tauri::Runtime>(
    window: &tauri::WebviewWindow<R>,
    verdict: NavigationVerdict,
    saas_base: &str,
) -> tauri::Result<()> {
    match verdict {
        NavigationVerdict::Allow => Ok(()),
        NavigationVerdict::Home => {
            let start = onboarding_url(saas_base)
                .parse::<tauri::Url>()
                .map_err(tauri::Error::InvalidUrl)?;
            window.navigate(start)
        }
        NavigationVerdict::Refuse => window.eval(refusal_notice_script()),
    }
}

/// Records a refused page and answers it on the main window.
///
/// The answer is sent OFF the handler: on Android `on_navigation` runs inside the webview client's
/// `shouldOverrideUrlLoading`, on the UI thread and under wry's own lock, and it has to return
/// before the webview can do anything else — including what we are about to ask of it.
fn refuse_navigation(
    app: &tauri::AppHandle,
    verdict: NavigationVerdict,
    saas_base: &str,
    target: &tauri::Url,
) {
    use tauri::Manager;

    // Origin and path, never the query: it can carry a one-time code.
    log::warn!(
        "shell: the {} copy refused {}{} ({verdict:?})",
        distribution_channel(),
        target.origin().ascii_serialization(),
        target.path()
    );
    let app = app.clone();
    let saas_base = saas_base.to_string();
    tauri::async_runtime::spawn(async move {
        let Some(window) = app.get_webview_window("main") else {
            log::error!("shell: no main window to answer a refused page on");
            return;
        };
        if let Err(e) = answer_refusal(&window, verdict, &saas_base) {
            log::error!("shell: could not answer a refused page ({verdict:?}): {e}");
        }
    });
}

// ── Hardware path: `invoke` handlers → erplora-peripherals ───────────────────────────────────────
//
// The shell IS the bridge (there is no separate bridge process): the hardware is exposed through
// `invoke` handlers that delegate to `erplora-peripherals`. The data contract matches the SDK:
// `getDevices` returns the array of `protocol::Device` and `discoverPrinters` the
// `PrinterDiscovery` outcome (serialized by serde exactly like the SDK's
// `BridgeDevice`/`PrinterDiscovery`); `print`/`testPrint`/`openDrawer` return nothing.
//
// The registry's `Watchdog` runs as an async task of the shell (it recovers a device whose IP DHCP
// changed), and the `PrintQueue` drains with retries in the background. Job outcomes and watchdog
// events are only logged; nothing forwards them to the UI.

use erplora_peripherals::discovery::{self, LocalNetworkAccess, PrinterDiscovery};
use erplora_peripherals::drawer;
use erplora_peripherals::escpos::{self, DocumentType};
use erplora_peripherals::protocol::Device;
use erplora_peripherals::queue::{JobOutcome, PrintJob, PrintQueue, RetryPolicy};
use erplora_peripherals::registry::DeviceRegistry;

/// Hardware state shared by the `invoke` handlers: the persistent device registry plus the print
/// queue with retries. It lives in Tauri's managed state (`app.manage`). Registry and queue sit
/// behind `Arc` so that the background tasks (watchdog and queue worker) share the same instances
/// as the `invoke` handlers.
struct PeripheralsState {
    registry: std::sync::Arc<DeviceRegistry>,
    queue: std::sync::Arc<PrintQueue>,
}

/// Builds the hardware state and **spawns** the background tasks on the current tokio runtime:
///   - the `PrintQueue` worker (sending with retries; every `JobOutcome` is logged),
///   - the registry's `Watchdog` (health check plus recovery by MAC when DHCP changes an IP).
fn build_peripherals_state(devices_path: PathBuf) -> PeripheralsState {
    let registry = std::sync::Arc::new(DeviceRegistry::load(devices_path));
    let queue = std::sync::Arc::new(PrintQueue::new(RetryPolicy::default()));

    // Print queue worker: drains and retries, logging every outcome. `async_runtime::spawn` uses
    // Tauri's global tokio runtime, so it works from the `setup` hook.
    let worker_queue = queue.clone();
    tauri::async_runtime::spawn(async move {
        let (outcomes_tx, mut outcomes_rx) = tokio::sync::mpsc::unbounded_channel::<JobOutcome>();
        tauri::async_runtime::spawn(async move {
            while let Some(outcome) = outcomes_rx.recv().await {
                match outcome {
                    JobOutcome::Completed { job_id } => {
                        eprintln!("peripherals: print job completed ({job_id:?})")
                    }
                    JobOutcome::Failed { job_id, error } => {
                        eprintln!("peripherals: print job failed ({job_id:?}): {error}")
                    }
                }
            }
        });
        worker_queue.run(outcomes_tx).await;
        eprintln!("peripherals: print queue worker stopped (queue closed)");
    });

    // Registry watchdog: recovers devices after DHCP changes their IP.
    let watchdog_registry = registry.clone();
    tauri::async_runtime::spawn(async move {
        use erplora_peripherals::registry::{Watchdog, WatchdogConfig, WatchdogEvent};
        let (events_tx, mut events_rx) = tokio::sync::mpsc::unbounded_channel::<WatchdogEvent>();
        tauri::async_runtime::spawn(async move {
            while let Some(ev) = events_rx.recv().await {
                match ev {
                    // `key` rather than `mac`: it is the device's stable identity and always exists
                    // (the MAC is `None` when ARP does not resolve, which is always on Android).
                    WatchdogEvent::Recovered(d) => {
                        eprintln!("peripherals: device recovered {} ({})", d.name, d.key)
                    }
                    WatchdogEvent::Lost(d) => {
                        eprintln!("peripherals: device lost {} ({})", d.name, d.key)
                    }
                }
            }
        });
        let watchdog = Watchdog::new(WatchdogConfig::default()).with_events(events_tx);
        watchdog.run(&watchdog_registry).await;
    });

    PeripheralsState { registry, queue }
}

/// Error of the hardware handlers. It is serialized as a string for the frontend, where the SDK
/// turns it into a rejected promise.
#[derive(Debug, thiserror::Error)]
pub enum HardwareError {
    #[error("{0}")]
    Peripheral(#[from] erplora_peripherals::PeripheralError),
}

impl Serialize for HardwareError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

/// Refusal of `erplora_add_network_printer`: the stable `code` the page branches on next to the
/// message for the log. `HardwareError` travels as a bare string, and the two answers this command
/// can give send the owner to opposite places — fix the typed address, or go check the printer —
/// so the page must be able to tell them apart without parsing prose.
#[derive(Debug, Serialize)]
struct AddPrinterError {
    code: &'static str,
    message: String,
}

impl From<erplora_peripherals::PeripheralError> for AddPrinterError {
    fn from(e: erplora_peripherals::PeripheralError) -> Self {
        Self {
            code: e.code(),
            message: e.to_string(),
        }
    }
}

/// `erplora_bridge_status` — `IpcBridgeTransport.detect()` calls it to learn whether the hardware
/// channel exists (in the shell it always does: the shell IS the bridge). Returns the version.
#[tauri::command]
fn erplora_bridge_status() -> serde_json::Value {
    serde_json::json!({ "version": bridge_status_version() })
}

/// The version the app declares to the rest of the system (the printing screen shows it).
///
/// It comes from `tauri.conf.json`, which is what **stamps the release** (`tauri-release.yml`
/// writes the tag there before building). Nobody stamps `CARGO_PKG_VERSION`: this shell's
/// `Cargo.toml` says `0.0.0` in every build, so an installed 1.0.0 app would introduce itself as
/// «Impresora lista · v0.0.0.».
///
/// The conf is read with `include_str!` rather than `AppHandle::package_info()` so that this is a
/// pure function a unit test can check — it is the same file `generate_context!` reads.
fn bridge_status_version() -> String {
    tauri_conf_version(include_str!("../tauri.conf.json"))
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
}

/// The `version` field of a `tauri.conf.json`, or `None` when there is no readable one.
fn tauri_conf_version(conf: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(conf)
        .ok()?
        .get("version")?
        .as_str()
        .map(str::to_string)
}

/// Reads the Android runtime-permission map and says whether the printer sweep may run.
///
/// Pure on purpose: an emulator is the only other way to reach this branch, and a state that
/// decides between two opposite messages to the user has to be provable on a laptop.
///
/// A permission the map does not mention is one this platform does not gate — desktop answers with
/// an empty map, and so does an Android older than the release that introduced it. Reading that
/// silence as "denied" would ground a till that can see its printer perfectly well.
fn local_network_access(status: &std::collections::HashMap<String, bool>) -> LocalNetworkAccess {
    match status.get(tauri_plugin_erplora_android::ACCESS_LOCAL_NETWORK) {
        Some(false) => LocalNetworkAccess::Denied {
            permission: tauri_plugin_erplora_android::ACCESS_LOCAL_NETWORK.to_string(),
        },
        _ => LocalNetworkAccess::Granted,
    }
}

/// What the OS allows right now — asked without prompting: the PWA already requested the
/// permission before invoking this, so here we only read the answer it gave.
fn shell_local_network_access<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> LocalNetworkAccess {
    use tauri_plugin_erplora_android::ErploraAndroidExt;

    match app.erplora_android().check_permissions() {
        Ok(status) => local_network_access(&status),
        // The CHECK failing is not a denial — on desktop there is no Android activity to ask. If
        // we treated it as one, the shell would start claiming "no permission" on the platform
        // that has no such permission, which is the same lie pointing the other way.
        Err(e) => {
            log::warn!("shell: could not read the local network permission ({e}); scanning anyway");
            LocalNetworkAccess::Granted
        }
    }
}

/// Converts a bonded printer (as Kotlin announces it) into the shared `PrinterInfo` shape.
///
/// `category` stays `unknown` on purpose — same honesty rule as the port-9100 sweep: SPP says
/// "accepts bytes", not "speaks ESC/POS", and guessing "thermal" is how wrong paper gets printed.
fn bluetooth_printer_info(
    p: &tauri_plugin_erplora_android::BluetoothPrinter,
) -> erplora_peripherals::protocol::PrinterInfo {
    erplora_peripherals::protocol::PrinterInfo {
        id: p.id.clone(),
        name: p.name.clone(),
        kind: "bluetooth".into(),
        category: erplora_peripherals::protocol::default_printer_category(),
        status: "ready".into(),
        paper_width: 80,
        mac: Some(p.mac.clone()),
    }
}

/// Folds the bonded Bluetooth printers into the network discovery outcome.
///
/// One list on purpose: "which printers can this device print on?" is ONE question to the person
/// setting up a till, whatever transport each answer arrives by. Deduplicated by id, so a
/// re-merge cannot turn one printer into two devices.
///
/// A `PermissionDenied` outcome stays a refusal WITHOUT printers: an answer that both names a
/// missing permission and lists printers would give the screen two contradictory instructions at
/// once. Discovery asks for both printer permissions up front, so that state is one
/// "allow" away from resolving itself.
fn merge_bluetooth_printers(
    outcome: PrinterDiscovery,
    bonded: Vec<tauri_plugin_erplora_android::BluetoothPrinter>,
) -> PrinterDiscovery {
    match outcome {
        PrinterDiscovery::Scanned { mut printers } => {
            for p in &bonded {
                if printers.iter().any(|known| known.id == p.id) {
                    continue;
                }
                printers.push(bluetooth_printer_info(p));
            }
            PrinterDiscovery::Scanned { printers }
        }
        blocked @ PrinterDiscovery::PermissionDenied { .. } => blocked,
    }
}

/// Folds the machine's USB print queues into the discovery outcome.
///
/// Same rule as the Bluetooth merge, and for the same reason: "which printers can this device
/// print on?" is ONE question to the person setting up a till, whatever cable each answer arrives
/// by. Deduplicated by id, so a re-scan cannot turn one printer into two.
///
/// A `PermissionDenied` outcome stays a refusal without printers. On desktop — the only place a
/// USB queue exists — that branch is unreachable (there is no local-network gate to deny), but the
/// shell having ONE merge rule beats it having two that differ in a case nobody can hit.
fn merge_usb_printers(
    outcome: PrinterDiscovery,
    usb: Vec<erplora_peripherals::protocol::PrinterInfo>,
) -> PrinterDiscovery {
    match outcome {
        PrinterDiscovery::Scanned { mut printers } => {
            for queue in usb {
                if printers.iter().any(|known| known.id == queue.id) {
                    continue;
                }
                printers.push(queue);
            }
            PrinterDiscovery::Scanned { printers }
        }
        blocked @ PrinterDiscovery::PermissionDenied { .. } => blocked,
    }
}

/// Puts the machine's own print queues into the device registry, so the owner can say which one
/// prints the kitchen's tickets.
///
/// The network and Bluetooth halves of discovery already register what they find; this is the
/// third. A queue has neither MAC nor socket, so it enters by its `printer_id` — the door
/// `DeviceRegistry::register_queue` exists for.
///
/// A refusal is **logged and skipped**, never propagated: a discovery is a batch, and one queue
/// with an id CUPS could not have produced must not cost the till the printer it does have. Logged
/// because a queue that silently never accepts a role is exactly the mute failure that sends a
/// user to press a button that does nothing.
fn register_discovered_queues(
    registry: &DeviceRegistry,
    queues: &[erplora_peripherals::protocol::PrinterInfo],
) {
    for queue in queues {
        if let Err(e) = registry.register_queue(&queue.id, &queue.name) {
            log::warn!(
                "shell: the print queue `{}` did not enter the device registry ({e}); it prints, \
                 but it will not accept a role",
                queue.id
            );
        }
    }
}

/// `erplora_discover_printers` — rescans the network (mDNS + subnet), lists the PAIRED Bluetooth
/// printers (Android only), registers and returns the printers. Mirrors the bridge's
/// `Command::DiscoverPrinters`.
///
/// Returns the OUTCOME (`{status, …}`), not a bare array: without the local network permission
/// there is no empty list to return, there is a reason — and «connect a printer» and «give the app
/// permission» are opposite instructions for the user.
#[tauri::command]
async fn erplora_discover_printers(
    app: tauri::AppHandle,
    state: tauri::State<'_, PeripheralsState>,
) -> Result<PrinterDiscovery, HardwareError> {
    use tauri_plugin_erplora_android::ErploraAndroidExt;

    let access = shell_local_network_access(&app);
    let outcome = discovery::discover_printers(&state.registry, access).await?;

    // Bonded SPP printers (Android; empty everywhere else). A bluetooth failure must not poison
    // the network half — a venue with a broken adapter still has its LAN printers.
    let bonded = app
        .erplora_android()
        .bluetooth_bonded_printers()
        .unwrap_or_else(|e| {
            log::warn!("shell: could not list bonded bluetooth printers ({e})");
            Vec::new()
        });
    // Registered like their network siblings, so roles (kitchen/bar/receipt) can be assigned to
    // them; the MAC is the key, ip/port stay empty — the watchdog knows to leave them alone.
    for p in &bonded {
        let _ = state.registry.register(Some(&p.mac), "", 0, &p.name, "bluetooth");
    }

    // The machine's own USB print queues (desktop only — Android has no queue to ask).
    // `spawn_blocking` because asking CUPS spawns a client and waits for it: on a wedged `cupsd`
    // that wait runs to its timeout, and doing it on the async runtime would stall every other
    // command for as long as it lasted.
    //
    // A CUPS failure must not poison the network half: a venue whose LAN printers work fine has to
    // keep seeing them when the CUPS client tools are missing. Logged, so it is not a silent zero.
    #[cfg(not(target_os = "android"))]
    let usb = tokio::task::spawn_blocking(|| {
        erplora_peripherals::usb::discover_usb_printers(&erplora_peripherals::usb::SystemCups)
    })
    .await
    .unwrap_or_else(|e| {
        Err(erplora_peripherals::PeripheralError::Unreachable(format!(
            "the USB queue listing did not finish: {e}"
        )))
    })
    .unwrap_or_else(|e| {
        log::warn!("shell: could not list the OS print queues ({e}); USB printers will not appear");
        Vec::new()
    });
    #[cfg(target_os = "android")]
    let usb = Vec::new();

    // Registered like their network and bluetooth siblings, so roles (kitchen/bar/receipt) can be
    // assigned to them; the printer_id is the key, ip/port stay empty — the watchdog only monitors
    // `network`, so a queue never enters the health probe or the ARP recovery sweep.
    register_discovered_queues(&state.registry, &usb);

    // Printers the owner typed by address stay listed even when the sweep cannot see
    // them — that is why they were typed.
    let outcome = discovery::with_manual_printers(outcome, &state.registry);

    Ok(merge_usb_printers(
        merge_bluetooth_printers(outcome, bonded),
        usb,
    ))
}

/// `erplora_add_network_printer` — adds a network printer by the address the owner TYPED: the way
/// in when the scan cannot see it (another subnet, an isolated Wi-Fi, mDNS blocked by the router).
/// It connects first and only a printer that answers is saved; the next discovery keeps listing
/// it. Returns the printer as the scan would have.
#[tauri::command]
async fn erplora_add_network_printer(
    state: tauri::State<'_, PeripheralsState>,
    host: String,
    port: u16,
) -> Result<erplora_peripherals::protocol::PrinterInfo, AddPrinterError> {
    Ok(discovery::add_network_printer(
        &state.registry,
        &host,
        port,
        discovery::MANUAL_PRINTER_PROBE_TIMEOUT,
    )
    .await?)
}

/// `erplora_get_devices` — the contents of the persistent device registry (with their roles).
#[tauri::command]
fn erplora_get_devices(state: tauri::State<'_, PeripheralsState>) -> Vec<Device> {
    state.registry.get_all()
}

/// Sends already-rendered bytes to a bonded SPP printer through the Android plugin.
///
/// Direct, not queued: the `PrintQueue` is the NETWORK path (its jobs carry a socket target).
/// Bluetooth gets the simple failure mode on purpose — the error comes straight back to the caller
/// (the print host reports `failed`), instead of retrying against a printer whose only health
/// signal IS the connect.
fn bluetooth_send(
    app: &tauri::AppHandle,
    mac: &str,
    payload: &[u8],
) -> Result<(), HardwareError> {
    use tauri_plugin_erplora_android::ErploraAndroidExt;
    app.erplora_android()
        .bluetooth_print(mac, payload)
        .map_err(|e| {
            HardwareError::from(erplora_peripherals::PeripheralError::Unreachable(
                e.to_string(),
            ))
        })
}

/// Sends already-rendered bytes to the printer behind an OS print queue.
///
/// Direct, not queued — the same shape as `bluetooth_send`, and the reasoning is in
/// `erplora_peripherals::usb::send_raw`: [`PrintQueue`] is the NETWORK path, its jobs carry a
/// socket target, and widening it is a real change to the one piece of the print chain that
/// already works. The failure stays visible: the error comes back to the caller and the print host
/// reports the job `failed`.
#[cfg(not(target_os = "android"))]
fn usb_send(target: &discovery::UsbTarget, payload: &[u8]) -> Result<(), HardwareError> {
    erplora_peripherals::usb::send_raw(&erplora_peripherals::usb::SystemCups, target, payload)?;
    Ok(())
}

/// Android has no OS print queue to hand bytes to, and USB Host there is left out on purpose: the
/// Bluetooth SPP transport already covers the cheap printer on a tablet.
///
/// This says so instead of quietly doing nothing. A hub configured on the desktop till and then
/// opened on a tablet keeps the same `printer_id`, so this IS reachable — and a ticket that
/// vanishes without a word is exactly the failure mode the print chain keeps being bitten by.
#[cfg(target_os = "android")]
fn usb_send(target: &discovery::UsbTarget, _payload: &[u8]) -> Result<(), HardwareError> {
    Err(HardwareError::from(
        erplora_peripherals::PeripheralError::Unreachable(format!(
            "`usb:{}` is a printer on a desktop till's print queue; this device cannot reach it \
             (use a network or a bonded Bluetooth printer here)",
            target.queue
        )),
    ))
}

/// Sends a document to a network printer and answers with what happened to the paper: `Ok` once
/// it is on the printer, the unreachable error once the queue's retries ran out. Answering on the
/// hand-over to the in-memory queue instead would let a switched-off printer lose the ticket with
/// nobody told — the till would hear «done», and so would the hub for a job from its queue.
async fn print_to_network(
    queue: &PrintQueue,
    target: discovery::NetworkTarget,
    payload: Vec<u8>,
    job_id: Option<String>,
) -> Result<(), HardwareError> {
    queue.print(PrintJob { job_id, target, payload, attempts: 0 }).await?;
    Ok(())
}

/// `erplora_print` — renders the ESC/POS document and sends it (same command as the bridge's
/// `Command::Print`): through the [`PrintQueue`] and its retries for a network printer, directly
/// for a Bluetooth or USB one. Every failure, a bad printer_id or payload included, comes back to
/// the caller.
///
/// **An unknown `document_type` is refused, not printed as `Generic`**. This is the end of the
/// chain: the queue in the hub already checks the vocabulary, but a document that got past it —
/// a job written by hand, a producer talking straight to this command — would otherwise reach the
/// paper as a nameless key/value dump, and a kitchen order that comes out wrong is only discovered
/// when the plate is missing. Failing here turns that into a `failed` the print host reports.
///
/// ⚠️ `async` is load-bearing: the bluetooth arm crosses into Kotlin through
/// `run_mobile_plugin`, which dispatches onto Android's main looper and BLOCKS for the answer — a
/// plain command runs on that very thread and the till would hang on the press that prints. And the
/// network arm awaits the paper, retries included, which only an async command can do.
#[tauri::command]
async fn erplora_print(
    app: tauri::AppHandle,
    state: tauri::State<'_, PeripheralsState>,
    printer_id: String,
    document_type: String,
    data: serde_json::Value,
    job_id: Option<String>,
) -> Result<(), HardwareError> {
    let target = discovery::parse_print_target(&printer_id)?;
    let doc = DocumentType::parse(&document_type).ok_or_else(|| {
        HardwareError::from(erplora_peripherals::PeripheralError::UnknownDocumentType(
            document_type.clone(),
        ))
    })?;
    let payload = escpos::render_document(doc, &data)?;
    match target {
        discovery::PrintTarget::Network(target) => {
            print_to_network(&state.queue, target, payload, job_id).await?;
        }
        discovery::PrintTarget::Bluetooth(bt) => bluetooth_send(&app, &bt.mac, &payload)?,
        discovery::PrintTarget::Usb(usb) => usb_send(&usb, &payload)?,
    }
    Ok(())
}

/// `erplora_test_print` — prints a test page on the given printer (over SPP when it is Bluetooth).
/// `async` for the same reasons as `erplora_print`: «Probar» on a switched-off network printer
/// says it did not print.
#[tauri::command]
async fn erplora_test_print(
    app: tauri::AppHandle,
    state: tauri::State<'_, PeripheralsState>,
    printer_id: String,
    data: Option<serde_json::Value>,
) -> Result<(), HardwareError> {
    let target = discovery::parse_print_target(&printer_id)?;
    // An `erplora-app` newer than the module that calls it gets `None` here — and the renderer
    // reads an empty document exactly as it reads a missing field, so the sheet still prints.
    let data = data.unwrap_or_else(|| serde_json::json!({}));
    let payload = escpos::render_test_page(&printer_id, &data);
    match target {
        discovery::PrintTarget::Network(target) => {
            print_to_network(&state.queue, target, payload, None).await?;
        }
        discovery::PrintTarget::Bluetooth(bt) => bluetooth_send(&app, &bt.mac, &payload)?,
        discovery::PrintTarget::Usb(usb) => usb_send(&usb, &payload)?,
    }
    Ok(())
}

/// `erplora_open_drawer` — opens the cash drawer with an ESC/POS kick over the printer's socket (or
/// over the SPP transport when the printer is Bluetooth: the kick is ESC/POS bytes like any other
/// document).
#[tauri::command]
async fn erplora_open_drawer(
    app: tauri::AppHandle,
    printer_id: String,
    pin: Option<u8>,
) -> Result<(), HardwareError> {
    match discovery::parse_print_target(&printer_id)? {
        discovery::PrintTarget::Network(target) => {
            drawer::open_drawer(&target, pin.unwrap_or(2)).await?;
        }
        discovery::PrintTarget::Bluetooth(bt) => {
            bluetooth_send(&app, &bt.mac, drawer::kick_command(pin.unwrap_or(2)))?;
        }
        // The kick is ESC/POS like any other document, so it rides the same raw queue.
        discovery::PrintTarget::Usb(usb) => {
            usb_send(&usb, drawer::kick_command(pin.unwrap_or(2)))?;
        }
    }
    Ok(())
}

// ── «Start on login» ─────────────────────────────────────────────────────────────────────────────
//
// The print queue lives in the hub and this device drains it: if nobody opened the
// app, the tickets wait. On a dedicated desktop till, starting with the session guarantees there
// is always a print host. Opt-in, OFF by default — and the OS owns the state (LaunchAgent /
// registry / autostart dir), we keep NO persistence of our own. Desktop only: the plugin does not
// support Android/iOS, and a tablet has its own idea of "login".

/// The OS-level switch behind the setting, as a seam: the real one writes a LaunchAgent — a unit
/// test that flipped it would leave the developer's machine starting a till at login.
trait AutostartSwitch {
    fn is_enabled(&self) -> Result<bool, String>;
    fn set_enabled(&self, enabled: bool) -> Result<(), String>;
}

/// Applies the desired state and answers with what the OS says NOW — read back, never assumed. A
/// toggle that showed ON while the OS said OFF would be a till that never starts, discovered the
/// morning the first unprinted ticket is.
fn apply_autostart(switch: &dyn AutostartSwitch, enabled: bool) -> Result<bool, String> {
    switch.set_enabled(enabled)?;
    switch.is_enabled()
}

/// The real switch: `tauri-plugin-autostart`, which talks to the LaunchAgent (macOS), the Run
/// registry key (Windows) or the autostart dir (Linux).
#[cfg(desktop)]
struct OsAutostart<'a>(&'a tauri::AppHandle);

#[cfg(desktop)]
impl AutostartSwitch for OsAutostart<'_> {
    fn is_enabled(&self) -> Result<bool, String> {
        use tauri_plugin_autostart::ManagerExt;
        self.0.autolaunch().is_enabled().map_err(|e| e.to_string())
    }
    fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        use tauri_plugin_autostart::ManagerExt;
        let launcher = self.0.autolaunch();
        if enabled { launcher.enable() } else { launcher.disable() }.map_err(|e| e.to_string())
    }
}

/// `autostart_is_enabled` — the CURRENT state, straight from the OS. On mobile the command
/// answers an error on purpose: the setting must not render there at all, and a silent `false`
/// would let it.
#[tauri::command]
fn autostart_is_enabled(app: tauri::AppHandle) -> Result<bool, ShellError> {
    #[cfg(desktop)]
    {
        return OsAutostart(&app).is_enabled().map_err(ShellError::Io);
    }
    #[cfg(not(desktop))]
    {
        let _ = app;
        Err(ShellError::Io("autostart is desktop-only".into()))
    }
}

/// `autostart_enable` — turns «Start on login» on and answers with the state read back.
#[tauri::command]
fn autostart_enable(app: tauri::AppHandle) -> Result<bool, ShellError> {
    #[cfg(desktop)]
    {
        return apply_autostart(&OsAutostart(&app), true).map_err(ShellError::Io);
    }
    #[cfg(not(desktop))]
    {
        let _ = app;
        Err(ShellError::Io("autostart is desktop-only".into()))
    }
}

/// `autostart_disable` — turns it off; answers with the state read back.
#[tauri::command]
fn autostart_disable(app: tauri::AppHandle) -> Result<bool, ShellError> {
    #[cfg(desktop)]
    {
        return apply_autostart(&OsAutostart(&app), false).map_err(ShellError::Io);
    }
    #[cfg(not(desktop))]
    {
        let _ = app;
        Err(ShellError::Io("autostart is desktop-only".into()))
    }
}

/// `erplora_set_device_role` — assigns a role (receipt/kitchen/bar/label) and returns the updated
/// registry. Mirrors the bridge's `Command::SetDeviceRole`.
#[tauri::command]
fn erplora_set_device_role(
    state: tauri::State<'_, PeripheralsState>,
    mac: String,
    role: String,
) -> Result<Vec<Device>, HardwareError> {
    state.registry.set_role(&mac, &role)?;
    Ok(state.registry.get_all())
}

/// `erplora_set_device_name` — renames a device and returns the updated registry.
#[tauri::command]
fn erplora_set_device_name(
    state: tauri::State<'_, PeripheralsState>,
    mac: String,
    name: String,
) -> Result<Vec<Device>, HardwareError> {
    state.registry.set_name(&mac, &name)?;
    Ok(state.registry.get_all())
}

/// `erplora_remove_device` — removes a device from the registry and returns the updated registry.
#[tauri::command]
fn erplora_remove_device(
    state: tauri::State<'_, PeripheralsState>,
    mac: String,
) -> Result<Vec<Device>, HardwareError> {
    state.registry.remove(&mac)?;
    Ok(state.registry.get_all())
}

// ── The badge, read off the device's own NFC reader ─────────────────────────────────────────────
//
// The counter reads badges through a USB reader that behaves as a keyboard, and the shell catches
// the burst by its speed (`badge-scanner.ts`). A tablet has no USB reader — and has had a
// reader inside it all along, unused. This is the second origin of the SAME badge path: what comes
// back is the same kind of string the wedge types, and the web hands it to the very subscribers the
// wedge feeds. No screen above learns where a card came from.

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
fn erplora_nfc_read(
    app: tauri::AppHandle,
    timeout_ms: Option<u64>,
) -> Result<NfcReadOutcome, ShellError> {
    use tauri_plugin_erplora_android::ErploraAndroidExt as _;
    app.erplora_android()
        .nfc_read(nfc_read_timeout(timeout_ms))
        .map(|badge| NfcReadOutcome { badge })
        .map_err(nfc_shell_error)
}

/// `erplora_notify` — a SYSTEM notice (the OS one, not a toast inside the app).
///
/// That is what it is for: warning when **nobody is looking at the screen**. The case behind it is
/// the kitchen order — an order comes in and the kitchen has to find out even with the tablet on
/// another view or locked. A toast inside the app does not reach anyone there.
///
/// The SHELL exposes it because in Tauri the shell **is** the hardware bridge.
///
/// `id` is the handle of the tap: the notification plugin reports a tap with the id of
/// the notice on Android and iOS alike, and the page remembers which screen that id leads to. A
/// page older than this shell sends none and the notice goes out under the plugin's own id.
///
/// `path` is the screen itself, for the taps that do not reach the page that sent the notice: on
/// the computer the shell shows the notice itself and keeps the click. On Android it travels in
/// `extra`, but the plugin never stores the notice on the intent of its tap, which comes back with
/// the id only: Kotlin writes the screen down under the id before the notice
/// goes out, and hands it back with the tap.
///
/// **Never fails upwards.** If the user denied the permission or the platform cannot show it, it is
/// logged and life goes on: a notice that does not go out cannot bring down the order behind it.
#[tauri::command]
fn erplora_notify(app: tauri::AppHandle, title: String, body: String, id: Option<i64>, path: Option<String>) {
    // The notification plugin shows a desktop notice and drops its handle: no click would come back.
    #[cfg(desktop)]
    // The thread is left to run on its own: it lasts as long as the notice does.
    let _ = notify_on_desktop(app, title, body, id, path, notice_tap::deliver);
    #[cfg(mobile)]
    {
        use tauri_plugin_erplora_android::ErploraAndroidExt as _;
        // Before the notice is out: a tap right after it must find its screen.
        remember_notice_target(id, path.as_deref(), |id, path| app.erplora_android().remember_notice(id, path));
        if let Err(e) = notice_builder(&app, &title, &body, id, path.as_deref()).show() {
            eprintln!("notify: the platform could not show «{title}» ({e}) — carrying on");
        }
    }
}

/// The notice `erplora_notify` shows, under the page's id when the plugin can hold it and with the
/// screen it leads to when it names one.
#[cfg_attr(desktop, allow(dead_code))]
fn notice_builder<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    title: &str,
    body: &str,
    id: Option<i64>,
    path: Option<&str>,
) -> tauri_plugin_notification::NotificationBuilder<R> {
    use tauri_plugin_notification::NotificationExt;

    let mut builder = app.notification().builder().title(title).body(body);
    if let Some(id) = notice_id(id) {
        builder = builder.id(id);
    }
    if let Some(path) = path {
        builder = builder.extra("path", path);
    }
    builder
}

/// `erplora_take_notice_tap` — the tap the page was not there to hear, handed over once:
/// a click on the computer, or the tap that started the app on Android. `null` when there is none.
#[tauri::command]
fn erplora_take_notice_tap(app: tauri::AppHandle) -> Option<serde_json::Value> {
    take_notice_tap(&app)
}

/// `erplora_notify` on the computer, with the platform's delivery handed in: the notice carries
/// back its id, when it has one the platform can hold, and its screen.
#[cfg(desktop)]
fn notify_on_desktop<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    title: String,
    body: String,
    id: Option<i64>,
    path: Option<String>,
    deliver: notice_tap::Deliver<R>,
) -> Option<std::thread::JoinHandle<()>> {
    notice_tap::show(app, title, body, notice_id(id).map(|id| notice_tap::NoticeTap { id, path }), deliver)
}

/// Kotlin's answer to «which tap started the app», in the shape the page reads every tap in.
fn launch_tap_payload<E: std::fmt::Display>(
    answer: Result<Option<tauri_plugin_erplora_android::LaunchNoticeTap>, E>,
) -> Option<serde_json::Value> {
    match answer {
        Ok(tap) => tap.map(|tap| notice_tap::NoticeTap::from_launch(tap.id, tap.notification.as_deref(), tap.path.as_deref()).payload()),
        Err(e) => {
            log::warn!("notice: the tap that started the app could not be read ({e})");
            None
        }
    }
}

fn take_notice_tap<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Option<serde_json::Value> {
    use tauri::Manager;
    use tauri_plugin_erplora_android::ErploraAndroidExt as _;

    if let Some(tap) = app.state::<notice_tap::KeptNoticeTap>().take() {
        return Some(tap.payload());
    }
    launch_tap_payload(app.erplora_android().take_notice_tap())
}

/// Writes down the screen a notice leads to under the number it goes out with, through
/// `remember`. A notice under no number the plugin can hold goes out under the plugin's own, which
/// nothing was written under. Never fails upwards: a screen that is not written down only means
/// that the tap opens the app without it.
#[cfg_attr(desktop, allow(dead_code))]
fn remember_notice_target<E: std::fmt::Display>(
    id: Option<i64>,
    path: Option<&str>,
    remember: impl FnOnce(i32, Option<&str>) -> Result<(), E>,
) {
    let Some(id) = notice_id(id) else { return };
    if let Err(e) = remember(id, path) {
        log::warn!("notice: the screen of notice {id} could not be written down ({e})");
    }
}

/// The id the plugin can hold (`i32`), or none: an id out of range must not stop the notice, only
/// lose its tap's destination.
fn notice_id(id: Option<i64>) -> Option<i32> {
    id.and_then(|id| i32::try_from(id).ok())
}

/// Another launch of the app, handed over by single-instance: a click on a notice, or a
/// link to a hub.
#[cfg(desktop)]
fn on_second_launch<R: tauri::Runtime>(app: &tauri::AppHandle<R>, argv: Vec<String>) {
    if notice_tap::answer_link(app, argv.iter().cloned()) {
        return;
    }
    if let Some(target) = deep_link_from_args(argv) {
        open_hub_from_link(app, &target);
    }
}

/// Every command the app answers. `run` hands it to Tauri, and the tests drive the same
/// dispatcher through the IPC.
fn app_commands() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        device_context,
        forget_hub,
        open_external_url,
        save_download,
        // The system print dialog for an A4 document.
        print_document,
        // The way out when the network dies under the window.
        shell_retry,
        // Data does NOT go through `invoke`: the PWA talks HTTP and WebSocket to its hub.
        // Hardware path: ESC/POS network printers and the cash drawer → peripherals.
        erplora_bridge_status,
        erplora_discover_printers,
        erplora_get_devices,
        erplora_print,
        erplora_test_print,
        erplora_open_drawer,
        erplora_set_device_role,
        erplora_add_network_printer,
        erplora_set_device_name,
        erplora_remove_device,
        erplora_notify,
        // The tap the page was not there to hear.
        erplora_take_notice_tap,
        // The badge over NFC: the second way through the SAME door as the keyboard-wedge reader.
        erplora_nfc_read,
        // «Start on login»: desktop-only in effect — on mobile they answer an
        // error, and the settings toggle never renders there.
        autostart_is_enabled,
        autostart_enable,
        autostart_disable
    ]
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();

    // Desktop: the system answers an `erplora://` link by LAUNCHING the executable again. Without
    // this, pressing a link with the app open would start a SECOND till on top of the first (two
    // windows, two print queues, two watchdogs). With it, the second instance dies at birth and
    // hands its arguments to the one already running, which is the one that navigates.
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| on_second_launch(app, argv)));

    // «Start on login»: `init` WITHOUT calling `enable()` — the plugin
    // registers the commands and nothing else, so a fresh install stays OFF until the user opts
    // in from the settings toggle. Desktop only: the plugin has no Android/iOS support.
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_autostart::init(
        tauri_plugin_autostart::MacosLauncher::LaunchAgent,
        None,
    ));

    builder
        // The A4 document of `print_document`, served from memory to its print window
        // with a CSP that runs no code. Any other path is a 404.
        .register_uri_scheme_protocol(PRINT_SCHEME, |ctx, request| {
            use tauri::Manager;
            ctx.app_handle()
                .state::<PrintDocuments>()
                .respond(request.uri().path())
        })
        .plugin(hub_link::notification_plugin())
        .plugin(hub_link::android_plugin())
        // The user's own browser. Registered for its RUST api only: no `opener:*`
        // permission is granted to any origin (`tests/remote_acl.rs`), so the page cannot reach the
        // plugin's own commands — which take any address, and two of which open FILES. What the
        // page gets is `open_external_url`, which checks the destination first.
        .plugin(tauri_plugin_opener::init())
        // Deep link `erplora://`: registers the scheme per platform and delivers the URL. It goes
        // AFTER single-instance on purpose: that is the order the plugin itself documents.
        .plugin(tauri_plugin_deep_link::init())
        .setup(|app| {
            use tauri::Manager;
            use tauri_plugin_deep_link::DeepLinkExt;

            // A link with the app ALREADY open (macOS, iOS and Android deliver it as an event; on
            // desktop single-instance hands it over). The cold-start link does not pass through
            // here: it arrives in the arguments and `open_main_window` resolves it.
            //
            // A link that does NOT resolve does nothing, on purpose: the fallback belongs to the
            // browser that launched it, and navigating «somewhere» would be worse than staying put.
            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                // Same filter as the cold start: one link per gesture and, should several arrive,
                // the first one that resolves wins.
                let urls: Vec<String> = event.urls().into_iter().map(String::from).collect();
                // A notice link is a click on a notice: single-instance hands it over.
                if notice_tap::click_from_args(urls.iter().cloned()).is_some() {
                    return;
                }
                match deep_link_from_args(urls) {
                    Some(target) => open_hub_from_link(&handle, &target),
                    None => log::warn!("shell: link ignored, it does not lead to a hub of ours"),
                }
            });

            // Windows and Linux learn the scheme at INSTALL time (registry / `.desktop` handler),
            // so in a development run (no installer) it is not registered and the link cannot even
            // be tried. Registering it here only affects that run.
            #[cfg(any(windows, target_os = "linux"))]
            if let Err(e) = app.deep_link().register_all() {
                log::warn!("shell: could not register the {DEEP_LINK_SCHEME}:// scheme: {e}");
            }

            // Per-installation data root: device.id + hub.url + devices.json. If it cannot be
            // resolved the shell starts anyway, without persistence: it is a thin client with no
            // database.
            let cache_dir = match app.path().app_data_dir() {
                Ok(dir) => {
                    if let Err(e) = std::fs::create_dir_all(&dir) {
                        eprintln!("shell: could not create app_data_dir ({}): {e}", dir.display());
                    }
                    Some(dir)
                }
                Err(e) => {
                    eprintln!("shell: app_data_dir unavailable: {e}");
                    None
                }
            };
            // Hardware state: device registry + print queue + watchdog.
            let devices_path = cache_dir
                .as_deref()
                .map(|d| d.join(DEVICES_FILE))
                .unwrap_or_else(|| PathBuf::from(DEVICES_FILE));
            app.manage(build_peripherals_state(devices_path));
            app.manage(PrintDocuments::default());
            // The notice tap the page was not there to hear, until it claims it.
            app.manage(notice_tap::KeptNoticeTap::default());
            // A tap on Android while the page listens: Kotlin keeps it with its screen and the page
            // is told to claim it — the plugin's own report of it says nothing.
            #[cfg(target_os = "android")]
            {
                use tauri_plugin_erplora_android::ErploraAndroidExt as _;
                let handle = app.handle().clone();
                if let Err(e) = app.erplora_android().watch_notice_taps(move || notice_tap::tell_page(&handle)) {
                    log::warn!("notice: the taps on the notices cannot be watched ({e})");
                }
            }
            // Who asks before a link links another hub: before the window, so a cold start by such
            // a link already has somebody to ask.
            app.manage(link_consent::native(app.handle().clone()));
            // Single window: the SaaS onboarding or the captured hub (app mode).
            if let Err(e) = open_main_window(app, cache_dir) {
                eprintln!("could not create the main window: {e}");
            }
            // Started by a click on a notice with the app closed (Windows): the tap waits
            // for the page, which claims it at boot.
            #[cfg(desktop)]
            notice_tap::answer_link(app.handle(), std::env::args());
            Ok(())
        })
        .invoke_handler(hub_link::guard(app_commands()))
        .run(tauri::generate_context!())
        .expect("error while running ERPlora shell");
}

// ── Tests ────────────────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── A ticket that does not come out is an error for whoever printed it ──────────────────────
    //
    // Answering `Ok` as soon as the bytes are in the in-memory queue would leave the till hearing
    // nothing with the printer switched off, and the device draining the hub's queue confirming
    // «printed». The network arm waits for the paper, retries included.

    #[test]
    fn a_switched_off_network_printer_is_an_error_for_whoever_printed() {
        let queue = std::sync::Arc::new(PrintQueue::new(RetryPolicy {
            max_attempts: 1,
            backoff_ms: 0,
            connect_timeout_ms: 500,
            write_timeout_ms: 500,
        }));
        let worker = queue.clone();
        tauri::async_runtime::spawn(async move {
            let (outcomes, _ignored) = tokio::sync::mpsc::unbounded_channel();
            worker.run(outcomes).await;
        });
        // A port nobody listens on any more: a printer that is off.
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a free port");
            listener.local_addr().expect("its address").port()
        };
        let target = discovery::NetworkTarget { host: "127.0.0.1".into(), port };

        let printed = tauri::async_runtime::block_on(print_to_network(
            &queue,
            target,
            b"ticket".to_vec(),
            Some("sale-1".into()),
        ));

        let err = printed.expect_err("a ticket that never reached the printer is not printed");
        assert!(
            matches!(err, HardwareError::Peripheral(erplora_peripherals::PeripheralError::Unreachable(_))),
            "got {err:?}"
        );
    }

    #[test]
    fn every_network_print_waits_for_the_paper() {
        // `erplora_print` and `erplora_test_print` («Probar») both go through `print_to_network`;
        // a hand-over that answers on the enqueue would report a print before the paper is out.
        let source = include_str!("lib.rs");
        let shell = source.split("\n#[cfg(test)]\nmod tests").next().unwrap_or_default();
        assert!(!shell.contains("queue.enqueue("), "a network print answers on the hand-over again");
        assert_eq!(shell.matches("print_to_network(&state.queue").count(), 2, "print and test print");
    }

    // ── The id a tap on a notice comes back with ─────────────────────────────────────────────────
    //
    // The plugin takes an `i32`. An id the page sends out of that range must not stop the notice:
    // it goes out under the plugin's own id, and only the tap loses its destination.

    #[test]
    fn a_notice_keeps_the_id_the_page_gave_it() {
        assert_eq!(notice_id(Some(1_727_000_000)), Some(1_727_000_000));
        assert_eq!(notice_id(Some(i64::from(i32::MIN))), Some(i32::MIN));
        assert_eq!(notice_id(None), None);
    }

    #[test]
    fn an_id_the_plugin_cannot_hold_still_lets_the_notice_out() {
        assert_eq!(notice_id(Some(i64::from(i32::MAX) + 1)), None);
        assert_eq!(notice_id(Some(i64::MIN)), None);
    }

    /// The builder only shows its fields through `Debug`; that is what the tap is matched on.
    fn built_notice(id: Option<i64>) -> String {
        built_notice_to(id, None)
    }

    fn built_notice_to(id: Option<i64>, path: Option<&str>) -> String {
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_notification::init())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app");
        format!("{:?}", notice_builder(app.handle(), "New booking", "Ana · 10:00", id, path))
    }

    // ── The screen travels with the notice ───────────────────────────────────────────────────────
    //
    // A tap that STARTS the app on Android reaches a page that never saw the notice go out, so its
    // memory of ids is empty. The notice carries its screen in `extra`, which Android hands back
    // with the tap.

    #[test]
    fn the_notice_carries_the_screen_its_tap_opens() {
        let built = built_notice_to(Some(7), Some("/m/appointments"));
        // On `extra` itself: the mock app's Debug also lists Tauri's own `path` plugin.
        assert!(built.contains(r#"extra: {"path": String("/m/appointments")}"#), "{built}");
    }

    #[test]
    fn a_notice_that_leads_nowhere_carries_no_screen() {
        let built = built_notice_to(Some(7), None);
        assert!(built.contains("extra: {}"), "{built}");
    }

    fn app_with_kept_tap() -> tauri::App<tauri::test::MockRuntime> {
        use tauri::Manager;
        let app = tauri::test::mock_builder()
            .plugin(hub_link::android_plugin())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app");
        app.manage(notice_tap::KeptNoticeTap::default());
        app
    }

    #[test]
    fn the_page_claims_the_kept_tap_once() {
        use tauri::Manager;
        let app = app_with_kept_tap();
        app.state::<notice_tap::KeptNoticeTap>()
            .keep(notice_tap::NoticeTap { id: 5, path: Some("/m/kds".into()) });
        assert_eq!(
            take_notice_tap(app.handle()),
            Some(serde_json::json!({ "notification": { "id": 5, "extra": { "path": "/m/kds" } } }))
        );
        assert_eq!(take_notice_tap(app.handle()), None);
    }

    /// What the page keeps after `erplora_notify` shows a notice on the computer and the person
    /// clicks it: the notice goes through the same path as the command, with a clicking platform.
    #[cfg(desktop)]
    fn clicked_desktop_notice(id: Option<i64>, path: Option<&str>) -> Option<notice_tap::NoticeTap> {
        use tauri::Manager;
        let app = app_with_kept_tap();
        let shown = notify_on_desktop(app.handle().clone(), "New booking".into(), "Ana".into(), id, path.map(Into::into), |_, _, _, _| Ok(true));
        shown.expect("no thread for the notice").join().expect("the notice thread panicked");
        app.state::<notice_tap::KeptNoticeTap>().take()
    }

    #[cfg(desktop)]
    #[test]
    fn a_desktop_notice_carries_its_id_and_screen_back() {
        assert_eq!(
            clicked_desktop_notice(Some(7), Some("/m/kds")),
            Some(notice_tap::NoticeTap { id: 7, path: Some("/m/kds".into()) })
        );
        assert_eq!(clicked_desktop_notice(Some(7), None), Some(notice_tap::NoticeTap { id: 7, path: None }));
        // No id the platform can hold, nothing to open: the click only brings the window up.
        assert_eq!(clicked_desktop_notice(None, Some("/m/kds")), None);
        assert_eq!(clicked_desktop_notice(Some(i64::from(i32::MAX) + 1), Some("/m/kds")), None);
    }

    // On Windows a click on a notice — on screen or later in the Action Center, with the
    // app open or closed — comes back as a launch of the app with the notice's link.

    #[cfg(desktop)]
    #[test]
    fn a_second_launch_by_a_notice_link_keeps_its_tap() {
        use tauri::Manager;
        let app = app_with_kept_tap();
        let tap = notice_tap::NoticeTap { id: 9, path: Some("/m/kds".into()) };
        on_second_launch(app.handle(), vec!["ERPlora.exe".into(), notice_tap::notice_link(Some(&tap))]);
        assert_eq!(app.state::<notice_tap::KeptNoticeTap>().take(), Some(tap));
    }

    #[cfg(desktop)]
    #[test]
    fn a_second_launch_by_a_hub_link_keeps_no_tap() {
        use tauri::Manager;
        let app = app_with_kept_tap();
        on_second_launch(app.handle(), vec!["ERPlora.exe".into(), "erplora://hub/demo.a.erplora.com".into()]);
        assert_eq!(app.state::<notice_tap::KeptNoticeTap>().take(), None);
    }

    #[test]
    fn a_cold_launch_by_a_notice_link_keeps_its_tap_for_the_page() {
        // The app was closed: Windows starts it with the link, and the page claims the tap at boot.
        let source = include_str!("lib.rs");
        let shell = source.split("\n#[cfg(test)]\nmod tests").next().unwrap_or_default();
        let setup = shell.split(".setup(|app| {").nth(1).unwrap_or_default();
        let claim = setup.find("notice_tap::answer_link(app.handle(), std::env::args())");
        let kept = setup.find("app.manage(notice_tap::KeptNoticeTap::default())");
        let window = setup.find("open_main_window(app, cache_dir)");
        // Each one found, or `None < Some(_)` would pass the order check below without a place to keep
        // the tap (on_click would panic on the unmanaged state).
        let (Some(claim), Some(kept), Some(window)) = (claim, kept, window) else {
            panic!("setup lost one of: the claim {claim:?}, the kept tap {kept:?}, the window {window:?}");
        };
        assert!(kept < claim && window < claim, "the link is answered before there is somewhere to keep it");
    }

    #[test]
    fn the_store_copy_answers_the_app_s_links() {
        // The `.exe`/`.msi` register the scheme at install; the Store copy only has what its manifest
        // declares. Without it a click on a notice, or any `erplora://` link, reaches nobody there.
        let manifest = include_str!("../msix/Package.appxmanifest");
        let protocol = format!(r#"<uap:Protocol Name="{DEEP_LINK_SCHEME}""#);
        assert!(manifest.contains(r#"<uap:Extension Category="windows.protocol">"#), "no protocol extension");
        assert!(manifest.contains(&protocol), "the Store copy does not declare {DEEP_LINK_SCHEME}://");
    }

    #[test]
    fn the_tap_that_started_the_app_reads_like_any_other() {
        let tap = tauri_plugin_erplora_android::LaunchNoticeTap {
            id: 4,
            notification: Some(r#"{"id":4,"extra":{"path":"/m/appointments"}}"#.into()),
            path: None,
        };
        assert_eq!(
            launch_tap_payload::<String>(Ok(Some(tap))),
            Some(serde_json::json!({ "notification": { "id": 4, "extra": { "path": "/m/appointments" } } }))
        );
        assert_eq!(launch_tap_payload::<String>(Ok(None)), None);
        assert_eq!(launch_tap_payload(Err("plugin gone")), None);
    }

    // ── On Android a tap comes back with the notice's number only ────────────────────────────────

    #[test]
    fn a_kept_tap_without_its_notice_opens_the_screen_the_app_wrote_down() {
        // The notification plugin never stores the notice on the tap's intent: the kitchen's notice
        // comes back as its number, and the screen is the one Kotlin wrote down as it went out.
        let tap = tauri_plugin_erplora_android::LaunchNoticeTap {
            id: 8,
            notification: None,
            path: Some("/kitchen/display".into()),
        };
        assert_eq!(
            launch_tap_payload::<String>(Ok(Some(tap))),
            Some(serde_json::json!({ "notification": { "id": 8, "extra": { "path": "/kitchen/display" } } }))
        );
    }

    #[test]
    fn a_notice_writes_down_its_screen_under_the_number_it_goes_out_with() {
        let mut written = Vec::new();
        remember_notice_target(Some(1_727_000_008), Some("/kitchen/display"), |id, path| {
            written.push((id, path.map(str::to_owned)));
            Ok::<(), String>(())
        });
        assert_eq!(written, vec![(1_727_000_008, Some("/kitchen/display".to_owned()))]);
    }

    #[test]
    fn a_notice_without_a_number_the_plugin_holds_writes_nothing_down() {
        // Its tap comes back under the plugin's own number, which no screen was written under.
        for id in [None, Some(i64::from(i32::MAX) + 1)] {
            remember_notice_target(id, Some("/kitchen/display"), |_, _| -> Result<(), String> {
                panic!("{id:?} was written down")
            });
        }
    }

    #[test]
    fn a_screen_that_cannot_be_written_down_does_not_stop_the_notice() {
        // Never fails upwards, like the notice itself: the order behind it goes on.
        remember_notice_target(Some(7), Some("/kitchen/display"), |_, _| Err("plugin gone"));
    }

    #[test]
    fn on_android_the_shell_writes_down_each_notice_and_tells_the_page_a_tap_waits() {
        // Android-only lines the desktop build never compiles: each one going missing leaves the tap
        // on a notice where it was, with the app open.
        let source = include_str!("lib.rs");
        let shell = source.split("\n#[cfg(test)]\nmod tests").next().unwrap_or_default();
        let body_of = |start: &str| shell.split(start).nth(1).unwrap_or_default().split("\n}\n").next().unwrap_or_default();
        let notify = body_of("fn erplora_notify(");
        let remember = notify
            .find("remember_notice_target(id, path.as_deref(), |id, path| app.erplora_android().remember_notice(id, path))")
            .expect("erplora_notify does not write down the screen of the notice");
        let show = notify.find("notice_builder(&app,").expect("erplora_notify does not show the notice");
        assert!(remember < show, "the screen is written down after the notice is out: a quick tap would miss it");
        assert!(
            body_of("pub fn run() {").contains(".watch_notice_taps(move || notice_tap::tell_page(&"),
            "run() never hands Kotlin a way to tell the page a tap waits"
        );
    }

    #[test]
    fn nothing_kept_is_nothing_to_claim() {
        let app = app_with_kept_tap();
        assert_eq!(take_notice_tap(app.handle()), None);
    }

    #[test]
    fn the_notice_goes_out_under_the_id_its_tap_comes_back_with() {
        let built = built_notice(Some(1_727_000_042));
        assert!(built.contains("id: 1727000042"), "{built}");
        assert!(built.contains("New booking") && built.contains("Ana · 10:00"), "{built}");
    }

    #[test]
    fn a_notice_without_an_id_still_goes_out_with_its_words() {
        let built = built_notice(None);
        assert!(!built.contains("id: 1727000042"), "{built}");
        assert!(built.contains("New booking") && built.contains("Ana · 10:00"), "{built}");
    }

    // ── Adding a printer by typing its address ───────────────────────────────────────────────────
    //
    // The screen has to tell a typo from a printer that did not answer, in the user's language. A
    // bare string (what `HardwareError` sends) cannot be branched on without parsing prose, so this
    // command's refusal carries the stable code next to the message.

    #[test]
    fn a_refused_manual_printer_reaches_the_page_as_a_code_and_a_message() {
        let unreachable = AddPrinterError::from(erplora_peripherals::PeripheralError::Unreachable(
            "10.0.0.9:9100: connection refused".into(),
        ));
        let wire = serde_json::to_value(&unreachable).expect("serializes");
        assert_eq!(wire["code"], "printer_unreachable");
        assert!(wire["message"].as_str().is_some_and(|m| m.contains("10.0.0.9")));

        let typo = AddPrinterError::from(erplora_peripherals::PeripheralError::InvalidPrinterId(
            "not an IPv4 address".into(),
        ));
        assert_eq!(serde_json::to_value(&typo).expect("serializes")["code"], "invalid_printer_address");
    }

    fn url(s: &str) -> tauri::Url {
        s.parse().expect("valid test url")
    }

    fn tempdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("erplora-shell-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("tempdir");
        dir
    }

    // ── «Start on login» — OFF by default, the OS owns the state ─────────────────────────────
    //
    // Behind a seam because the real switch writes a LaunchAgent / registry key / autostart
    // file: a unit test that ENABLED it would leave the developer's machine starting a till at
    // login. The fake proves the command logic; the real impl is one delegation to the plugin.

    struct FakeAutostart {
        enabled: std::cell::Cell<bool>,
        /// A switch whose `enable()` succeeds but changes nothing — how a sandboxed macOS build
        /// or a broken LaunchAgent dir actually behaves.
        stuck: bool,
    }

    impl FakeAutostart {
        fn off() -> Self {
            Self { enabled: std::cell::Cell::new(false), stuck: false }
        }
    }

    impl AutostartSwitch for FakeAutostart {
        fn is_enabled(&self) -> Result<bool, String> {
            Ok(self.enabled.get())
        }
        fn set_enabled(&self, enabled: bool) -> Result<(), String> {
            if !self.stuck {
                self.enabled.set(enabled);
            }
            Ok(())
        }
    }

    #[test]
    fn autostart_is_off_until_somebody_turns_it_on() {
        // The whole sequence: default disabled → enable → is_enabled → disable.
        let switch = FakeAutostart::off();
        assert_eq!(switch.is_enabled(), Ok(false), "opt-in means the default is OFF");
        assert_eq!(apply_autostart(&switch, true), Ok(true));
        assert_eq!(switch.is_enabled(), Ok(true));
        assert_eq!(apply_autostart(&switch, false), Ok(false));
        assert_eq!(switch.is_enabled(), Ok(false));
    }

    #[test]
    fn the_reported_state_is_what_the_os_says_not_what_was_asked() {
        // No persistence of our own: the OS owns the state, so the answer must be
        // read BACK after the change. A switch that quietly failed to change must be reported as
        // it is — a toggle that shows ON while the OS says OFF is a till that never starts.
        let stuck = FakeAutostart { enabled: std::cell::Cell::new(false), stuck: true };
        assert_eq!(apply_autostart(&stuck, true), Ok(false));
    }

    // ── Bonded Bluetooth printers join discovery ─────────────────────────────────────────────
    //
    // The LAN sweep and the bonded list are two halves of ONE question — "which printers can
    // this device print on?" — so they come back as one list. The merge is pure so the emulator
    // (which has no Bluetooth at all) is not the only place it can be exercised.

    fn a_bonded_printer() -> tauri_plugin_erplora_android::BluetoothPrinter {
        serde_json::from_value(serde_json::json!({
            "id": "bluetooth:AA:BB:CC:DD:EE:FF",
            "name": "Kitchen BT",
            "mac": "AA:BB:CC:DD:EE:FF"
        }))
        .expect("the wire shape Kotlin builds")
    }

    #[test]
    fn a_bonded_bluetooth_printer_joins_the_scanned_list() {
        let merged = merge_bluetooth_printers(
            PrinterDiscovery::Scanned { printers: vec![] },
            vec![a_bonded_printer()],
        );
        let printers = merged.scanned_printers().expect("still a scanned outcome");
        assert_eq!(printers.len(), 1);
        assert_eq!(printers[0].id, "bluetooth:AA:BB:CC:DD:EE:FF");
        assert_eq!(printers[0].kind, "bluetooth");
        assert_eq!(printers[0].name, "Kitchen BT");
        assert_eq!(printers[0].mac.as_deref(), Some("AA:BB:CC:DD:EE:FF"));
    }

    #[test]
    fn a_blocked_lan_scan_stays_blocked_and_carries_no_printers() {
        // A limit, stated on purpose: with the LAN permission denied the outcome is the
        // permission refusal, and the bonded list does NOT ride along — an outcome that both
        // names a missing permission and lists printers would give the screen two contradictory
        // instructions at once. The permission prompt for discovery asks for both, so
        // this state is one "allow" away from resolving itself.
        let merged = merge_bluetooth_printers(
            PrinterDiscovery::PermissionDenied {
                permission: "android.permission.ACCESS_LOCAL_NETWORK".into(),
            },
            vec![a_bonded_printer()],
        );
        assert_eq!(merged.scanned_printers(), None);
    }

    #[test]
    fn a_bonded_printer_already_listed_is_not_duplicated() {
        let once = merge_bluetooth_printers(
            PrinterDiscovery::Scanned { printers: vec![] },
            vec![a_bonded_printer()],
        );
        let twice = merge_bluetooth_printers(once, vec![a_bonded_printer()]);
        assert_eq!(
            twice.scanned_printers().expect("scanned").len(),
            1,
            "the same bonded printer merged twice must stay one device"
        );
    }

    // ── The machine's USB print queues join discovery ────────────────────────────────────────
    //
    // The USB thermal printer is the cheapest in the catalogue and the one a single-till bar or
    // salon actually buys. It reaches us through the OS print queue, so the merge is pure and
    // testable on any laptop — including the CI machines that have no printer plugged in.

    fn a_usb_printer() -> erplora_peripherals::protocol::PrinterInfo {
        erplora_peripherals::protocol::PrinterInfo {
            id: "usb:Star_TSP143".into(),
            name: "Star TSP143".into(),
            kind: "usb".into(),
            category: erplora_peripherals::protocol::default_printer_category(),
            status: "ready".into(),
            paper_width: 80,
            mac: None,
        }
    }

    #[test]
    fn a_usb_queue_joins_the_scanned_list() {
        let merged =
            merge_usb_printers(PrinterDiscovery::Scanned { printers: vec![] }, vec![a_usb_printer()]);
        let printers = merged.scanned_printers().expect("still a scanned outcome");
        assert_eq!(printers.len(), 1);
        assert_eq!(printers[0].id, "usb:Star_TSP143");
        assert_eq!(printers[0].kind, "usb");
    }

    #[test]
    fn a_usb_queue_already_listed_is_not_duplicated() {
        let once =
            merge_usb_printers(PrinterDiscovery::Scanned { printers: vec![] }, vec![a_usb_printer()]);
        let twice = merge_usb_printers(once, vec![a_usb_printer()]);
        assert_eq!(
            twice.scanned_printers().expect("scanned").len(),
            1,
            "the same queue merged twice must stay one device"
        );
    }

    #[test]
    fn a_blocked_scan_stays_blocked_and_carries_no_usb_queues() {
        let merged = merge_usb_printers(
            PrinterDiscovery::PermissionDenied {
                permission: "android.permission.ACCESS_LOCAL_NETWORK".into(),
            },
            vec![a_usb_printer()],
        );
        assert_eq!(merged.scanned_printers(), None);
    }

    // ── A discovered queue is a DEVICE, so the owner can say it is the kitchen's ─────────────
    //
    // Listing the queue is not enough: without an entry in the registry it can be printed to but
    // never named, so a venue with a USB printer at the counter and a network one in the kitchen
    // can only tell the hub about one of the two.

    #[test]
    fn a_discovered_usb_queue_enters_the_device_registry() {
        let registry = DeviceRegistry::load(tempdir().join("devices.json"));
        let second = erplora_peripherals::protocol::PrinterInfo {
            id: "usb:EPSON_TM".into(),
            name: "EPSON TM-T20III".into(),
            ..a_usb_printer()
        };

        register_discovered_queues(&registry, &[a_usb_printer(), second]);

        let mut keys: Vec<String> = registry.get_all().into_iter().map(|d| d.key).collect();
        keys.sort();
        assert_eq!(
            keys,
            ["usb:EPSON_TM", "usb:Star_TSP143"],
            "each queue keeps an identity of its own"
        );
        registry
            .set_role("usb:Star_TSP143", "kitchen")
            .expect("and can be told which paper it prints");
    }

    #[test]
    fn a_queue_that_cannot_be_registered_does_not_cost_the_others_theirs() {
        // A discovery is a batch: one id the registry refuses must not take the working printer
        // down with it, or a single odd queue would leave the till with no roles at all.
        let registry = DeviceRegistry::load(tempdir().join("devices.json"));
        let broken = erplora_peripherals::protocol::PrinterInfo {
            id: "usb:".into(),
            name: "nameless".into(),
            ..a_usb_printer()
        };

        register_discovered_queues(&registry, &[broken, a_usb_printer()]);

        let keys: Vec<String> = registry.get_all().into_iter().map(|d| d.key).collect();
        assert_eq!(keys, ["usb:Star_TSP143"], "the good one is registered anyway");
    }

    // ── Forgetting BY CHOICE lands on the chooser, forgetting on a 410 does not ──────────────
    //
    // `/shell/` redirects a single-hub user straight back into their hub — correct after a 410
    // (the hub is gone, the SaaS will route somewhere sane), useless for «I want to pick»: the
    // owner with two businesses and one tablet would bounce right back into the one they were
    // trying to leave. `?choose=1` is the SaaS's own affordance for forcing the list.

    #[test]
    fn forgetting_by_choice_lands_on_the_chooser() {
        assert_eq!(
            forget_destination("https://erplora.com", true),
            "https://erplora.com/shell/?choose=1"
        );
    }

    #[test]
    fn forgetting_on_a_410_keeps_the_plain_onboarding() {
        // The 410 path sends no `choose`: with the hub gone there is nothing to pick between,
        // and the plain onboarding lets the SaaS route (or re-onboard) as it sees fit.
        assert_eq!(
            forget_destination("https://erplora.com", false),
            "https://erplora.com/shell/"
        );
    }

    // ── Which hub the window starts linked to, and the wiring of the gate ───────────────────────

    #[test]
    fn a_fresh_install_starts_linked_to_nothing_and_lets_the_saas_choose() {
        let (entries, linked) = boot_link(None, None, "https://erplora.com");
        assert_eq!(entries, vec!["https://erplora.com".to_string()]);
        assert_eq!(linked, None);
    }

    #[test]
    fn the_remembered_hub_is_the_linked_one_at_boot() {
        let (_, linked) = boot_link(None, Some("https://panaderia.a.erplora.com"), "https://erplora.com");
        assert_eq!(linked.as_deref(), Some("https://panaderia.a.erplora.com"));
    }

    // ── A link to another hub waits for the person's word ───────────────────────────────────────

    #[test]
    fn a_cold_start_by_a_link_to_another_hub_boots_on_the_linked_one_and_asks() {
        let (_, linked) = boot_link(None, Some("https://panaderia.a.erplora.com"), "https://erplora.com");
        assert_eq!(linked.as_deref(), Some("https://panaderia.a.erplora.com"));
        assert_eq!(
            cold_start_link(Some("https://otronegocio.a.erplora.com/?shell=1"), linked.as_deref()),
            ColdStartLink::Ask("https://otronegocio.a.erplora.com/?shell=1".into())
        );
    }

    #[test]
    fn a_cold_start_by_a_link_to_the_linked_hub_boots_on_it_without_asking() {
        assert_eq!(
            cold_start_link(
                Some("https://panaderia.a.erplora.com/?shell=1"),
                Some("https://panaderia.a.erplora.com")
            ),
            ColdStartLink::Open("https://panaderia.a.erplora.com/?shell=1".into())
        );
    }

    #[test]
    fn a_cold_start_by_a_link_on_a_device_linked_to_nothing_asks() {
        assert_eq!(
            cold_start_link(Some("https://otronegocio.a.erplora.com/?shell=1"), None),
            ColdStartLink::Ask("https://otronegocio.a.erplora.com/?shell=1".into())
        );
        assert_eq!(cold_start_link(None, Some("https://panaderia.a.erplora.com")), ColdStartLink::None);
    }

    /// A till as it stands after booting on the bakery it remembers (`boot_link` links it, the
    /// window shows it), and the person in front of it: `None` is nobody there to ask.
    fn till_of_the_bakery(
        linked: Option<&str>,
        person: Option<bool>,
    ) -> (tauri::App<tauri::test::MockRuntime>, Arc<std::sync::Mutex<Vec<link_consent::Question>>>) {
        use tauri::Manager;
        let app = app_with_kept_tap();
        let (entries, _) = boot_link(None, linked, "https://erplora.com");
        let showing = linked.map(|hub| url(&format!("{hub}/")));
        let link = hub_link::HubLink::new(entries, showing.as_ref());
        if let Some(hub) = linked {
            link.link(hub);
        }
        app.manage(link);
        let start = showing.unwrap_or_else(|| url("https://erplora.com/shell/"));
        tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::External(start))
            .build()
            .expect("mock window");
        let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
        if let Some(open) = person {
            let heard = asked.clone();
            app.manage(link_consent::AskBeforeLinking::new(move |question, answer| {
                heard.lock().expect("questions").push(question);
                answer(open);
            }));
        }
        (app, asked)
    }

    fn window_url(app: &tauri::App<tauri::test::MockRuntime>) -> String {
        use tauri::Manager;
        app.get_webview_window("main").expect("window").url().expect("url").to_string()
    }

    fn linked_hub(app: &tauri::App<tauri::test::MockRuntime>) -> Option<String> {
        use tauri::Manager;
        app.state::<hub_link::HubLink>().linked()
    }

    const BAKERY: &str = "https://panaderia.a.erplora.com";
    const OTHER_LINK: &str = "https://otronegocio.a.erplora.com/?shell=1";

    #[test]
    fn a_link_to_another_hub_does_not_move_the_till_with_nobody_to_ask() {
        let (app, _) = till_of_the_bakery(Some(BAKERY), None);
        open_hub_from_link(app.handle(), OTHER_LINK);
        assert_eq!(linked_hub(&app).as_deref(), Some(BAKERY), "the printer went to the link's hub");
        assert_eq!(window_url(&app), "https://panaderia.a.erplora.com/");
    }

    #[test]
    fn a_link_to_another_hub_links_it_once_the_person_says_open() {
        let (app, asked) = till_of_the_bakery(Some(BAKERY), Some(true));
        open_hub_from_link(app.handle(), OTHER_LINK);
        assert_eq!(
            *asked.lock().expect("questions"),
            vec![link_consent::Question {
                to: "otronegocio.a.erplora.com".into(),
                from: Some("panaderia.a.erplora.com".into()),
            }]
        );
        assert_eq!(linked_hub(&app).as_deref(), Some("https://otronegocio.a.erplora.com"));
        assert_eq!(window_url(&app), OTHER_LINK);
    }

    #[test]
    fn cancelling_leaves_the_till_on_its_business() {
        let (app, asked) = till_of_the_bakery(Some(BAKERY), Some(false));
        open_hub_from_link(app.handle(), OTHER_LINK);
        assert_eq!(asked.lock().expect("questions").len(), 1, "nobody was asked");
        assert_eq!(linked_hub(&app).as_deref(), Some(BAKERY));
        assert_eq!(window_url(&app), "https://panaderia.a.erplora.com/");
    }

    #[test]
    fn a_link_to_the_linked_hub_opens_it_without_asking() {
        let (app, asked) = till_of_the_bakery(Some(BAKERY), Some(false));
        open_hub_from_link(app.handle(), "https://panaderia.a.erplora.com/?shell=1");
        assert!(asked.lock().expect("questions").is_empty(), "the till asked to open its own hub");
        assert_eq!(linked_hub(&app).as_deref(), Some(BAKERY));
        assert_eq!(window_url(&app), "https://panaderia.a.erplora.com/?shell=1");
    }

    #[test]
    fn a_link_on_a_till_linked_to_nothing_asks_too() {
        // A fresh install (or one that just changed business): yes would link the hub AND remember
        // it for every boot to come, so a link from an email has to ask here as well.
        let (app, asked) = till_of_the_bakery(None, Some(false));
        open_hub_from_link(app.handle(), OTHER_LINK);
        assert_eq!(
            *asked.lock().expect("questions"),
            vec![link_consent::Question { to: "otronegocio.a.erplora.com".into(), from: None }]
        );
        assert_eq!(linked_hub(&app), None);
        assert_eq!(window_url(&app), "https://erplora.com/shell/");
    }

    #[test]
    fn the_development_override_links_a_local_hub_but_not_a_local_saas() {
        let (entries, linked) =
            boot_link(Some("http://127.0.0.1:5173/"), None, "https://erplora.com");
        assert_eq!(linked.as_deref(), Some("http://127.0.0.1:5173"));
        assert!(entries.contains(&"http://127.0.0.1:5173".to_string()));
        let (entries, linked) = boot_link(
            Some("http://127.0.0.1:8001/shell/"),
            Some("https://panaderia.a.erplora.com"),
            "http://127.0.0.1:8001",
        );
        assert_eq!(linked, None, "the local SaaS became the linked hub");
        assert_eq!(entries, vec!["http://127.0.0.1:8001".to_string()]);
    }

    #[cfg(desktop)]
    #[test]
    fn a_link_handed_over_by_a_second_launch_asks_before_linking_its_hub() {
        let (app, asked) = till_of_the_bakery(Some(BAKERY), Some(false));
        on_second_launch(app.handle(), vec!["ERPlora.exe".into(), "erplora://hub/otronegocio.a.erplora.com".into()]);
        assert_eq!(asked.lock().expect("questions").len(), 1, "nobody was asked");
        assert_eq!(linked_hub(&app).as_deref(), Some(BAKERY));
    }

    #[test]
    fn forgetting_the_hub_unlinks_it() {
        use tauri::Manager;
        let app = app_with_kept_tap();
        let link = hub_link::HubLink::new(vec!["https://erplora.com".into()], None);
        link.link("https://panaderia.a.erplora.com");
        app.manage(link);
        forget_linked_hub(app.handle());
        assert_eq!(app.state::<hub_link::HubLink>().linked(), None);
    }

    #[test]
    fn the_app_wires_the_linked_hub_gate() {
        // The gate and the link are only as good as their call sites: each of these lines going
        // missing hands the device back to every page under erplora.com.
        let source = include_str!("lib.rs");
        let shell = source.split("\n#[cfg(test)]\nmod tests").next().unwrap_or_default();
        let body_of = |start: &str| shell.split(start).nth(1).unwrap_or_default().split("\n}\n").next().unwrap_or_default();
        assert!(
            body_of("pub fn run() {").contains(".invoke_handler(hub_link::guard(app_commands()))"),
            "run() hands Tauri the commands without the gate"
        );
        assert!(
            body_of("pub fn run() {").contains(".plugin(hub_link::android_plugin())"),
            "run() registers the Android plugin without the gate"
        );
        assert!(
            body_of("pub fn run() {").contains(".plugin(hub_link::notification_plugin())")
                && !shell.contains(".plugin(tauri_plugin_notification::init())"),
            "run() registers the notification plugin without the gate"
        );
        let window = body_of("fn open_main_window(");
        assert!(window.contains("app.manage(hub_link)"), "the window opens with no link state");
        let navigation = window.split(".on_navigation(").nth(1).unwrap_or_default();
        assert!(navigation.contains(".follow(nav)"), "a navigation links a hub without asking who chose it");
        assert!(!navigation.contains("shell_capture_origin(nav)"), "a navigation links a hub without asking who chose it");
        let loaded = window.split(".on_page_load(").nth(1).unwrap_or_default();
        assert!(
            loaded.contains("PageLoadEvent::Finished") && loaded.contains(".landed("),
            "the hardware opens before the hub's page has finished loading: the page that sent the window there may still run"
        );
        assert!(body_of("fn forget_hub(").contains("forget_linked_hub(&app)"), "«Change business» keeps the old hub linked");
        assert!(
            body_of("fn spawn_hub_liveness_check(").contains("forget_linked_hub(&app)"),
            "a deleted hub stays linked"
        );
        assert!(
            body_of("pub fn run() {").contains("Some(target) => open_hub_from_link(&handle, &target)"),
            "a link with the app open navigates without linking its hub"
        );
        // The person is asked by the system's own dialog, set up before the window
        // exists, and a cold start by a link to another hub goes through that same question.
        let setup = body_of("pub fn run() {").split(".setup(|app| {").nth(1).unwrap_or_default();
        let asker = setup.find("app.manage(link_consent::native(app.handle().clone()))");
        let window_opens = setup.find("open_main_window(app, cache_dir)");
        assert!(
            matches!((asker, window_opens), (Some(a), Some(w)) if a < w),
            "a link reaches a till with nobody to ask (asker {asker:?}, window {window_opens:?})"
        );
        assert!(window.contains("cold_start_link(deep_link.as_deref(), linked.as_deref())"));
        assert!(
            window.contains("ColdStartLink::Ask(target) => open_hub_from_link(app.handle(), &target)"),
            "a cold start by a link to another hub never asks"
        );
    }

    // ── shell_capture_origin: the contract of the ?shell=1 marker ────────────────────────────

    #[test]
    fn capture_requires_the_shell_1_marker() {
        assert_eq!(shell_capture_origin(&url("https://demo.erplora.com/")), None);
        assert_eq!(
            shell_capture_origin(&url("https://demo.erplora.com/?shell=2")),
            None
        );
        assert_eq!(
            shell_capture_origin(&url("https://demo.erplora.com/?other=1")),
            None
        );
    }

    #[test]
    fn capture_returns_the_https_origin() {
        assert_eq!(
            shell_capture_origin(&url("https://demo.erplora.com/pos?shell=1&x=2")),
            Some("https://demo.erplora.com".to_string())
        );
    }

    #[test]
    fn capture_accepts_http_only_on_loopback() {
        // Remote http is NOT captured (a deployed hub is always https).
        assert_eq!(shell_capture_origin(&url("http://evil.com/?shell=1")), None);
        // Loopback is (local development: runtime :8787 / Vite :5173).
        assert_eq!(
            shell_capture_origin(&url("http://127.0.0.1:8787/?shell=1")),
            Some("http://127.0.0.1:8787".to_string())
        );
        assert_eq!(
            shell_capture_origin(&url("http://localhost:5173/?shell=1")),
            Some("http://localhost:5173".to_string())
        );
    }

    #[test]
    fn capture_refuses_non_http_schemes() {
        assert_eq!(shell_capture_origin(&url("tauri://localhost/?shell=1")), None);
        assert_eq!(shell_capture_origin(&url("file:///tmp/x?shell=1")), None);
    }

    // ── The marker says "remember me"; it does not say WHO may ask ────────────────────────────
    //
    // If `?shell=1` were the whole capture contract, ANY https origin that carried it would become
    // the origin this installation boots at, for good. The shell blocks no navigation to a foreign
    // host (`on_navigation` refuses only SaaS pages, in the Play copy), so one link would be
    // enough: an open redirect on the SaaS, an injection into the third-party checkout the same
    // window loads, or simply a link a user taps. From then on the till would open full-screen,
    // chrome-less and titled "ERPlora" on somebody else's page, and the operator would type the
    // hub password into it.
    //
    // The destination rule is therefore the SAME one the deep link applies and the same one
    // `capabilities/*.json` declares: only a hub of ours, or the loopback of this machine. The
    // three must agree — an origin the capabilities refuse is a till that looks fine and cannot
    // print, which is the failure `remote_acl.rs` exists to prevent.

    #[test]
    fn capture_refuses_an_origin_that_is_not_a_hub_of_ours() {
        // A captured origin keeps a non-default port, but a stranger's domain with one is still
        // not this till's hub.
        assert_eq!(
            shell_capture_origin(&url("https://hub.example.com:8443/?shell=1")),
            None
        );
        assert_eq!(shell_capture_origin(&url("https://evil.com/?shell=1")), None);
    }

    #[test]
    fn capture_refuses_the_saas_apex() {
        // `erplora.com` is where the app BOOTS, not a hub — and it is deliberately kept out of the
        // hardware capability. Remembering it would pin the till to the marketing site
        // on every cold start, with the onboarding one redirect further away than before.
        assert_eq!(shell_capture_origin(&url("https://erplora.com/?shell=1")), None);
        assert_eq!(
            shell_capture_origin(&url("https://erplora.com/shell/?shell=1")),
            None
        );
    }

    #[test]
    fn capture_refuses_the_domains_that_only_look_like_ours() {
        // Same three shapes `remote_acl.rs` rejects at the ACL, checked here at the capture: a
        // boundary that only holds in one of the two places holds nowhere.
        assert_eq!(
            shell_capture_origin(&url("https://demo.a.erplora.com.attacker.com/?shell=1")),
            None
        );
        assert_eq!(shell_capture_origin(&url("https://myerplora.com/?shell=1")), None);
        // Reads as ours to a human; the host is `evil.com`.
        assert_eq!(
            shell_capture_origin(&url("https://demo.a.erplora.com@evil.com/?shell=1")),
            None
        );
    }

    #[test]
    fn capture_accepts_every_shape_a_real_cloud_hub_takes() {
        // A lettered aura and a numbered aura — TWO labels under the registrable domain. A rule
        // that only allowed one would kill the app in production.
        assert_eq!(
            shell_capture_origin(&url("https://panaderia.a.erplora.com/pos?shell=1&x=2")),
            Some("https://panaderia.a.erplora.com".to_string())
        );
        assert_eq!(
            shell_capture_origin(&url("https://panaderia.3.erplora.com/?shell=1")),
            Some("https://panaderia.3.erplora.com".to_string())
        );
    }

    #[test]
    fn capture_accepts_loopback_over_tls_too() {
        // A dev PWA served over TLS (`vite --https`) is the same machine as the plain-http one, so
        // the loopback exemption belongs to the HOST, not to the scheme.
        assert_eq!(
            shell_capture_origin(&url("https://127.0.0.1:5173/?shell=1")),
            Some("https://127.0.0.1:5173".to_string())
        );
    }

    // ── An install poisoned by an older build must not stay poisoned ──────────────────────────

    #[test]
    fn a_persisted_origin_that_is_not_a_hub_of_ours_is_refused_on_load() {
        // Refusing at the capture only protects installs that have not been captured yet. Every
        // till that already ran a build without the check carries the foreign origin in
        // `hub.url`, and an upgrade that keeps reading it blindly fixes nothing where it matters.
        // `None` sends the window back to the onboarding, which is the one screen it can leave.
        let dir = tempdir();
        std::fs::write(dir.join(HUB_URL_FILE), "https://evil.com").expect("write");
        assert_eq!(load_hub_url(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_persisted_value_that_is_not_even_a_url_is_refused_on_load() {
        // `hub.url` is a plain file in the user's data dir: nothing stops it being edited, or
        // truncated by a crash. Without the check `initial_url_for` would hand whatever it said
        // to the webview.
        let dir = tempdir();
        std::fs::write(dir.join(HUB_URL_FILE), "not a url").expect("write");
        assert_eq!(load_hub_url(&dir), None);
        std::fs::write(dir.join(HUB_URL_FILE), "javascript:alert(1)").expect("write");
        assert_eq!(load_hub_url(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_persisted_hub_loads_reduced_to_its_ORIGIN() {
        // Whatever ends up in the file, what boots is the origin: a stored path or query would
        // otherwise be replayed on every cold start.
        let dir = tempdir();
        std::fs::write(dir.join(HUB_URL_FILE), "https://demo.a.erplora.com/pos?x=1\n").expect("write");
        assert_eq!(load_hub_url(&dir), Some("https://demo.a.erplora.com".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── A user with SEVERAL hubs ──────────────────────────────────────────────────────────────

    #[test]
    fn capturing_a_second_hub_replaces_the_first() {
        // How switching hubs works, and the reason it needs no extra machinery: `/shell/?choose=1`
        // lists the user's hubs and 302s into the chosen one carrying the marker, so the capture
        // that persists hub B is the same one that persisted hub A. Were the first capture sticky,
        // a till moved to another business would keep opening the old one.
        let dir = tempdir();
        persist_hub_url(&dir, "https://a.a.erplora.com").expect("first");
        persist_hub_url(&dir, "https://b.a.erplora.com").expect("second");
        assert_eq!(load_hub_url(&dir), Some("https://b.a.erplora.com".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Persisting hub.url ───────────────────────────────────────────────────────────────────

    #[test]
    fn hub_url_roundtrip_persist_load_clear() {
        let dir = tempdir();
        assert_eq!(load_hub_url(&dir), None);
        persist_hub_url(&dir, "https://demo.erplora.com").expect("persist");
        assert_eq!(load_hub_url(&dir), Some("https://demo.erplora.com".to_string()));
        clear_hub_url(&dir);
        assert_eq!(load_hub_url(&dir), None);
        clear_hub_url(&dir); // best effort: clearing again does not fail
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_hub_url_ignores_blank_files_and_trims() {
        let dir = tempdir();
        std::fs::write(dir.join(HUB_URL_FILE), "  \n").expect("write");
        assert_eq!(load_hub_url(&dir), None);
        std::fs::write(dir.join(HUB_URL_FILE), "https://a.erplora.com\n").expect("write");
        assert_eq!(load_hub_url(&dir), Some("https://a.erplora.com".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persist_hub_url_creates_the_directory() {
        let dir = tempdir().join("anidado");
        persist_hub_url(&dir, "https://b.erplora.com").expect("persist");
        assert_eq!(load_hub_url(&dir), Some("https://b.erplora.com".to_string()));
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    // ── Start URL: precedence dev override → persisted hub → onboarding ──────────────────────

    #[test]
    fn initial_url_precedence() {
        // The development override wins over the persisted hub.
        assert_eq!(
            initial_url_for(
                None,
                Some("http://127.0.0.1:8001/shell/"),
                Some("https://demo.erplora.com"),
                "https://erplora.com"
            ),
            "http://127.0.0.1:8001/shell/"
        );
        // Persisted hub → app mode (origin + "/").
        assert_eq!(
            initial_url_for(None, None, Some("https://demo.erplora.com"), "https://erplora.com"),
            "https://demo.erplora.com/"
        );
        // Nothing persisted → the SaaS onboarding.
        assert_eq!(
            initial_url_for(None, None, None, "https://erplora.com"),
            "https://erplora.com/shell/"
        );
    }

    #[test]
    fn a_cold_start_through_a_link_opens_the_hub_the_link_named() {
        // Windows and Linux answer a link by launching the executable with the URL appended, so at
        // a COLD start the link is all there is. If the persisted hub won, clicking "Open Terminal"
        // for hub B from a till that remembers hub A would quietly open A — the wrong business,
        // and with no error to notice it by.
        assert_eq!(
            initial_url_for(
                Some("https://demo.b.erplora.com/?shell=1"),
                None,
                Some("https://demo.a.erplora.com"),
                "https://erplora.com"
            ),
            "https://demo.b.erplora.com/?shell=1"
        );
        // It also beats the dev override: the override is static configuration, the link is what
        // the user is asking for right now.
        assert_eq!(
            initial_url_for(
                Some("https://demo.b.erplora.com/?shell=1"),
                Some("http://127.0.0.1:8001/shell/"),
                None,
                "https://erplora.com"
            ),
            "https://demo.b.erplora.com/?shell=1"
        );
    }

    // ── The remembered hub no longer exists: the app must NOT be walled in ───────────────────

    #[test]
    fn a_deleted_hub_is_forgotten_at_start() {
        // Seen on a Mac: `hub.url` pointed at a hub a production purge had deleted, and the app
        // opened on «404 page not found» — for good and with NO way out. The existing `forget_hub`
        // does not help here: the frontend calls it on a 410, and in this case the PWA never loads
        // at all because the 404 is served by the edge.
        assert!(should_forget_hub(HubProbe::Status(404)));
        // 410 is the server's explicit `hub_not_found` contract.
        assert!(should_forget_hub(HubProbe::Status(410)));
    }

    #[test]
    fn a_live_hub_is_not_forgotten() {
        assert!(!should_forget_hub(HubProbe::Status(200)));
        // The hub redirects to the login: it exists.
        assert!(!should_forget_hub(HubProbe::Status(302)));
    }

    #[test]
    fn being_unauthenticated_does_not_mean_the_hub_is_gone() {
        // Confusing the two would throw the user back to the onboarding every time a session
        // expires.
        assert!(!should_forget_hub(HubProbe::Status(401)));
        assert!(!should_forget_hub(HubProbe::Status(403)));
    }

    #[test]
    fn a_hub_that_is_down_is_not_forgotten() {
        // Down ≠ gone. Forgetting it over a 30-second outage would delete the user's hub and send
        // them through the onboarding again, which is FAR worse than waiting.
        for s in [500, 502, 503, 504] {
            assert!(!should_forget_hub(HubProbe::Status(s)), "{s} must not forget the hub");
        }
    }

    #[test]
    fn no_network_forgets_nothing() {
        // A till starts in venues with bad Wi-Fi, with the router rebooting or with the laptop not
        // yet associated. Deleting the hub for not reaching it would be catastrophic: the hub is
        // perfectly alive and the user would end up on the onboarding without knowing why.
        assert!(!should_forget_hub(HubProbe::Unreachable));
    }

    #[test]
    fn onboarding_url_normalizes_the_base() {
        assert_eq!(onboarding_url("https://erplora.com"), "https://erplora.com/shell/");
        assert_eq!(
            initial_url_for(None, None, None, "https://erplora.com/"),
            "https://erplora.com/shell/"
        );
        assert_eq!(normalize_base("  https://erplora.com/  "), "https://erplora.com");
    }

    // ── Device identity ──────────────────────────────────────────────────────────────────────

    #[test]
    fn ensure_device_id_is_stable_across_calls() {
        let dir = tempdir();
        let a = ensure_device_id(&dir).expect("first");
        let b = ensure_device_id(&dir).expect("second");
        assert_eq!(a, b);
        assert!(!a.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── "The OS won't let me look" is not "there are no printers" ────────────────────────────
    //
    // The shell is where the two states are told apart, because it is the only layer that can ask
    // Android what it granted. Get this wrong in either direction and the till hands the user the
    // opposite instruction to the one that would fix it.

    use erplora_peripherals::discovery::LocalNetworkAccess;
    use std::collections::HashMap;
    use tauri_plugin_erplora_android::{ACCESS_LOCAL_NETWORK, POST_NOTIFICATIONS};

    fn permission_status(pairs: &[(&str, bool)]) -> HashMap<String, bool> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
    }

    #[test]
    fn a_platform_that_never_answers_is_not_a_platform_that_said_no() {
        // Desktop returns an empty map — there is no such permission on Windows/macOS/Linux, and
        // Android below API 37 does not know it either. Reading that silence as a denial would
        // ground a till whose printer is right there.
        assert_eq!(
            local_network_access(&HashMap::new()),
            LocalNetworkAccess::Granted
        );
    }

    #[test]
    fn a_granted_permission_lets_the_scan_run() {
        assert_eq!(
            local_network_access(&permission_status(&[(ACCESS_LOCAL_NETWORK, true)])),
            LocalNetworkAccess::Granted
        );
    }

    #[test]
    fn a_denied_permission_names_itself_instead_of_faking_an_empty_venue() {
        // The name is what turns a dead end into an instruction: the screen can say which toggle
        // to flip rather than inviting the user to go hunting for a printer that is already on.
        assert_eq!(
            local_network_access(&permission_status(&[(ACCESS_LOCAL_NETWORK, false)])),
            LocalNetworkAccess::Denied {
                permission: ACCESS_LOCAL_NETWORK.to_string(),
            }
        );
    }

    #[test]
    fn saying_no_to_notifications_does_not_ground_printer_discovery() {
        // Two independent permissions asked in the same dialog run. Refusing the notifications one
        // must not make the till claim it cannot reach a network it can reach.
        assert_eq!(
            local_network_access(&permission_status(&[
                (POST_NOTIFICATIONS, false),
                (ACCESS_LOCAL_NETWORK, true),
            ])),
            LocalNetworkAccess::Granted
        );
        assert_eq!(
            local_network_access(&permission_status(&[(POST_NOTIFICATIONS, false)])),
            LocalNetworkAccess::Granted
        );
    }

    #[test]
    fn denying_both_still_blames_the_permission_that_actually_blocks_the_printers() {
        assert_eq!(
            local_network_access(&permission_status(&[
                (POST_NOTIFICATIONS, false),
                (ACCESS_LOCAL_NETWORK, false),
            ])),
            LocalNetworkAccess::Denied {
                permission: ACCESS_LOCAL_NETWORK.to_string(),
            }
        );
    }

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

    // ── The app must not introduce itself as «v0.0.0» ────────────────────────────────────────
    // The printing screen shows the version `erplora_bridge_status` answers. NOBODY stamps
    // `CARGO_PKG_VERSION`: the release (`tauri-release.yml`) writes the version into
    // `tauri.conf.json`, and this shell's `Cargo.toml` stays at 0.0.0 for good. Answering it would
    // make an installed 1.0.0 app introduce itself as «Impresora lista · v0.0.0.», with no way to
    // tell from inside which binary is running.

    #[test]
    fn the_status_version_is_not_the_crate_version() {
        // If this fails somebody started stamping Cargo.toml — good news, but then this test no
        // longer proves anything and it has to be decided which of the two versions wins.
        assert_eq!(env!("CARGO_PKG_VERSION"), "0.0.0");
        assert_ne!(bridge_status_version(), "0.0.0");
    }

    #[test]
    fn the_version_comes_from_tauri_conf_json() {
        assert_eq!(
            tauri_conf_version(r#"{"productName":"ERPlora","version":"1.2.3"}"#),
            Some("1.2.3".to_string())
        );
        // An unreadable conf must not bring the settings screen down: the answer is "unknown".
        assert_eq!(tauri_conf_version("{ no es json"), None);
        assert_eq!(tauri_conf_version(r#"{"version":42}"#), None);
    }

    // ── What the window does with a page the Play copy refuses ──────────────────────────────────

    /// A real window on Tauri's mock runtime, which records `navigate()` and answers `url()` with
    /// it — so the answer to a refusal is asserted as a MOVE (or its absence), not as a bool.
    fn mock_window_at(start: &str) -> tauri::WebviewWindow<tauri::test::MockRuntime> {
        // `mock_context(noop_assets())`, not `generate_context!()`: see `connectivity::tests`.
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app");
        tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::External(url(start)))
            .build()
            .expect("mock window")
    }

    #[test]
    fn the_saas_home_page_takes_the_window_to_the_app_s_start() {
        let window = mock_window_at("https://erplora.com/accounts/logout/");
        answer_refusal(&window, NavigationVerdict::Home, "https://erplora.com").expect("answer");
        assert_eq!(
            window.url().expect("url").as_str(),
            "https://erplora.com/shell/"
        );
    }

    #[test]
    fn a_refused_page_leaves_the_window_where_it_was() {
        // Anywhere but the app's start, or "stayed" and "went home" would look the same.
        let here = "https://erplora.com/account/login/?next=/shell/";
        let window = mock_window_at(here);
        answer_refusal(&window, NavigationVerdict::Refuse, "https://erplora.com").expect("answer");
        assert_eq!(window.url().expect("url").as_str(), here);
    }

    #[test]
    fn the_refusal_notice_speaks_both_languages_and_names_no_other_place_to_go() {
        let script = refusal_notice_script();
        // The English source and its Spanish translation, picked the way the
        // bundled offline page picks them (Spanish unless the device says otherwise).
        assert!(script.contains(REFUSAL_NOTICE_EN), "{script}");
        assert!(script.contains(REFUSAL_NOTICE_ES), "{script}");
        assert!(script.contains("navigator.language"), "{script}");
        assert!(script.starts_with("alert("), "{script}");
        // Anti-steering cuts both ways: telling the person to finish this on the website is the
        // very communication Play forbids, so the notice may not name any address.
        for text in [REFUSAL_NOTICE_EN, REFUSAL_NOTICE_ES] {
            let lower = text.to_lowercase();
            for forbidden in ["erplora.com", "http", "www", "browser", "navegador", "web"] {
                assert!(
                    !lower.contains(forbidden),
                    "the notice points elsewhere: {text}"
                );
            }
        }
    }
}
