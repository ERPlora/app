//! The USB print queues the OS knows about, listed with their state next to the network ones.

use std::collections::HashSet;

use super::cups::{CupsClient, CUPS_LPSTAT};
use super::first_meaningful_line;
use super::state::{ask_queue_state, QueueState};
use crate::print_target::is_cups_queue_name;
use crate::protocol::{default_printer_category, PrinterInfo};
use crate::{PeripheralError, Result};

/// `PrinterInfo.status` for a queue that will print the next job.
pub const QUEUE_STATUS_READY: &str = "ready";
/// `PrinterInfo.status` for a queue that would take the job and HOLD it — no paper, cover open,
/// cable pulled, paused.
pub const QUEUE_STATUS_STOPPED: &str = "stopped";
/// `PrinterInfo.status` for a queue whose state the OS did not tell us. Not a verdict: it is the
/// absence of one, so it is never shown as "ready".
pub const QUEUE_STATUS_UNKNOWN: &str = "unknown";

/// The `printer_id` prefix a queue is addressed by (`usb:{queue}`), and the way back from the id
/// to the queue name the OS knows.
const USB_PRINTER_ID_PREFIX: &str = "usb:";

/// The device-URI scheme the CUPS USB backend uses. Queues on any other scheme are network
/// printers the sweep already finds, and listing them twice would turn one printer into two.
const USB_DEVICE_URI_SCHEME: &str = "usb://";

/// Thermal receipt width, in millimetres — the same assumption the network sweep makes.
const DEFAULT_PAPER_WIDTH: u32 = 80;

/// The USB print queues the OS knows about, out of two `lpstat` listings: `-e`, the bare
/// destination names, and `-v`, each destination's device URI.
///
/// Two listings because `lpstat -v` is LOCALISED: `device for <queue>: <uri>` in the C locale,
/// `dispositivo para …` on a Spanish Mac, `<queue> のデバイス: …` in Japanese — and macOS picks that
/// language from the system preference (`AppleLanguages`), not from `LANG`/`LC_ALL`, so forcing a
/// locale on the child does not help. What no translation moves is that the line carries the queue
/// name as a word of its own and ends with the URI; and `lpstat -e` prints the names untranslated.
/// So each line is matched against those names instead of against a template we would have to
/// guess per language.
///
/// The URI is the only thing that says which cable a queue is on, and it is what keeps the network
/// printers out: the sweep already finds them, and offering them twice would turn one printer into
/// two devices. A queue name our own contract would refuse is skipped rather than listed — a
/// printer on screen that fails the moment it is picked is worse than one that is not there.
pub fn parse_usb_queues(destinations: &str, devices: &str) -> Vec<PrinterInfo> {
    let names: HashSet<&str> = destinations
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect();
    devices
        .lines()
        .filter_map(|line| {
            let (before_uri, uri) = split_at_device_uri(line)?;
            if !uri.to_ascii_lowercase().starts_with(USB_DEVICE_URI_SCHEME) {
                return None;
            }
            let Some(queue) = queue_named_in(before_uri, &names) else {
                // A USB cable we cannot put a name to is the one case where the till HAS a USB
                // printer and the screen will not show it — worth a line in the log, never a guess:
                // an invented name is a printer that fails the moment it is picked.
                tracing::warn!(
                    "usb: `lpstat -v` lists a USB queue that `lpstat -e` does not name; not offered: {line}"
                );
                return None;
            };
            if !is_cups_queue_name(queue) {
                return None;
            }
            Some(PrinterInfo {
                id: format!("{USB_PRINTER_ID_PREFIX}{queue}"),
                name: device_name(uri).unwrap_or_else(|| queue.to_string()),
                kind: "usb".into(),
                // A USB port says "bytes go through", not "this speaks ESC/POS": an A4 inkjet
                // plugs into the very same cable. Same honesty rule as the 9100 sweep and SPP.
                category: default_printer_category(),
                // `lpstat` carries no printer state at all, so this listing cannot know one.
                // [`discover_usb_printers`] asks and fills it in; on its own, the parser says so
                // rather than answering "ready" and having the till act on it.
                status: QUEUE_STATUS_UNKNOWN.into(),
                paper_width: DEFAULT_PAPER_WIDTH,
                mac: None,
            })
        })
        .collect()
}

/// Splits an `lpstat -v` line into what comes before the device URI and the URI itself. The URI
/// is where `://` is, extended left over its scheme: the one landmark no translation moves.
fn split_at_device_uri(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    let separator = line.find("://")?;
    let scheme_start = line[..separator]
        .char_indices()
        .rev()
        .find(|&(_, c)| !(c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')))
        .map_or(0, |(i, c)| i + c.len_utf8());
    Some((&line[..scheme_start], &line[scheme_start..]))
}

/// The destination name that `before_uri` carries as a word of its own — the one nearest the URI,
/// should a template word happen to be a queue name too. Words are cut on whitespace and on the
/// `:` (or full-width `：`) CUPS puts between the name and the URI, with or without space around it.
fn queue_named_in<'a>(before_uri: &'a str, names: &HashSet<&str>) -> Option<&'a str> {
    before_uri
        .rsplit(|c: char| c.is_whitespace() || c == ':' || c == '：')
        .find(|word| !word.is_empty() && names.contains(word))
}

/// Every USB print queue this machine has, ready to be offered next to the network ones.
///
/// Asks twice — `lpstat -e` for the names (CUPS 2.2+, 2016; macOS and any current Linux have it),
/// `lpstat -v` for the cables — because the second listing is localised and the first is not; see
/// [`parse_usb_queues`]. A till with no printer at all answers an empty list from both, exit 0.
///
/// The failure is returned, not swallowed into an empty list: "this till has no USB printer" and
/// "we could not ask the OS" need opposite things from the user, and a silent empty list reads as
/// the first while being the second. The caller decides whether that is fatal — for the shell's
/// discovery it is not: a venue whose LAN printers work must not lose them because CUPS is absent.
pub fn discover_usb_printers(client: &dyn CupsClient) -> Result<Vec<PrinterInfo>> {
    let destinations = lpstat(client, "-e")?;
    let devices = lpstat(client, "-v")?;
    let mut queues = parse_usb_queues(&destinations, &devices);
    // One more question per USB queue — a till has one or two, and the answer is the difference
    // between a screen that says "lista" next to a printer with no paper and one that does not.
    // Network printers are not asked: they never came from CUPS.
    for printer in &mut queues {
        let queue = printer
            .id
            .strip_prefix(USB_PRINTER_ID_PREFIX)
            .unwrap_or(&printer.id)
            .to_string();
        printer.status = match ask_queue_state(client, &queue) {
            QueueState::Ready => QUEUE_STATUS_READY,
            QueueState::NotReady { .. } => QUEUE_STATUS_STOPPED,
            QueueState::Unknown => QUEUE_STATUS_UNKNOWN,
        }
        .into();
    }
    Ok(queues)
}

/// One `lpstat` listing, or why it could not be had.
fn lpstat(client: &dyn CupsClient, flag: &str) -> Result<String> {
    let out = client.run(CUPS_LPSTAT, &[flag], &[])?;
    if out.success {
        return Ok(out.stdout);
    }
    let reason = first_meaningful_line(&out.stderr)
        .or_else(|| first_meaningful_line(&out.stdout))
        .unwrap_or("no reason given");
    Err(PeripheralError::Unreachable(format!(
        "could not list the OS print queues (`lpstat {flag}`): {reason}"
    )))
}

/// A human name out of a `usb://Make/Model?serial=…` device URI.
///
/// `usb://Star/TSP143%20(STR_T-001)?serial=X5` → `Star TSP143 (STR_T-001)`. `None` when the URI
/// carries nothing but the scheme, so the caller can fall back to the queue name.
fn device_name(uri: &str) -> Option<String> {
    let rest = uri.get(USB_DEVICE_URI_SCHEME.len()..)?;
    let rest = rest.split('?').next().unwrap_or(rest);
    let name = percent_decode(&rest.replace('/', " "));
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Decodes the `%XX` escapes CUPS puts in a device URI. Anything that is not a well-formed escape
/// is left exactly as it was: this is a label for a screen, never a round-trip.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let decoded = (bytes[i] == b'%' && i + 2 < bytes.len())
            .then(|| std::str::from_utf8(&bytes[i + 1..i + 3]).ok())
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match decoded {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usb::cups::{CupsOutput, CUPS_LPOPTIONS};
    use crate::usb::fake_cups::{cups_said, FakeCups, LPOPTIONS_IDLE};

    #[test]
    fn only_usb_queues_are_offered_and_they_are_named_after_the_device() {
        // `lpstat -v` is the only place the OS says which queue is on which cable. The office
        // laser on IPP and the shared network printer are already found by the sweep; offering
        // them again here would turn one printer into two devices in the registry.
        let listing = "device for Star_TSP143: usb://Star/TSP143%20(STR_T-001)?serial=X5\n\
                       device for Office_Laser: ipp://10.0.0.9:631/ipp/print\n\
                       device for Kitchen: socket://10.0.0.5:9100\n\
                       device for EPSON_TM: usb://EPSON/TM-T20III\n";

        let found = parse_usb_queues("Star_TSP143\nOffice_Laser\nKitchen\nEPSON_TM\n", listing);

        let ids: Vec<&str> = found.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["usb:Star_TSP143", "usb:EPSON_TM"], "only the USB cables");

        let star = &found[0];
        assert_eq!(star.kind, "usb");
        assert_eq!(
            star.name, "Star TSP143 (STR_T-001)",
            "the device URI carries make and model; percent escapes are not for human eyes"
        );
        // Same honesty rule as the port-9100 sweep and SPP: a USB cable says "bytes go through",
        // not "this speaks ESC/POS" — an A4 inkjet plugs into the same port.
        assert_eq!(star.category, default_printer_category());
        // `lpstat` carries no printer state at all, so the parser cannot know. Answering "ready"
        // anyway would put that word on screen next to a printer with no paper, and the till acts
        // on it. What the listing does not say, the listing does not claim.
        assert_eq!(star.status, QUEUE_STATUS_UNKNOWN);
        assert_eq!(star.mac, None, "a queue has no MAC; its name is its identity");
    }

    #[test]
    fn a_listing_in_the_systems_language_still_finds_the_usb_queue() {
        // CUPS localises the `lpstat -v` line, and macOS picks that language from the system
        // preference (`AppleLanguages`), NOT from `LANG`/`LC_ALL` — forcing the C locale on the
        // child does not help there (measured: `dispositivo para …` with `LC_ALL=C`).
        // A till in Spain runs in Spanish, so an English-only parser would list zero USB printers
        // for exactly the customer this transport exists for. Templates are the real ones from
        // CUPS' `locale/cups_<lang>.po`; the Spanish line is verbatim from a Mac.
        let listing = "dispositivo para Star_TSP143: usb://Star/TSP143?serial=X5\n\
                       dispositivo para Brother_HL_3150CDW_series: dnssd://Brother%20HL-3150CDW%20series._ipp._tcp.local./?uuid=e3248000-80ce-11db-8000-3c2af45a10ef\n\
                       matériel pour EPSON_TM : usb://EPSON/TM-T20III\n\
                       Gerät für Bar_Star: usb://Star/TSP654\n\
                       dispositiu per Cuina: usb://EPSON/TM-T88\n\
                       Kitchen_Star のデバイス: usb://Star/TSP143\n\
                       用于 Caja_Epson 的设备：usb://EPSON/TM-m30\n";
        // `lpstat -e` is the same in every language: bare names, one per line.
        let names = "Star_TSP143\nBrother_HL_3150CDW_series\nEPSON_TM\nBar_Star\nCuina\nKitchen_Star\nCaja_Epson\n";

        let ids: Vec<String> = parse_usb_queues(names, listing).into_iter().map(|p| p.id).collect();

        assert_eq!(
            ids,
            vec![
                "usb:Star_TSP143",
                "usb:EPSON_TM",
                "usb:Bar_Star",
                "usb:Cuina",
                "usb:Kitchen_Star",
                "usb:Caja_Epson"
            ],
            "every USB queue must be found whatever language CUPS speaks, and the Bonjour one never"
        );
    }

    #[test]
    fn a_queue_we_could_never_address_is_not_offered() {
        // If the OS reports a destination whose name our own contract refuses (`-oraw` would be
        // read by `lp -d` as a flag), listing it would put a printer on screen that fails the
        // moment it is picked. Better absent than a trap.
        let listing = "device for -oraw: usb://Star/TSP143\n\
                       device for good_one: usb://Star/TSP143\n";

        let ids: Vec<String> =
            parse_usb_queues("-oraw\ngood_one\n", listing).into_iter().map(|p| p.id).collect();

        assert_eq!(ids, vec!["usb:good_one"]);
    }

    #[test]
    fn a_usb_cable_the_destination_list_does_not_name_is_not_guessed() {
        // Matching against `lpstat -e` is what makes the parser language-proof; the flip side is
        // that a line whose name is not in that list must NOT be offered under whatever word sits
        // before the colon (`のデバイス` in Japanese) — that is a printer that fails when picked.
        let listing = "Kitchen_Star のデバイス: usb://Star/TSP143\n";

        assert!(parse_usb_queues("", listing).is_empty(), "no name, no guess");
        assert!(parse_usb_queues("Other_Queue\n", listing).is_empty(), "a different name is no name");
    }

    #[test]
    fn the_real_lpstat_listing_of_this_machine_parses() {
        // Captured verbatim from `lpstat -v` on macOS. A network printer shared over
        // Bonjour: it is already found by the LAN sweep, so offering it here too would turn one
        // printer into two devices. Pinning REAL output keeps the parser honest about the shape
        // CUPS actually emits, which no hand-written fixture can promise.
        let real = "device for Brother_HL_3150CDW_series: dnssd://Brother%20HL-3150CDW%20series._ipp._tcp.local./?uuid=e3248000-80ce-11db-8000-3c2af45a10ef\n";

        assert!(
            parse_usb_queues("Brother_HL_3150CDW_series\n", real).is_empty(),
            "a Bonjour printer is not on a USB cable"
        );
    }

    #[test]
    fn discovery_asks_the_os_for_its_queues_and_their_cables() {
        let cups = FakeCups::listing(
            "Star_TSP143\nOffice\n",
            "device for Star_TSP143: usb://Star/TSP143\ndevice for Office: ipp://10.0.0.9:631/ipp/print\n",
        );

        let found = discover_usb_printers(&cups).expect("a healthy CUPS answers");

        let calls = cups.calls.borrow();
        let asked: Vec<(&str, &[String])> =
            calls.iter().map(|(program, args, _)| (program.as_str(), args.as_slice())).collect();
        assert_eq!(
            asked,
            vec![
                (CUPS_LPSTAT, &["-e".to_string()][..]),
                (CUPS_LPSTAT, &["-v".to_string()][..]),
                // And then, for the USB ones only, how each of them is doing.
                (CUPS_LPOPTIONS, &["-p".to_string(), "Star_TSP143".to_string()][..]),
            ],
            "`-e` is the names (never translated); `-v` is what prints the device URI, without \
             which there is no way to tell a cable from a LAN"
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "usb:Star_TSP143");
    }

    #[test]
    fn not_being_able_to_ask_the_os_is_an_error_not_an_empty_list() {
        // "no USB printer here" and "we could not ask" need opposite things from the user: plug
        // one in, versus install the CUPS tools. An empty list would say the first while meaning
        // the second, and the till would look like it simply has no printer.
        let cups = FakeCups::answering(CupsOutput {
            success: false,
            stdout: String::new(),
            stderr: "lpstat: Bad file descriptor".into(),
        });

        let err = discover_usb_printers(&cups).expect_err("a failed listing must not read as empty");

        assert!(err.to_string().contains("Bad file descriptor"), "got: {err}");
    }

    #[test]
    fn a_listing_without_any_usb_queue_is_an_empty_answer_not_a_failure() {
        // A till with only LAN printers is normal, not broken — and so is one with no printer at
        // all, where both listings come back empty with exit 0 (checked in CUPS' `lpstat.c`:
        // `show_devices` prints nothing and returns 0 when there is nothing to show).
        assert!(parse_usb_queues("Office\n", "device for Office: ipp://10.0.0.9:631/ipp/print\n").is_empty());
        assert!(parse_usb_queues("", "").is_empty());
        assert!(parse_usb_queues("", "lpstat: No destinations added.\n").is_empty());
        assert!(discover_usb_printers(&FakeCups::listing("", "")).expect("empty is fine").is_empty());
    }

    // ── The list ─────────────────────────────────────────────────────────────────────────────

    #[test]
    fn the_list_shows_each_usb_queues_real_state_instead_of_a_fixed_ready() {
        // "Lista" next to a printer with no paper is the same lie as the accepted job, one screen
        // earlier: it is what makes the cashier pick that printer in the first place.
        let cups = FakeCups {
            answers: vec![
                (vec!["-e"], cups_said("Star_TSP143\nEPSON_TM\n")),
                (
                    vec!["-v"],
                    cups_said(
                        "device for Star_TSP143: usb://Star/TSP143\n\
                         device for EPSON_TM: usb://EPSON/TM-T20III\n",
                    ),
                ),
                (vec!["-p", "Star_TSP143"], cups_said(LPOPTIONS_IDLE)),
                (
                    vec!["-p", "EPSON_TM"],
                    cups_said("printer-is-accepting-jobs=true printer-state=5 printer-state-reasons=media-empty-error"),
                ),
            ],
            ..Default::default()
        };

        let found = discover_usb_printers(&cups).expect("a healthy CUPS answers");

        let shown: Vec<(&str, &str)> =
            found.iter().map(|p| (p.id.as_str(), p.status.as_str())).collect();
        assert_eq!(
            shown,
            vec![
                ("usb:Star_TSP143", QUEUE_STATUS_READY),
                ("usb:EPSON_TM", QUEUE_STATUS_STOPPED)
            ]
        );
    }

    #[test]
    fn a_queue_whose_state_could_not_be_asked_is_listed_unknown_not_ready() {
        // Same rule as the submit: "we could not ask" is its own answer. Printing it as `ready`
        // is the bug; hiding the printer would take away the one the till actually has.
        let cups = FakeCups {
            answers: vec![
                (vec!["-e"], cups_said("Star_TSP143\n")),
                (vec!["-v"], cups_said("device for Star_TSP143: usb://Star/TSP143\n")),
                // Nothing scripted for `-p`: the fake answers a failure, like a CUPS that cannot.
            ],
            ..Default::default()
        };

        let found = discover_usb_printers(&cups).expect("the listing itself worked");

        assert_eq!(found.len(), 1, "the printer is there, we just could not ask how it is");
        assert_eq!(found[0].status, QUEUE_STATUS_UNKNOWN);
    }
}
