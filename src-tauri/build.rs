use std::path::Path;

/// The application manifest every executable of this crate needs on Windows: the
/// dependency on Common Controls v6, the same content as tauri-build's default.
const WINDOWS_MANIFEST: &str = "windows-app-manifest.xml";

fn main() {
    // tauri-build embeds its default manifest through tauri-winres with `rustc-link-arg-bins`:
    // the app binary gets it, the test harnesses do not. Tauri's menu crate (`muda`) imports
    // `TaskDialogIndirect` from comctl32.dll, which only v6 exports, so without the manifest every
    // test executable fails to load (`0xc0000139 STATUS_ENTRYPOINT_NOT_FOUND`). Tauri's documented
    // fix: turn its manifest off and embed the same one with `rustc-link-arg`, which reaches every
    // target of the package. MSVC linker flags, so only for the MSVC toolchain on Windows.
    // Guarded by `tests/windows_test_manifest.rs`.
    let windows_msvc = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let mut windows = tauri_build::WindowsAttributes::new();
    if windows_msvc {
        windows = tauri_build::WindowsAttributes::new_without_app_manifest();
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join(WINDOWS_MANIFEST);
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }

    // The shell is a thin client: the window loads REMOTE ORIGINS (the SaaS for the onboarding and
    // the hub PWA at `https://<sub>.erplora.com`). Tauri's ACL only lets a remote origin `invoke` a
    // command that has an `allow-<cmd>` permission granted by a capability with `remote.urls`.
    // Without `AppManifest::commands(...)` those permissions do NOT exist and the ACL refuses EVERY
    // app command in the bundle (dev hides it: a local origin with no app manifest skips the gate).
    // The commands are declared here so that `tauri-build` generates `allow-<cmd>`; the
    // capabilities grant them. Keep in sync with `generate_handler!` in `lib.rs`
    // (`tests/shell_surface.rs` checks it).
    let attributes = tauri_build::Attributes::new().windows_attributes(windows);
    tauri_build::try_build(
        attributes.app_manifest(tauri_build::AppManifest::new().commands(&[
            // Device identity (X-Device-Id, one session per device) and forgetting the hub.
            "device_context",
            "forget_hub",
            // The way OUT to the user's own browser: the SaaS checkout, the plans page, the
            // billing portal. `window.open` opens nothing inside the webview.
            "open_external_url",
            // The other half of the same trip: bytes the page already holds, written where the
            // user will find them, with the path returned so the app can SAY so.
            "save_download",
            // The system print dialog for an A4 document the page holds: a laser printer or
            // «Save as PDF», which the webview's own `window.print()` cannot reach.
            "print_document",
            // The way out when the network dies under the window: the retry control of the
            // bundled offline page. Granted to that page ONLY (`capabilities/degraded.json`).
            "shell_retry",
            // Hardware (the shell IS the bridge).
            "erplora_bridge_status",
            "erplora_discover_printers",
            "erplora_get_devices",
            "erplora_print",
            "erplora_test_print",
            "erplora_open_drawer",
            "erplora_set_device_role",
            // Adding a printer by its typed IP when the scan cannot see it.
            "erplora_add_network_printer",
            "erplora_set_device_name",
            "erplora_remove_device",
            // OS notification: the notice for when NOBODY is watching the screen (kitchen orders).
            "erplora_notify",
            // The tap the page was not there to hear: a click on the computer, or the tap that
            // started the app on Android. The page claims it once.
            "erplora_take_notice_tap",
            // The badge through the device's own NFC reader: the second origin of the SAME door
            // as the keyboard-wedge reader.
            "erplora_nfc_read",
            // «Start on login». App commands and not the autostart
            // plugin's own (`autostart:allow-*`): the plugin is a DESKTOP-ONLY dependency, so on
            // an Android build its permissions do not exist and a capability naming them would
            // fail the build. These three exist on every platform — on mobile they answer an
            // error, and the settings toggle never renders there.
            "autostart_is_enabled",
            "autostart_enable",
            "autostart_disable",
        ])),
    )
    .expect("tauri-build failed (app manifest / capabilities)");
}
