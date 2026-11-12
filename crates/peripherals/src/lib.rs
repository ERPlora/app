//! Reusable POS hardware logic: renders **ESC/POS** and delivers it over the transport the
//! `printer_id` names.
//!
//! The crate has no UI and no server of its own. The installable app calls it in-process through
//! `invoke`, so there is no local port and no pairing token.
//!
//! **Three transports**, all fed by the same renderer — only the way the bytes leave changes:
//!   - `network:{ip}:{port}` — ESC/POS over TCP, usually on port 9100. The only one with a queue
//!     and retries (`queue`).
//!   - `bluetooth:{mac}` — SPP, **Android only**: the shell hands the rendered bytes to the Kotlin
//!     transport. There is no module here: the transport lives in the plugin.
//!   - `usb:{queue}` — the OS RAW print queue, **desktop only** (`usb`). It is not a driver of
//!     ours: it is `lp -o raw`, and the manufacturer supplies the driver. That is what makes USB
//!     supportable without writing one driver per OS.
//!
//! An HID barcode scanner is handled by the OS or the browser as a keyboard, which is why there is
//! no `scanner` or `keyboard` module.
//!
//! Retrying is the job of `queue` alone: a second retry policy elsewhere would be one nobody
//! maintains.

pub mod discovery;
pub mod drawer;
pub mod escpos;
pub mod protocol;
pub mod queue;
pub mod registry;
pub mod usb;

#[cfg(test)]
mod test_support;

/// The standard network port for raw ESC/POS printing.
pub const ESCPOS_NETWORK_PORT: u16 = 9100;

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
    /// The stable word a screen branches on: screens match codes, never prose. The message is for
    /// the log; this is what survives the trip through `invoke` to a module that has to say, in the
    /// user's language, which of two opposite things went wrong — a typo in the address, or a
    /// printer that did not answer.
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
