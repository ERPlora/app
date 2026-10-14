//! Impresora de mentira para los tests del crate: un listener TCP local que acepta **una**
//! conexión, lee hasta EOF y deja los bytes disponibles para asertarlos.
//!
//! Vive aquí, y no en cada `mod tests`, porque tres módulos la necesitan por la misma razón —
//! `drawer` (kick), `queue` (envío con reintentos) y `registry` (sonda del watchdog) hablan todos
//! por el mismo socket ESC/POS. Es `#[cfg(test)]`: no entra en el binario.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;

use crate::discovery::NetworkTarget;

/// Listener local que modela una impresora de un solo trabajo: acepta una conexión, lee todo y se
/// cierra. Captura el contrato de cable real sin hardware.
pub(crate) struct MockPrinter {
    pub(crate) target: NetworkTarget,
    captured: Arc<tokio::sync::Mutex<Vec<u8>>>,
    accepted: Arc<AtomicBool>,
}

impl MockPrinter {
    /// Arranca el listener en un puerto efímero del loopback y spawnea el aceptador.
    pub(crate) async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let captured = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let accepted = Arc::new(AtomicBool::new(false));

        let cap = captured.clone();
        let acc = accepted.clone();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                acc.store(true, Ordering::SeqCst);
                let mut buf = Vec::new();
                let _ = sock.read_to_end(&mut buf).await;
                *cap.lock().await = buf;
            }
            // El listener se dropea aquí: el puerto deja de aceptar.
        });

        Self {
            target: NetworkTarget {
                host: addr.ip().to_string(),
                port: addr.port(),
            },
            captured,
            accepted,
        }
    }

    /// Espera (timeout corto) a que el aceptador haya leído hasta EOF y devuelve los bytes.
    pub(crate) async fn captured_bytes(&self) -> Vec<u8> {
        for _ in 0..200 {
            {
                let guard = self.captured.lock().await;
                if !guard.is_empty() {
                    return guard.clone();
                }
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        self.captured.lock().await.clone()
    }

    /// Si llegó a aceptar una conexión.
    pub(crate) fn accepted(&self) -> bool {
        self.accepted.load(Ordering::SeqCst)
    }
}

/// Un destino del loopback al que **nadie** escucha: modela la impresora apagada. El puerto 1 es
/// privilegiado y está cerrado en cualquier máquina de desarrollo o CI.
pub(crate) fn unreachable_target() -> NetworkTarget {
    NetworkTarget {
        host: "127.0.0.1".into(),
        port: 1,
    }
}
