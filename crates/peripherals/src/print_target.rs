//! Parsing of `printer_id` (**all three transports**) into the [`PrintTarget`] a print job goes to.

use crate::{Result, ESCPOS_NETWORK_PORT};

/// Network print destination: a TCP host and port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkTarget {
    pub host: String,
    pub port: u16,
}

impl NetworkTarget {
    /// `host:port`, as `tokio::net::TcpStream::connect` takes it.
    pub fn socket_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Bluetooth Classic SPP print destination — Android only.
///
/// The MAC is the whole identity: RFCOMM connects to a bonded device by address, there is no
/// host/port. Desktop and iOS never produce nor accept this variant — iOS because of Apple's MFi
/// restriction on Bluetooth Classic, desktop because the cheap printer there arrives by cable and
/// is served by the OS print queue instead ([`UsbTarget`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BluetoothTarget {
    /// Normalized (`AA:BB:CC:DD:EE:FF`): the same spelling the device registry keys on and the
    /// Kotlin transport connects with — two spellings of one printer would be two devices.
    pub mac: String,
}

/// Raw print destination through the operating system's own print queue — desktop only.
///
/// The queue name is the whole identity. There is no host, no port and no MAC: the cable is the
/// OS's business, and what we address is the spooler entry the user (or the vendor installer)
/// created for the printer. That is deliberate — it is what lets us support USB *without shipping
/// a driver per OS*. The vendor writes the driver; we keep emitting the same ESC/POS bytes we emit
/// over TCP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbTarget {
    /// A CUPS destination name, already validated: non-empty, no SPACE/TAB/`/`/`#`, no leading
    /// `-`, at most [`CUPS_MAX_QUEUE_NAME`] bytes. See [`parse_print_target`].
    pub queue: String,
}

/// Where a print job goes, parsed from a `printer_id`.
///
/// `network:{ip}:{port}` feeds the TCP path everywhere; `bluetooth:{mac}` exists ONLY on Android,
/// where the shell hands the rendered ESC/POS bytes to the Kotlin SPP transport. The rendering
/// stays in Rust either way — the transport is the only thing that changes.
#[derive(Debug, Clone, PartialEq)]
pub enum PrintTarget {
    Network(NetworkTarget),
    Bluetooth(BluetoothTarget),
    Usb(UsbTarget),
}

/// The longest destination name `cupsd` will accept (`printer-name` is capped at 127 bytes).
pub const CUPS_MAX_QUEUE_NAME: usize = 127;

/// Whether `name` could be a CUPS destination at all.
///
/// CUPS forbids SPACE, TAB, `/` and `#` in a destination name, so anything carrying one names a
/// queue that *cannot exist* — better said here than as an `lp` exit code nobody can trace back to
/// the printer the cashier picked. The leading `-` is ours: `lp -d <queue>` would read it as a
/// flag, and a job that quietly prints somewhere else is worse than one that fails.
pub(crate) fn is_cups_queue_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= CUPS_MAX_QUEUE_NAME
        && !name.starts_with('-')
        && !name
            .chars()
            .any(|c| c == ' ' || c == '\t' || c == '/' || c == '#' || c.is_control())
}

/// Parses a `printer_id` into the transport it names: `network:{ip}:{port}`, `bluetooth:{mac}`
/// or `usb:{queue}`. Anything else — an unknown scheme, a malformed MAC, a queue name the OS could
/// never have — is refused HERE, at the contract's edge, instead of travelling on to blow up inside
/// a socket connect or an `lp` exit code with a message nobody can map back to the id.
pub fn parse_print_target(printer_id: &str) -> Result<PrintTarget> {
    match printer_id.split_once(':') {
        Some(("bluetooth", mac)) => {
            if !crate::registry::mac::is_mac(mac) {
                return Err(crate::PeripheralError::InvalidPrinterId(format!(
                    "malformed bluetooth MAC in: {printer_id}"
                )));
            }
            Ok(PrintTarget::Bluetooth(BluetoothTarget {
                mac: crate::registry::mac::normalize_mac(mac),
            }))
        }
        Some(("usb", queue)) => {
            if !is_cups_queue_name(queue) {
                return Err(crate::PeripheralError::InvalidPrinterId(format!(
                    "not a usable OS print queue name in: {printer_id}"
                )));
            }
            Ok(PrintTarget::Usb(UsbTarget {
                queue: queue.to_string(),
            }))
        }
        _ => parse_printer_id(printer_id).map(PrintTarget::Network),
    }
}

/// Parses a `network:{ip}:{port}` `printer_id` into a [`NetworkTarget`].
/// It is the parser of the TCP path (print queue, drawer over a socket, registry enrichment), so it
/// refuses `bluetooth:` and `usb:` — those variants reach hardware only through
/// [`parse_print_target`] (Android's SPP transport and the OS's raw print queue).
pub fn parse_printer_id(printer_id: &str) -> Result<NetworkTarget> {
    // The scheme is split off at the FIRST ':'.
    let (scheme, rest) = match printer_id.split_once(':') {
        Some((s, r)) => (s, r),
        None => (printer_id, ""),
    };

    if scheme != "network" {
        return Err(crate::PeripheralError::InvalidPrinterId(format!(
            "not a network printer id (this is the TCP path only): {scheme}"
        )));
    }

    // Split at the LAST ':': the host may contain ':' (IPv6), the port is always last.
    let (host, port) = match rest.rsplit_once(':') {
        Some((h, p)) => {
            let port = p.parse::<u16>().map_err(|_| {
                crate::PeripheralError::InvalidPrinterId(format!("invalid port in: {printer_id}"))
            })?;
            (h.to_string(), port)
        }
        None => (rest.to_string(), ESCPOS_NETWORK_PORT),
    };

    if host.is_empty() {
        return Err(crate::PeripheralError::InvalidPrinterId(format!(
            "empty host in: {printer_id}"
        )));
    }

    Ok(NetworkTarget { host, port })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── `bluetooth:{mac}` in the printer_id contract ──────────────────────────────────────────
    //
    // A Kotlin plugin runs in-process with the full Android APIs, so the cheap bonded Bluetooth
    // thermal printer at the table can be driven from the app. Only Android produces and accepts
    // the variant.

    #[test]
    fn a_bluetooth_printer_id_is_a_valid_print_target_now() {
        match parse_print_target("bluetooth:aa:bb:cc:dd:ee:ff") {
            Ok(PrintTarget::Bluetooth(t)) => {
                // Normalized on the way in: the MAC is the device's identity in the registry and
                // in the Kotlin transport, and two spellings of one printer would be two devices.
                assert_eq!(t.mac, "AA:BB:CC:DD:EE:FF");
            }
            other => panic!("bluetooth:{{mac}} must parse as a Bluetooth target, got {other:?}"),
        }
    }

    #[test]
    fn a_network_print_target_parses_exactly_as_before() {
        match parse_print_target("network:10.0.0.5:9100") {
            Ok(PrintTarget::Network(t)) => {
                assert_eq!(t.host, "10.0.0.5");
                assert_eq!(t.port, 9100);
            }
            other => panic!("network ids must keep parsing unchanged, got {other:?}"),
        }
    }

    // ── `usb:{queue}` in the printer_id contract, on the desktop ──────────────────────────────
    //
    // USB does not need a driver per OS from us: the OS print queue IS the generic transport, the
    // vendor writes the driver, and we keep emitting the ESC/POS we already emit. The USB thermal
    // printer is the cheapest in the catalogue and the one a single-till bar or salon actually
    // buys.

    #[test]
    fn a_usb_printer_id_names_an_os_queue() {
        match parse_print_target("usb:Star_TSP143") {
            Ok(PrintTarget::Usb(t)) => assert_eq!(t.queue, "Star_TSP143"),
            other => panic!("usb:{{queue}} must parse as a USB target, got {other:?}"),
        }
    }

    #[test]
    fn a_queue_name_the_os_could_never_have_is_refused_at_the_edge() {
        // CUPS forbids SPACE, TAB, `/` and `#` in a destination name, so an id carrying one names
        // a queue that cannot exist. Refusing it HERE keeps it from travelling on to come back as
        // an `lp` exit code nobody can map to the printer the cashier picked.
        assert!(parse_print_target("usb:").is_err(), "an empty queue names nothing");
        assert!(parse_print_target("usb:Star TSP143").is_err(), "CUPS forbids spaces");
        assert!(parse_print_target("usb:kitchen/star").is_err(), "CUPS forbids `/`");
        assert!(parse_print_target("usb:star#1").is_err(), "CUPS forbids `#`");
        assert!(parse_print_target("usb:star\tone").is_err(), "CUPS forbids TAB");
    }

    #[test]
    fn a_leading_dash_is_refused_so_a_queue_can_never_become_an_lp_flag() {
        // `lp -d <queue>`: a destination that starts with `-` is an argument `lp` would read as a
        // flag. There is no shell here (argv, never a string), so this is not an injection — it
        // is a queue that would silently print somewhere else, which is worse than an error.
        assert!(parse_print_target("usb:-oraw").is_err());
        assert!(parse_print_target("usb:--help").is_err());
    }

    #[test]
    fn a_malformed_bluetooth_mac_is_still_refused() {
        // A `bluetooth:` id whose rest is not a MAC must fail HERE, not travel to Kotlin to blow
        // up inside the socket connect with a message nobody maps back to the id.
        assert!(parse_print_target("bluetooth:not-a-mac").is_err());
        assert!(parse_print_target("bluetooth:").is_err());
        assert!(parse_print_target("bluetooth:AA:BB:CC:DD:EE").is_err(), "5 groups is not a MAC");
    }

    #[test]
    fn the_network_only_parser_still_refuses_usb() {
        // `parse_printer_id` is the NETWORK parser: the print queue (`PrintJob.target`), the
        // drawer's socket path and the registry enrichment all feed TCP and cannot take a queue
        // name. USB reaches hardware exclusively through `parse_print_target`'s USB arm.
        assert!(parse_printer_id("usb:Star_TSP143").is_err());
    }

    #[test]
    fn the_network_only_parser_still_refuses_bluetooth() {
        // `parse_printer_id` is the NETWORK parser: the print queue (`PrintJob.target`), the
        // drawer and the registry enrichment feed TCP sockets and cannot take a MAC. Bluetooth
        // ids reach hardware exclusively through `parse_print_target`'s Bluetooth arm (the
        // Android SPP transport), so this door stays closed.
        assert!(parse_printer_id("bluetooth:AA:BB:CC:DD:EE:FF").is_err());
        assert!(parse_printer_id("network:10.0.0.5:9100").is_ok());
    }
}
