//! `erplora-peripherals` — lógica de hardware POS reutilizable. Renderiza **ESC/POS** y lo
//! entrega por el transporte que nombre el `printer_id`. ARQUITECTURA.md §2.7.
//!
//! Este crate concentra lo que era el Bridge Python (`bridge/ERPlora-Bridge-desktop`) en una
//! librería sin I/O de UI ni servidor WebSocket — el consumidor monta su propio transporte. Desde
//! ADR-0196 (ejecutado en hub#340) queda **uno solo**: la app instalable `apps/tauri`, que llama a
//! este crate **in-process** por `invoke`. El standalone `apps/bridge`, que lo servía por un
//! WebSocket en `localhost:12321`, ya no existe: no hay puerto local, ni token de emparejamiento.
//!
//! **Tres transportes**, y el renderizado es el mismo para los tres — lo único que cambia es por
//! dónde salen los bytes:
//!   - `network:{ip}:{port}` — ESC/POS por TCP al 9100. La vía original y la única con cola y
//!     reintentos (`queue`).
//!   - `bluetooth:{mac}` — SPP, **solo Android**: el shell pasa los bytes ya renderizados al
//!     transporte Kotlin (ADR-0204, hub#388). Aquí no hay módulo: el transporte vive en el plugin.
//!   - `usb:{queue}` — la cola RAW del SO, **solo escritorio** (`usb`, hub#1083). No es un driver
//!     nuestro: es `lp -o raw`, y el driver lo pone el fabricante. Eso es lo que permite soportar
//!     USB sin caer en el «un driver por SO no escala» que protegía la decisión red-only.
//!
//! El escáner por HID lo sigue manejando el SO/navegador como teclado: por eso **no** hay
//! `scanner` ni `keyboard`.
//!
//! Estado: **implementado** — render ESC/POS (`escpos`), descubrimiento (`discovery`), registro +
//! watchdog (`registry`), cajón (`drawer`), cola con reintentos (`queue`) y cola RAW del SO
//! (`usb`). El mapeo Python→Rust por módulo está en `README.md`.
//!
//! Hubo además un `printer` con una capa de traits (`Printer`/`CashDrawer`) y **su propia**
//! política de reintentos: nunca tuvo consumidor —la app encola por `queue` y abre el cajón por
//! `drawer`— y dos políticas de reintento coexistiendo son una que nadie mantiene. hub#379 la
//! borró y pasó su cobertura a las vías vivas.

pub mod discovery;
pub mod drawer;
pub mod escpos;
pub mod protocol;
pub mod queue;
pub mod registry;
pub mod usb;

#[cfg(test)]
mod test_support;

/// Puerto estándar ESC/POS por red (raw printing).
pub const ESCPOS_NETWORK_PORT: u16 = 9100;

/// Error común de la capa de periféricos.
#[derive(Debug, thiserror::Error)]
pub enum PeripheralError {
    #[error("printer id inválido: {0}")]
    InvalidPrinterId(String),
    #[error("impresora inalcanzable: {0}")]
    Unreachable(String),
    #[error("tipo de documento desconocido: {0}")]
    UnknownDocumentType(String),
    #[error("payload inválido: {0}")]
    InvalidPayload(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl PeripheralError {
    /// The stable word a screen branches on (ADR-0055: codes, never prose). The message is for the
    /// log; this is what survives the trip through `invoke` to a module that has to say, in the
    /// user's language, which of two opposite things went wrong — a typo in the address, or a
    /// printer that did not answer (hub#1924).
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidPrinterId(_) => "invalid_printer_address",
            Self::Unreachable(_) => "printer_unreachable",
            Self::UnknownDocumentType(_) => "unknown_document_type",
            Self::InvalidPayload(_) => "invalid_payload",
            Self::Io(_) => "io_error",
            Self::Json(_) => "invalid_payload",
        }
    }
}

pub type Result<T> = std::result::Result<T, PeripheralError>;
