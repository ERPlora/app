use std::path::Path;

/// The application manifest every executable of this crate needs on Windows (hub#2705): the
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

    // ADR-0159 (cliente fino): la ventana carga ORÍGENES REMOTOS (el SaaS para el onboarding y la
    // PWA del hub cloud en `https://<sub>.erplora.com`). El ACL de Tauri solo permite un `invoke`
    // desde un origen remoto si el comando tiene un permiso `allow-<cmd>` concedido por una
    // capability con `remote.urls`. Sin un `AppManifest::commands(...)`, esos permisos NO existen y
    // el ACL rechaza TODOS los comandos de la app en el bundle (dev lo enmascara: origen local sin
    // app-manifest desactiva el gate). Declaramos aquí los comandos para que `tauri-build`
    // autogenere `allow-<cmd>`; la capability los concede. Mantener en sync con
    // `generate_handler![...]` en `lib.rs`.
    let attributes = tauri_build::Attributes::new().windows_attributes(windows);
    tauri_build::try_build(
        attributes.app_manifest(tauri_build::AppManifest::new().commands(&[
            // Identidad de dispositivo (X-Device-Id, sesión única ADR-0154) + olvido del hub.
            "device_context",
            "forget_hub",
            // The way OUT to the user's own browser: the SaaS checkout, the plans page, the
            // billing portal (hub#475). `window.open` opens nothing inside the webview.
            "open_external_url",
            // The other half of the same trip: bytes the page already holds, written where the
            // user will find them, with the path returned so the app can SAY so (hub#480).
            "save_download",
            // The system print dialog for an A4 document the page holds (hub#2006): a laser printer
            // or «Save as PDF», which the webview's own `window.print()` cannot reach.
            "print_document",
            // The way out when the network dies under the window (hub#1716): the retry control of
            // the bundled offline page. Granted to that page ONLY (`capabilities/degraded.json`).
            "shell_retry",
            // Hardware (el shell ES el bridge, §2.7).
            "erplora_bridge_status",
            "erplora_discover_printers",
            "erplora_get_devices",
            "erplora_print",
            "erplora_test_print",
            "erplora_open_drawer",
            "erplora_set_device_role",
            // Alta por IP tecleada cuando el escaneo no la ve (hub#1924).
            "erplora_add_network_printer",
            "erplora_set_device_name",
            "erplora_remove_device",
            // Notificación del SO: el aviso cuando NADIE mira la pantalla (comanda a cocina).
            "erplora_notify",
            // The tap the page was not there to hear (hub#2360): a click on the computer, or the
            // tap that started the app on Android. The page claims it once.
            "erplora_take_notice_tap",
            // La placa por el lector NFC del propio aparato (hub#988): el segundo origen de la
            // MISMA puerta que el lector-teclado.
            "erplora_nfc_read",
            // «Start on login» (ADR-0204 §7, hub#389). App commands and not the autostart
            // plugin's own (`autostart:allow-*`): the plugin is a DESKTOP-ONLY dependency, so on
            // an Android build its permissions do not exist and a capability naming them would
            // fail the build. These three exist on every platform — on mobile they answer an
            // error, and the settings toggle never renders there.
            "autostart_is_enabled",
            "autostart_enable",
            "autostart_disable",
        ])),
    )
    .expect("error en tauri-build (app manifest / capabilities)");
}
