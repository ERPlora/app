//! Hardware probe for USB printing through the OS print queue (hub#1083).
//!
//! The unit tests cover everything that can be proven without a printer; this covers what cannot.
//! CI has no printer plugged in, so the only place the USB path is validated against real hardware
//! is a machine with the cable in it — `qa-hub-macos`, or Ioan's Mac.
//!
//! ```text
//! cargo run -p erplora-peripherals --example usb_queue_probe            # list, prints nothing
//! cargo run -p erplora-peripherals --example usb_queue_probe -- <queue> # sends a REAL test page
//! ```

use erplora_peripherals::discovery::{parse_print_target, PrintTarget};
use erplora_peripherals::escpos;
use erplora_peripherals::usb::{discover_usb_printers, send_raw, SystemCups};

fn main() {
    match discover_usb_printers(&SystemCups) {
        Ok(found) if found.is_empty() => {
            println!("no USB print queue on this machine (a LAN-only till looks exactly like this)");
        }
        Ok(found) => {
            println!("{} USB print queue(s):", found.len());
            for p in &found {
                // The state comes from CUPS itself (hub#1541), so a queue with no paper reads
                // `stopped` here instead of the `ready` this listing used to print for every cable.
                println!("  {}  ->  {}  [{}]", p.id, p.name, p.status);
            }
        }
        Err(e) => {
            // Not a panic: "we could not ask the OS" is a real answer, and the operator running
            // this needs to read it rather than a backtrace.
            eprintln!("could not list the OS print queues: {e}");
            std::process::exit(1);
        }
    }

    let Some(queue) = std::env::args().nth(1) else {
        println!("\npass a queue name to send it a real test page.");
        return;
    };

    let printer_id = format!("usb:{queue}");
    let target = match parse_print_target(&printer_id) {
        Ok(PrintTarget::Usb(t)) => t,
        Ok(other) => {
            eprintln!("`{printer_id}` is not a USB target: {other:?}");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    println!("\nsending a test page to `{}`...", target.queue);
    match send_raw(&SystemCups, &target, &escpos::render_test_page(&printer_id, &serde_json::json!({}))) {
        Ok(()) => println!("submitted. Paper out of the printer = the whole chain works."),
        Err(e) => {
            eprintln!("refused: {e}");
            std::process::exit(1);
        }
    }
}
