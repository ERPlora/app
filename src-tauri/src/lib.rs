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

use std::path::PathBuf;

use serde::Serialize;

/// What the window shows when the network dies under it.
mod connectivity;
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
mod autostart;
mod device;
mod downloads;
mod hardware;
mod hub_url;
mod links;
mod nfc;
mod notices;
mod printer_discovery;
mod printing;
mod window;
pub use device::DeviceContext;
pub use downloads::{
    download_file_name, downloads_dir_is_reachable, free_download_path, reachable_downloads_dir, save_target,
    SaveTarget, SavedDownload,
};
pub use hardware::HardwareError;
pub use hub_url::shell_capture_origin;
pub use links::{deep_link_from_args, external_browser_url, resolve_deep_link, DEEP_LINK_SCHEME};
pub use nfc::NfcReadOutcome;

use hardware::build_peripherals_state;
use window::{open_hub_from_link, open_main_window};

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

/// Registrable domain every ERPlora hub lives under (`{slug}.{aura}.erplora.com`).
/// Kept at the crate root: the Android plugin checks its own copy of the domain against this line.
const HUB_DOMAIN_SUFFIX: &str = ".erplora.com";

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
        device::device_context,
        window::forget_hub,
        links::open_external_url,
        downloads::save_download,
        // The system print dialog for an A4 document.
        native_print::print_document,
        // The way out when the network dies under the window.
        window::shell_retry,
        // Data does NOT go through `invoke`: the PWA talks HTTP and WebSocket to its hub.
        // Hardware path: ESC/POS network printers and the cash drawer → peripherals.
        hardware::erplora_bridge_status,
        printer_discovery::erplora_discover_printers,
        hardware::erplora_get_devices,
        printing::erplora_print,
        printing::erplora_test_print,
        printing::erplora_open_drawer,
        hardware::erplora_set_device_role,
        printer_discovery::erplora_add_network_printer,
        hardware::erplora_set_device_name,
        hardware::erplora_remove_device,
        notices::erplora_notify,
        // The tap the page was not there to hear.
        notices::erplora_take_notice_tap,
        // The badge over NFC: the second way through the SAME door as the keyboard-wedge reader.
        nfc::erplora_nfc_read,
        // «Start on login»: desktop-only in effect — on mobile they answer an
        // error, and the settings toggle never renders there.
        autostart::autostart_is_enabled,
        autostart::autostart_enable,
        autostart::autostart_disable
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

/// Helpers the tests of several modules share.
#[cfg(test)]
mod test_support {
    use std::path::PathBuf;

    use crate::{hub_link, notice_tap};

    pub(crate) fn url(s: &str) -> tauri::Url {
        s.parse().expect("valid test url")
    }

    pub(crate) fn tempdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("erplora-shell-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("tempdir");
        dir
    }

    pub(crate) fn app_with_kept_tap() -> tauri::App<tauri::test::MockRuntime> {
        use tauri::Manager;
        let app = tauri::test::mock_builder()
            .plugin(hub_link::android_plugin())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app");
        app.manage(notice_tap::KeptNoticeTap::default());
        app
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::app_with_kept_tap;

    /// One of the app's source files without its tests: the code the wiring checks below read.
    fn code_of(source: &str) -> &str {
        source.split("\n#[cfg(test)]\n").next().unwrap_or_default()
    }

    /// The body of the item that starts with `start` in `code`, up to its closing brace.
    fn body_of<'a>(code: &'a str, start: &str) -> &'a str {
        code.split(start).nth(1).unwrap_or_default().split("\n}\n").next().unwrap_or_default()
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
        let shell = code_of(include_str!("lib.rs"));
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
    fn on_android_the_shell_writes_down_each_notice_and_tells_the_page_a_tap_waits() {
        // Android-only lines the desktop build never compiles: each one going missing leaves the tap
        // on a notice where it was, with the app open.
        let shell = code_of(include_str!("lib.rs"));
        let notify = body_of(code_of(include_str!("notices.rs")), "fn erplora_notify(");
        let remember = notify
            .find("remember_notice_target(id, path.as_deref(), |id, path| app.erplora_android().remember_notice(id, path))")
            .expect("erplora_notify does not write down the screen of the notice");
        let show = notify.find("notice_builder(&app,").expect("erplora_notify does not show the notice");
        assert!(remember < show, "the screen is written down after the notice is out: a quick tap would miss it");
        assert!(
            body_of(shell, "pub fn run() {").contains(".watch_notice_taps(move || notice_tap::tell_page(&"),
            "run() never hands Kotlin a way to tell the page a tap waits"
        );
    }

    #[test]
    fn the_app_wires_the_linked_hub_gate() {
        // The gate and the link are only as good as their call sites: each of these lines going
        // missing hands the device back to every page under erplora.com.
        let shell = code_of(include_str!("lib.rs"));
        let window_code = code_of(include_str!("window.rs"));
        assert!(
            body_of(shell, "pub fn run() {").contains(".invoke_handler(hub_link::guard(app_commands()))"),
            "run() hands Tauri the commands without the gate"
        );
        assert!(
            body_of(shell, "pub fn run() {").contains(".plugin(hub_link::android_plugin())"),
            "run() registers the Android plugin without the gate"
        );
        assert!(
            body_of(shell, "pub fn run() {").contains(".plugin(hub_link::notification_plugin())")
                && !shell.contains(".plugin(tauri_plugin_notification::init())"),
            "run() registers the notification plugin without the gate"
        );
        let window = body_of(window_code, "fn open_main_window(");
        assert!(window.contains("app.manage(hub_link)"), "the window opens with no link state");
        let navigation = window.split(".on_navigation(").nth(1).unwrap_or_default();
        assert!(navigation.contains(".follow(nav)"), "a navigation links a hub without asking who chose it");
        assert!(!navigation.contains("shell_capture_origin(nav)"), "a navigation links a hub without asking who chose it");
        let loaded = window.split(".on_page_load(").nth(1).unwrap_or_default();
        assert!(
            loaded.contains("PageLoadEvent::Finished") && loaded.contains(".landed("),
            "the hardware opens before the hub's page has finished loading: the page that sent the window there may still run"
        );
        assert!(body_of(window_code, "fn forget_hub(").contains("forget_linked_hub(&app)"), "«Change business» keeps the old hub linked");
        assert!(
            body_of(window_code, "fn spawn_hub_liveness_check(").contains("forget_linked_hub(&app)"),
            "a deleted hub stays linked"
        );
        assert!(
            body_of(shell, "pub fn run() {").contains("Some(target) => open_hub_from_link(&handle, &target)"),
            "a link with the app open navigates without linking its hub"
        );
        // The person is asked by the system's own dialog, set up before the window
        // exists, and a cold start by a link to another hub goes through that same question.
        let setup = body_of(shell, "pub fn run() {").split(".setup(|app| {").nth(1).unwrap_or_default();
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
}
