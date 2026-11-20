//! mDNS/Bonjour printer discovery: the printers that announce `_pdl-datastream._tcp` or
//! `_ipp._tcp` on the local network.

use std::collections::HashSet;
use std::time::Duration;

use crate::protocol::PrinterInfo;
use crate::{Result, ESCPOS_NETWORK_PORT};

const MDNS_BROWSE_WINDOW: Duration = Duration::from_millis(1500);
const MDNS_SERVICE_TYPES: [&str; 2] = ["_pdl-datastream._tcp.local.", "_ipp._tcp.local."];

/// mDNS/Bonjour discovery. On any error it returns an empty `Vec` instead of propagating it.
pub async fn discover_mdns() -> Result<Vec<PrinterInfo>> {
    // The `mdns-sd` daemon uses synchronous `flume` channels, so it runs on a blocking thread
    // rather than hogging the async runtime for the whole listening window.
    let result = tokio::task::spawn_blocking(browse_mdns_blocking).await;

    match result {
        Ok(printers) => Ok(printers),
        Err(e) => {
            tracing::error!("mDNS discovery failed (join): {e}");
            Ok(Vec::new())
        }
    }
}

/// The blocking half of mDNS: starts the daemon, browses the service types, listens for the window
/// and maps every `ServiceResolved` to a `PrinterInfo`. Never fails: on an error it logs and goes
/// on.
fn browse_mdns_blocking() -> Vec<PrinterInfo> {
    use mdns_sd::{ServiceDaemon, ServiceEvent};

    let mut printers: Vec<PrinterInfo> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();

    let daemon = match ServiceDaemon::new() {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("could not start the mDNS daemon: {e}");
            return printers;
        }
    };

    // The service type is kept next to its receiver: it is the ONLY thing that tells an office
    // printer (`_ipp._tcp`) from a thermal one.
    let mut receivers = Vec::new();
    for service_type in MDNS_SERVICE_TYPES {
        match daemon.browse(service_type) {
            Ok(rx) => receivers.push((service_type, rx)),
            Err(e) => tracing::debug!("could not browse {service_type}: {e}"),
        }
    }

    let deadline = std::time::Instant::now() + MDNS_BROWSE_WINDOW;
    loop {
        let now = std::time::Instant::now();
        if now >= deadline {
            break;
        }
        let remaining = deadline - now;

        // Poll every receiver with a short timeout until the window runs out.
        let mut progressed = false;
        for (service_type, rx) in &receivers {
            let slice = remaining.min(Duration::from_millis(100));
            match rx.recv_timeout(slice) {
                Ok(ServiceEvent::ServiceResolved(info)) => {
                    progressed = true;
                    if let Some(printer) = service_to_printer(&info, service_type) {
                        if seen_ids.insert(printer.id.clone()) {
                            tracing::debug!(
                                "mDNS printer found: {} ({})",
                                printer.name,
                                printer.id
                            );
                            printers.push(printer);
                        }
                    }
                }
                Ok(_) => {
                    progressed = true;
                }
                Err(_) => {
                    // This receiver timed out or disconnected: try the next one.
                }
            }
        }

        if !progressed {
            // Nothing pending: avoid a busy loop until the window runs out.
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    let _ = daemon.shutdown();
    printers
}

/// Maps a resolved `ServiceInfo` to a network `PrinterInfo`. `None` when it has no address.
fn service_to_printer(info: &mdns_sd::ServiceInfo, service_type: &str) -> Option<PrinterInfo> {
    let ip = info.get_addresses().iter().next()?.to_string();
    let port = match info.get_port() {
        0 => ESCPOS_NETWORK_PORT,
        p => p,
    };
    // The name is the first segment of the full name.
    let fullname = info.get_fullname();
    let name = match fullname.split('.').next() {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => format!("mDNS Printer ({ip})"),
    };

    Some(PrinterInfo {
        id: format!("network:{ip}:{port}"),
        name,
        kind: "network".into(),
        category: category_for_service(service_type).to_string(),
        status: "ready".into(),
        paper_width: 80,
        mac: None,
    })
}

/// Classifies a printer by the mDNS service that announced it.
///
/// `_ipp._tcp` is published by office multifunction and AirPrint printers and by hardly any ESC/POS
/// thermal printer, so it is the only reliable sign of "A4". `_pdl-datastream._tcp` (raw printing
/// on 9100) is announced by **both families**, so it classifies nothing.
pub fn category_for_service(service_type: &str) -> &'static str {
    if service_type.starts_with("_ipp.") {
        crate::protocol::PRINTER_CATEGORY_A4
    } else {
        crate::protocol::PRINTER_CATEGORY_UNKNOWN
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{PRINTER_CATEGORY_A4, PRINTER_CATEGORY_UNKNOWN};

    #[test]
    fn ipp_gives_away_an_office_printer() {
        assert_eq!(category_for_service("_ipp._tcp.local."), PRINTER_CATEGORY_A4);
    }

    #[test]
    fn raw_printing_classifies_nothing() {
        // `_pdl-datastream._tcp` means "I take bytes on 9100": a thermal printer and an A4 laser
        // both say it. Calling it thermal would be guessing.
        assert_eq!(
            category_for_service("_pdl-datastream._tcp.local."),
            PRINTER_CATEGORY_UNKNOWN
        );
    }
}
