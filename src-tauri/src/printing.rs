//! Printing on the hardware: a document, a test page or the cash drawer's kick, to a network,
//! Bluetooth or USB printer.

use erplora_peripherals::print_target;
use erplora_peripherals::drawer;
use erplora_peripherals::escpos::{self, DocumentType};
use erplora_peripherals::queue::{PrintJob, PrintQueue};

use crate::hardware::{HardwareError, PeripheralsState};

/// Sends already-rendered bytes to a bonded SPP printer through the Android plugin.
///
/// Direct, not queued: the `PrintQueue` is the NETWORK path (its jobs carry a socket target).
/// Bluetooth gets the simple failure mode on purpose — the error comes straight back to the caller
/// (the print host reports `failed`), instead of retrying against a printer whose only health
/// signal IS the connect.
fn bluetooth_send(
    app: &tauri::AppHandle,
    mac: &str,
    payload: &[u8],
) -> Result<(), HardwareError> {
    use tauri_plugin_erplora_android::ErploraAndroidExt;
    app.erplora_android()
        .bluetooth_print(mac, payload)
        .map_err(|e| {
            HardwareError::from(erplora_peripherals::PeripheralError::Unreachable(
                e.to_string(),
            ))
        })
}

/// Sends already-rendered bytes to the printer behind an OS print queue.
///
/// Direct, not queued — the same shape as `bluetooth_send`, and the reasoning is in
/// `erplora_peripherals::usb::send_raw`: [`PrintQueue`] is the NETWORK path, its jobs carry a
/// socket target, and widening it is a real change to the one piece of the print chain that
/// already works. The failure stays visible: the error comes back to the caller and the print host
/// reports the job `failed`.
#[cfg(not(target_os = "android"))]
fn usb_send(target: &print_target::UsbTarget, payload: &[u8]) -> Result<(), HardwareError> {
    erplora_peripherals::usb::send_raw(&erplora_peripherals::usb::SystemCups, target, payload)?;
    Ok(())
}

/// Android has no OS print queue to hand bytes to, and USB Host there is left out on purpose: the
/// Bluetooth SPP transport already covers the cheap printer on a tablet.
///
/// This says so instead of quietly doing nothing. A hub configured on the desktop till and then
/// opened on a tablet keeps the same `printer_id`, so this IS reachable — and a ticket that
/// vanishes without a word is exactly the failure mode the print chain keeps being bitten by.
#[cfg(target_os = "android")]
fn usb_send(target: &print_target::UsbTarget, _payload: &[u8]) -> Result<(), HardwareError> {
    Err(HardwareError::from(
        erplora_peripherals::PeripheralError::Unreachable(format!(
            "`usb:{}` is a printer on a desktop till's print queue; this device cannot reach it \
             (use a network or a bonded Bluetooth printer here)",
            target.queue
        )),
    ))
}

/// Sends a document to a network printer and answers with what happened to the paper: `Ok` once
/// it is on the printer, the unreachable error once the queue's retries ran out. Answering on the
/// hand-over to the in-memory queue instead would let a switched-off printer lose the ticket with
/// nobody told — the till would hear «done», and so would the hub for a job from its queue.
async fn print_to_network(
    queue: &PrintQueue,
    target: print_target::NetworkTarget,
    payload: Vec<u8>,
    job_id: Option<String>,
) -> Result<(), HardwareError> {
    queue.print(PrintJob { job_id, target, payload, attempts: 0 }).await?;
    Ok(())
}

/// `erplora_print` — renders the ESC/POS document and sends it (same command as the bridge's
/// `Command::Print`): through the [`PrintQueue`] and its retries for a network printer, directly
/// for a Bluetooth or USB one. Every failure, a bad printer_id or payload included, comes back to
/// the caller.
///
/// **An unknown `document_type` is refused, not printed as `Generic`**. This is the end of the
/// chain: the queue in the hub already checks the vocabulary, but a document that got past it —
/// a job written by hand, a producer talking straight to this command — would otherwise reach the
/// paper as a nameless key/value dump, and a kitchen order that comes out wrong is only discovered
/// when the plate is missing. Failing here turns that into a `failed` the print host reports.
///
/// ⚠️ `async` is load-bearing: the bluetooth arm crosses into Kotlin through
/// `run_mobile_plugin`, which dispatches onto Android's main looper and BLOCKS for the answer — a
/// plain command runs on that very thread and the till would hang on the press that prints. And the
/// network arm awaits the paper, retries included, which only an async command can do.
#[tauri::command]
pub(crate) async fn erplora_print(
    app: tauri::AppHandle,
    state: tauri::State<'_, PeripheralsState>,
    printer_id: String,
    document_type: String,
    data: serde_json::Value,
    job_id: Option<String>,
) -> Result<(), HardwareError> {
    let target = print_target::parse_print_target(&printer_id)?;
    let doc = DocumentType::parse(&document_type).ok_or_else(|| {
        HardwareError::from(erplora_peripherals::PeripheralError::UnknownDocumentType(
            document_type.clone(),
        ))
    })?;
    let payload = escpos::render_document(doc, &data)?;
    match target {
        print_target::PrintTarget::Network(target) => {
            print_to_network(&state.queue, target, payload, job_id).await?;
        }
        print_target::PrintTarget::Bluetooth(bt) => bluetooth_send(&app, &bt.mac, &payload)?,
        print_target::PrintTarget::Usb(usb) => usb_send(&usb, &payload)?,
    }
    Ok(())
}

/// `erplora_test_print` — prints a test page on the given printer (over SPP when it is Bluetooth).
/// `async` for the same reasons as `erplora_print`: «Probar» on a switched-off network printer
/// says it did not print.
#[tauri::command]
pub(crate) async fn erplora_test_print(
    app: tauri::AppHandle,
    state: tauri::State<'_, PeripheralsState>,
    printer_id: String,
    data: Option<serde_json::Value>,
) -> Result<(), HardwareError> {
    let target = print_target::parse_print_target(&printer_id)?;
    // An `erplora-app` newer than the module that calls it gets `None` here — and the renderer
    // reads an empty document exactly as it reads a missing field, so the sheet still prints.
    let data = data.unwrap_or_else(|| serde_json::json!({}));
    let payload = escpos::render_test_page(&printer_id, &data);
    match target {
        print_target::PrintTarget::Network(target) => {
            print_to_network(&state.queue, target, payload, None).await?;
        }
        print_target::PrintTarget::Bluetooth(bt) => bluetooth_send(&app, &bt.mac, &payload)?,
        print_target::PrintTarget::Usb(usb) => usb_send(&usb, &payload)?,
    }
    Ok(())
}

/// `erplora_open_drawer` — opens the cash drawer with an ESC/POS kick over the printer's socket (or
/// over the SPP transport when the printer is Bluetooth: the kick is ESC/POS bytes like any other
/// document).
#[tauri::command]
pub(crate) async fn erplora_open_drawer(
    app: tauri::AppHandle,
    printer_id: String,
    pin: Option<u8>,
) -> Result<(), HardwareError> {
    match print_target::parse_print_target(&printer_id)? {
        print_target::PrintTarget::Network(target) => {
            drawer::open_drawer(&target, pin.unwrap_or(2)).await?;
        }
        print_target::PrintTarget::Bluetooth(bt) => {
            bluetooth_send(&app, &bt.mac, drawer::kick_command(pin.unwrap_or(2)))?;
        }
        // The kick is ESC/POS like any other document, so it rides the same raw queue.
        print_target::PrintTarget::Usb(usb) => {
            usb_send(&usb, drawer::kick_command(pin.unwrap_or(2)))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use erplora_peripherals::queue::RetryPolicy;

    // ── A ticket that does not come out is an error for whoever printed it ──────────────────────
    //
    // Answering `Ok` as soon as the bytes are in the in-memory queue would leave the till hearing
    // nothing with the printer switched off, and the device draining the hub's queue confirming
    // «printed». The network arm waits for the paper, retries included.

    #[test]
    fn a_switched_off_network_printer_is_an_error_for_whoever_printed() {
        let queue = std::sync::Arc::new(PrintQueue::new(RetryPolicy {
            max_attempts: 1,
            backoff_ms: 0,
            connect_timeout_ms: 500,
            write_timeout_ms: 500,
        }));
        let worker = queue.clone();
        tauri::async_runtime::spawn(async move {
            let (outcomes, _ignored) = tokio::sync::mpsc::unbounded_channel();
            worker.run(outcomes).await;
        });
        // A port nobody listens on any more: a printer that is off.
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a free port");
            listener.local_addr().expect("its address").port()
        };
        let target = print_target::NetworkTarget { host: "127.0.0.1".into(), port };

        let printed = tauri::async_runtime::block_on(print_to_network(
            &queue,
            target,
            b"ticket".to_vec(),
            Some("sale-1".into()),
        ));

        let err = printed.expect_err("a ticket that never reached the printer is not printed");
        assert!(
            matches!(err, HardwareError::Peripheral(erplora_peripherals::PeripheralError::Unreachable(_))),
            "got {err:?}"
        );
    }

    #[test]
    fn every_network_print_waits_for_the_paper() {
        // `erplora_print` and `erplora_test_print` («Probar») both go through `print_to_network`;
        // a hand-over that answers on the enqueue would report a print before the paper is out.
        let source = include_str!("printing.rs");
        let shell = source.split("\n#[cfg(test)]\nmod tests").next().unwrap_or_default();
        assert!(!shell.contains("queue.enqueue("), "a network print answers on the hand-over again");
        assert_eq!(shell.matches("print_to_network(&state.queue").count(), 2, "print and test print");
    }
}
