//! Hardware path: `invoke` handlers → erplora-peripherals.
//!
//! The shell IS the bridge (there is no separate bridge process): the hardware is exposed through
//! `invoke` handlers that delegate to `erplora-peripherals`. The data contract matches the SDK:
//! `getDevices` returns the array of `protocol::Device` and `discoverPrinters` the
//! `PrinterDiscovery` outcome (serialized by serde exactly like the SDK's
//! `BridgeDevice`/`PrinterDiscovery`); `print`/`testPrint`/`openDrawer` return nothing.
//!
//! The registry's `Watchdog` runs as an async task of the shell (it recovers a device whose IP DHCP
//! changed), and the `PrintQueue` drains with retries in the background. Job outcomes and watchdog
//! events are only logged; nothing forwards them to the UI.

use std::path::PathBuf;

use serde::Serialize;

use erplora_peripherals::protocol::Device;
use erplora_peripherals::queue::{JobOutcome, PrintQueue, RetryPolicy};
use erplora_peripherals::registry::DeviceRegistry;

/// Hardware state shared by the `invoke` handlers: the persistent device registry plus the print
/// queue with retries. It lives in Tauri's managed state (`app.manage`). Registry and queue sit
/// behind `Arc` so that the background tasks (watchdog and queue worker) share the same instances
/// as the `invoke` handlers.
pub(crate) struct PeripheralsState {
    pub(crate) registry: std::sync::Arc<DeviceRegistry>,
    pub(crate) queue: std::sync::Arc<PrintQueue>,
}

/// Builds the hardware state and **spawns** the background tasks on the current tokio runtime:
///   - the `PrintQueue` worker (sending with retries; every `JobOutcome` is logged),
///   - the registry's `Watchdog` (health check plus recovery by MAC when DHCP changes an IP).
pub(crate) fn build_peripherals_state(devices_path: PathBuf) -> PeripheralsState {
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
        use erplora_peripherals::registry::watchdog::{Watchdog, WatchdogConfig, WatchdogEvent};
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

/// Error of the hardware handlers. The page gets only its stable code (`code`), which the SDK turns
/// into a rejected promise: the device that prints for the hub reports it as the job's last error,
/// and the Printing screen says it in the reader's language. The words, which name the system's
/// error and the printer's address, go to the device's log as the error leaves.
#[derive(Debug, thiserror::Error)]
pub enum HardwareError {
    #[error("{0}")]
    Peripheral(#[from] erplora_peripherals::PeripheralError),
    /// Android refused `BLUETOOTH_CONNECT`: the person has to allow it; the printer is fine.
    #[error("{0}")]
    BluetoothPermissionDenied(String),
}

impl HardwareError {
    /// The stable word a screen branches on (`PeripheralError::code`).
    pub fn code(&self) -> &'static str {
        match self {
            Self::Peripheral(e) => e.code(),
            Self::BluetoothPermissionDenied(_) => {
                tauri_plugin_erplora_android::BLUETOOTH_PERMISSION_DENIED
            }
        }
    }
}

impl Serialize for HardwareError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        eprintln!("peripherals: {self}");
        s.serialize_str(self.code())
    }
}

/// Refusal of `erplora_add_network_printer`: the stable `code` the page branches on next to the
/// message, which names the address that was typed. The two answers this command can give send
/// the owner to opposite places — fix the typed address, or go check the printer.
#[derive(Debug, Serialize)]
pub(crate) struct AddPrinterError {
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
pub(crate) fn erplora_bridge_status() -> serde_json::Value {
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

/// `erplora_get_devices` — the contents of the persistent device registry (with their roles).
#[tauri::command]
pub(crate) fn erplora_get_devices(state: tauri::State<'_, PeripheralsState>) -> Vec<Device> {
    state.registry.get_all()
}

/// `erplora_set_device_role` — assigns a role (receipt/kitchen/bar/label) and returns the updated
/// registry. Mirrors the bridge's `Command::SetDeviceRole`.
#[tauri::command]
pub(crate) fn erplora_set_device_role(
    state: tauri::State<'_, PeripheralsState>,
    mac: String,
    role: String,
) -> Result<Vec<Device>, HardwareError> {
    state.registry.set_role(&mac, &role)?;
    Ok(state.registry.get_all())
}

/// `erplora_set_device_name` — renames a device and returns the updated registry.
#[tauri::command]
pub(crate) fn erplora_set_device_name(
    state: tauri::State<'_, PeripheralsState>,
    mac: String,
    name: String,
) -> Result<Vec<Device>, HardwareError> {
    state.registry.set_name(&mac, &name)?;
    Ok(state.registry.get_all())
}

/// `erplora_remove_device` — removes a device from the registry and returns the updated registry.
#[tauri::command]
pub(crate) fn erplora_remove_device(
    state: tauri::State<'_, PeripheralsState>,
    mac: String,
) -> Result<Vec<Device>, HardwareError> {
    state.registry.remove(&mac)?;
    Ok(state.registry.get_all())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Adding a printer by typing its address ───────────────────────────────────────────────────
    //
    // The screen has to tell a typo from a printer that did not answer, in the user's language, so
    // this command's refusal carries the stable code next to the message.

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

    // ── A print that fails reaches the page as its code ─────────────────────────────────────────
    //
    // The device that prints for the hub reports this answer as the job's last error, and the
    // Printing screen says it in the reader's language. The system's words («Connection refused
    // (os error 111)») tell whoever runs the shop nothing; they go to the device's log.

    #[test]
    fn a_failed_print_reaches_the_page_as_its_code_not_the_systems_words() {
        use erplora_peripherals::PeripheralError;
        let cases = [
            (
                PeripheralError::Unreachable("192.168.1.50:9100: Connection refused (os error 111)".into()),
                "printer_unreachable",
            ),
            (PeripheralError::InvalidPrinterId("not an IPv4 address".into()), "invalid_printer_address"),
            (PeripheralError::UnknownDocumentType("menu".into()), "unknown_document_type"),
            (PeripheralError::InvalidPayload("no lines".into()), "invalid_payload"),
            (
                PeripheralError::Io(std::io::Error::from(std::io::ErrorKind::ConnectionRefused)),
                "io_error",
            ),
        ];
        for (error, code) in cases {
            let wire = serde_json::to_value(HardwareError::from(error)).expect("serializes");
            assert_eq!(wire, serde_json::json!(code));
        }
    }

    #[test]
    fn a_bluetooth_permission_refusal_has_its_own_code() {
        let denied = HardwareError::BluetoothPermissionDenied("BLUETOOTH_CONNECT".into());
        assert_eq!(
            serde_json::to_value(&denied).expect("serializes"),
            serde_json::json!("bluetooth_permission_denied")
        );
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
}
