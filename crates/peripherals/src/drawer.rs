//! Cajón portamonedas: comando kick ESC/POS enviado por el socket TCP de la impresora.
//! Porta `bridge/ERPlora-Bridge-desktop/erplora_bridge/hardware/drawer.py`.

use crate::discovery::NetworkTarget;
use crate::Result;

/// `ESC p 0 25 50` — pulso por el pin 2 del conector DK (por defecto).
pub const KICK_PIN_2: &[u8] = &[0x1b, 0x70, 0x00, 0x19, 0x32];

/// `ESC p 1 25 50` — pulso por el pin 5.
pub const KICK_PIN_5: &[u8] = &[0x1b, 0x70, 0x01, 0x19, 0x32];

/// Bytes del kick según el pin (2 → pin 2; cualquier otro → pin 5, igual que Python).
pub fn kick_command(pin: u8) -> &'static [u8] {
    if pin == 2 {
        KICK_PIN_2
    } else {
        KICK_PIN_5
    }
}

/// Abre el cajón conectado a la impresora: abre socket al `target` y envía el kick.
/// Porta `open_drawer(printer_id, pin)`. Es la vía que usa `erplora_open_drawer` (Tauri).
pub async fn open_drawer(target: &NetworkTarget, pin: u8) -> Result<()> {
    let stream = tokio::net::TcpStream::connect(target.socket_addr()).await?;
    let mut buf = kick_command(pin);
    while !buf.is_empty() {
        stream.writable().await?;
        match stream.try_write(buf) {
            Ok(n) => buf = &buf[n..],
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{unreachable_target, MockPrinter};

    /// **El pin correcto en el cable, no solo en la constante.** El cajón es un relé del conector
    /// DK de la impresora: si sale el pulso del otro pin, el cajón simplemente **no se abre** y no
    /// hay ningún error que lo diga — el cajero se queda con el dinero en la mano.
    #[tokio::test]
    async fn opening_the_drawer_puts_the_pin_2_kick_on_the_wire() {
        let mock = MockPrinter::start().await;

        open_drawer(&mock.target, 2)
            .await
            .expect("el kick se envía");

        assert_eq!(
            mock.captured_bytes().await,
            KICK_PIN_2,
            "pin 2 debe poner `ESC p 0 25 50` en el socket de la impresora"
        );
        assert!(mock.accepted(), "se abrió la conexión al destino");
    }

    /// El otro cableado del mismo conector. Los dos pines existen porque el fabricante del cajón
    /// elige uno: mandar el del pin 2 a un cajón cableado al 5 es exactamente el fallo silencioso
    /// de arriba.
    #[tokio::test]
    async fn a_drawer_wired_to_pin_5_gets_the_pin_5_kick() {
        let mock = MockPrinter::start().await;

        open_drawer(&mock.target, 5)
            .await
            .expect("el kick se envía");

        assert_eq!(mock.captured_bytes().await, KICK_PIN_5);
    }

    /// Cualquier pin que no sea el 2 es el 5, igual que en Python (`if pin == 2 … else`). Se fija
    /// aquí para que un `pin: Option<u8>` inesperado desde la UI no cambie el cable en silencio.
    #[test]
    fn any_pin_other_than_2_is_the_pin_5_kick() {
        assert_eq!(kick_command(2), KICK_PIN_2);
        for other in [0u8, 1, 3, 5, 255] {
            assert_eq!(kick_command(other), KICK_PIN_5, "pin {other}");
        }
    }

    /// **Una impresora apagada da error, no un `Ok` mudo.** `erplora_open_drawer` propaga este
    /// `Result` a la UI; si se tragara el fallo, la pantalla diría "cajón abierto" con el cajón
    /// cerrado.
    #[tokio::test]
    async fn an_unreachable_printer_fails_instead_of_reporting_success() {
        let err = open_drawer(&unreachable_target(), 2)
            .await
            .expect_err("un destino cerrado no puede abrir ningún cajón");
        assert!(
            matches!(err, crate::PeripheralError::Io(_)),
            "el fallo es de socket, y se dice: {err}"
        );
    }
}
