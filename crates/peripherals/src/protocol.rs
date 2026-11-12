//! The JSON shapes the app exchanges with the Hub's web page — the **single source of truth** for
//! them.
//!
//! The `invoke` handlers return the printer and device records below serialized by serde, and the
//! Hub reads the field names exactly as they are here: renaming a field or a tag breaks the page.
//!
//! There are no scanner or virtual keyboard messages: an HID scanner is handled by the OS or the
//! browser as a keyboard.

use serde::{Deserialize, Serialize};

/// A Hub → app message, tagged by its `action` key.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Command {
    /// The app version and the printers it has cached.
    GetStatus,
    /// Scans the network for printers again.
    DiscoverPrinters,
    Print {
        printer_id: String,
        document_type: String,
        data: serde_json::Value,
        #[serde(default)]
        job_id: Option<String>,
    },
    /// Opens the cash drawer with an ESC/POS kick sent through the printer's socket.
    OpenDrawer {
        printer_id: String,
        #[serde(default = "default_drawer_pin")]
        pin: u8,
    },
    /// Prints a test page.
    TestPrint { printer_id: String },
    /// An OS-level notification.
    SendNotification { title: String, body: String },
    /// The contents of the device registry.
    GetDevices,
    /// Gives a device a role (`receipt` | `kitchen` | `bar` | `label`).
    SetDeviceRole { mac: String, role: String },
    SetDeviceName { mac: String, name: String },
    /// Removes a device from the registry.
    RemoveDevice { mac: String },
}

/// An app → Hub message, tagged by its `event` key.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Status {
        version: String,
        printers: Vec<PrinterInfo>,
        /// Always `false`: the scanner is not handled here. The field stays so the message keeps
        /// its shape.
        scanner: bool,
    },
    Printers { printers: Vec<PrinterInfo> },
    Devices { devices: Vec<Device> },
    DeviceRecovered { device: Device },
    DeviceLost { device: Device },
    PrintComplete { job_id: Option<String> },
    PrintError { job_id: Option<String>, error: String },
    DrawerOpened { printer_id: String },
    Error { message: String, code: String },
}

/// A printer, in the same shape whatever transport found it.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct PrinterInfo {
    /// `network:{ip}:{port}`, `bluetooth:{mac}` or `usb:{queue}`.
    pub id: String,
    pub name: String,
    /// Transport: `"network"` | `"bluetooth"` | `"usb"` — the scheme of `id`, in a field of its own
    /// so a screen can group by connection without parsing the id.
    #[serde(rename = "type")]
    pub kind: String,
    /// Printer family: `"a4"` | `"unknown"`.
    ///
    /// Port 9100 is a dumb pipe: a thermal printer and an office A4 laser **both listen there**,
    /// but they speak different languages (ESC/POS vs PCL/PostScript). Sending ESC/POS to an A4
    /// printer spits out pages of garbage, so the two have to be told apart.
    ///
    /// The only sure sign is the mDNS **`_ipp._tcp`** announcement: office and AirPrint printers
    /// publish it, and practically no ESC/POS thermal printer does. Everything else — including
    /// what only shows up in the 9100 scan — stays `"unknown"`: **it is never guessed**.
    #[serde(default = "default_printer_category")]
    pub category: String,
    /// `ready` | `busy` | `error` | `offline`.
    pub status: String,
    pub paper_width: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mac: Option<String>,
}

/// The default family is unknown: a printer is never assumed to be thermal without evidence.
pub fn default_printer_category() -> String {
    PRINTER_CATEGORY_UNKNOWN.to_string()
}

/// An office / A4 printer: it advertises IPP and does not accept raw ESC/POS.
pub const PRINTER_CATEGORY_A4: &str = "a4";
/// Not enough evidence to classify it (e.g. it only answers on 9100).
pub const PRINTER_CATEGORY_UNKNOWN: &str = "unknown";

/// An entry of the persisted device registry.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Device {
    /// The device's stable identity in the registry: the normalized MAC when it is known and,
    /// otherwise, the `printer_id` (`network:{ip}:{port}`, or `usb:{queue}` for an OS queue).
    ///
    /// The MAC is not always available: on Android it **never** is (there is no `arp` binary and
    /// `/proc/net/arp` is restricted since Android 10), and on desktop it fails behind a VPN,
    /// containers or a firewall. Keyed by MAC alone, a device without one could not enter the
    /// registry, `get_devices` would return `[]` and **it could not be given a role**
    /// (kitchen/bar/till) — which would leave the `printing` module useless.
    ///
    /// `#[serde(default)]`: older `devices.json` files do not carry it; on load it is filled with
    /// the map key, which in those files is the MAC.
    #[serde(default)]
    pub key: String,
    /// The real MAC, only when the system could resolve it through ARP. `None` does not stop
    /// anything: `key` identifies the device. The only thing that needs the MAC is recovering
    /// after a DHCP change of IP, which is impossible without it by definition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac: Option<String>,
    pub ip: String,
    pub port: u16,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    pub first_seen: String,
    pub last_seen: String,
    /// `online` | `offline`.
    pub status: String,
    /// The owner typed its address by hand instead of the scan finding it. A typed
    /// printer usually sits where the scan cannot look — another subnet, an isolated Wi-Fi — so it
    /// is listed from the registry on every discovery, or it would vanish at the next «Re-scan».
    /// Scanned printers are not, so a printer found once and gone now does not linger forever.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub manual: bool,
}

fn default_drawer_pin() -> u8 {
    2
}
