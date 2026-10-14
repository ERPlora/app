//! Contrato JSON Hub ↔ Bridge sobre WebSocket — **fuente única de verdad** del protocolo.
//!
//! Espejo de `bridge/ERPlora-Bridge-desktop/erplora_bridge/protocol.py` y de `Protocol.kt`
//! (Android). `hub/static/js/bridge.js` consume este formato sin cambios.
//!
//! Cambios de superficie por la decisión red-only (§2.7) respecto al protocolo Python:
//!   - **Eliminadas** las acciones/eventos de escáner y teclado virtual: `toggle_keyboard`,
//!     evento `barcode`, evento `keyboard_toggled` (el escáner HID lo maneja el SO/navegador).
//!   - `printer_id` nació `network:{ip}:{port}` a secas. Hoy son tres: `bluetooth:{mac}` volvió
//!     con ADR-0204 (solo Android) y `usb:{queue}` con hub#1083 (solo escritorio).

use serde::{Deserialize, Serialize};

/// Mensaje Hub → Bridge. Discriminado por la clave `action`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Command {
    /// Versión del bridge + impresoras cacheadas.
    GetStatus,
    /// Re-escanea la red en busca de impresoras.
    DiscoverPrinters,
    /// Imprime un documento.
    Print {
        printer_id: String,
        document_type: String,
        data: serde_json::Value,
        #[serde(default)]
        job_id: Option<String>,
    },
    /// Abre el cajón vía kick ESC/POS por el socket de la impresora.
    OpenDrawer {
        printer_id: String,
        #[serde(default = "default_drawer_pin")]
        pin: u8,
    },
    /// Página de prueba.
    TestPrint { printer_id: String },
    /// Notificación a nivel de SO.
    SendNotification { title: String, body: String },
    /// Contenido del registro de dispositivos.
    GetDevices,
    /// Asigna rol a un dispositivo (`receipt` | `kitchen` | `bar` | `label`).
    SetDeviceRole { mac: String, role: String },
    /// Renombra un dispositivo.
    SetDeviceName { mac: String, name: String },
    /// Elimina un dispositivo del registro.
    RemoveDevice { mac: String },
}

/// Mensaje Bridge → Hub. Discriminado por la clave `event`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Status {
        version: String,
        printers: Vec<PrinterInfo>,
        /// Siempre `false` en red-only; se conserva por compatibilidad con `bridge.js`.
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

/// Info estandarizada de una impresora (espejo de `printer_info()` en Python).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct PrinterInfo {
    /// `network:{ip}:{port}`, `bluetooth:{mac}` (ADR-0204) o `usb:{queue}` (hub#1083).
    pub id: String,
    pub name: String,
    /// Transporte: `"network"` | `"bluetooth"` | `"usb"` — el esquema de `id`, en un campo aparte
    /// para que una pantalla pueda agrupar por cable sin parsear el id.
    #[serde(rename = "type")]
    pub kind: String,
    /// Familia de la impresora: `"a4"` | `"unknown"`.
    ///
    /// El puerto 9100 es un tubo tonto: una térmica y una láser A4 de oficina **escuchan las dos
    /// ahí**, pero hablan idiomas distintos (ESC/POS vs PCL/PostScript). Mandarle ESC/POS a una A4
    /// escupe folios de basura, así que hay que distinguirlas.
    ///
    /// Lo único que lo delata con certeza es el anuncio mDNS **`_ipp._tcp`**: lo publican las de
    /// oficina y las AirPrint, y prácticamente ninguna térmica ESC/POS. Lo demás —incluido lo que
    /// solo aparece en el escaneo del 9100— se queda en `"unknown"`: **no se adivina**.
    #[serde(default = "default_printer_category")]
    pub category: String,
    /// `ready` | `busy` | `error` | `offline`.
    pub status: String,
    pub paper_width: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mac: Option<String>,
}

/// Familia por defecto: desconocida. Nunca se asume "térmica" sin evidencia.
pub fn default_printer_category() -> String {
    PRINTER_CATEGORY_UNKNOWN.to_string()
}

/// Impresora de oficina / A4: anuncia IPP. No admite ESC/POS crudo.
pub const PRINTER_CATEGORY_A4: &str = "a4";
/// No hay evidencia suficiente para clasificarla (p. ej. solo responde al 9100).
pub const PRINTER_CATEGORY_UNKNOWN: &str = "unknown";

/// Entrada del registro persistente de dispositivos (espejo del dict en `network.py`).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Device {
    /// Identidad estable del dispositivo en el registro: la MAC normalizada cuando se conoce y,
    /// si no, el `printer_id` (`network:{ip}:{port}`).
    ///
    /// La MAC no siempre está disponible: en Android **nunca** lo está (no existe el binario `arp`
    /// y `/proc/net/arp` está restringido desde Android 10), y en escritorio falla con VPN,
    /// contenedores o firewall. Antes de existir este campo, un dispositivo sin MAC no llegaba a
    /// entrar en el registro, así que `get_devices` devolvía `[]` y **no se le podía asignar rol**
    /// (cocina/barra/caja) — lo que dejaba el módulo `printing` inservible.
    ///
    /// `#[serde(default)]`: los `devices.json` escritos antes de esta versión no lo traen; al
    /// cargarlos se rellena con la clave del mapa (que era la MAC).
    #[serde(default)]
    pub key: String,
    /// MAC real, solo si el sistema pudo resolverla por ARP. `None` no impide operar: para
    /// identificar el dispositivo está `key`. Lo único que exige MAC es la recuperación tras un
    /// cambio de IP por DHCP, que sin ella es imposible por definición.
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
    /// The owner typed its address by hand (hub#1924) instead of the scan finding it. A typed
    /// printer usually sits where the scan cannot look — another subnet, an isolated Wi-Fi — so it
    /// is listed from the registry on every discovery, or it would vanish at the next «Re-scan».
    /// Scanned printers are not, so a printer found once and gone now does not linger forever.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub manual: bool,
}

fn default_drawer_pin() -> u8 {
    2
}
