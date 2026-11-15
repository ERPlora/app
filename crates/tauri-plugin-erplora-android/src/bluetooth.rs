//! Bluetooth Classic printing: the bonded printers and the ESC/POS bytes sent to one of them.

use serde::{Deserialize, Serialize};
use tauri::Runtime;

#[cfg(target_os = "android")]
use crate::Empty;
use crate::{Error, ErploraAndroid};

/// The code a bluetooth operation is refused under when `BLUETOOTH_CONNECT` is denied.
/// Mirror of `ErploraAndroidPlugin.BLUETOOTH_PERMISSION_DENIED` — same trip as
/// [`DOWNLOADS_UNREACHABLE`](crate::DOWNLOADS_UNREACHABLE): Kotlin rejects with a code, Tauri renders `[code] - message`, and
/// the shell recognises it out of the text.
pub const BLUETOOTH_PERMISSION_DENIED: &str = "bluetooth_permission_denied";

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
/// its shape — reading it as [`Empty`](crate::Empty) reported a ticket already on paper as `failed`.
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

impl<R: Runtime> ErploraAndroid<R> {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    // ── Bluetooth Classic SPP ────────────────────────────────────────────────────────────────
    //
    // The transport lives in Kotlin (`BluetoothSpp.kt`); what Rust owns is the CONTRACT across
    // the JNI boundary — command names, argument keys, the shape of the answer, and the refusal
    // code. Every one of those is a bare literal on both sides, so each gets a drift guard: a
    // typo in either one is invisible until a real till stops finding its bluetooth printer.

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
        // Only Android produces or accepts `bluetooth:{mac}`. On desktop the bonded list is
        // honestly empty and a bluetooth print is an ERROR, never a silent no-op: a resolved print
        // that put no paper out would tell the till the ticket was printed.
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

    #[test]
    fn a_bluetooth_answer_with_no_data_is_a_ticket_on_paper() {
        for answer in [serde_json::Value::Null, serde_json::json!({})] {
            assert!(
                serde_json::from_value::<BluetoothPrintAnswer>(answer.clone()).is_ok(),
                "{answer} must read as a printed ticket"
            );
        }
    }
}
