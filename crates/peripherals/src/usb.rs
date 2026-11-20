//! USB printing through the operating system's own print queue — desktop only.
//!
//! The transport is CUPS on macOS and Linux: `lp -d <queue> -o raw` hands our already-rendered
//! ESC/POS straight to the spooler, filters bypassed. That indirection is the whole point of the
//! design: *a driver per operating system does not scale*, and this transport needs none. The
//! vendor (Star, Epson) already ships the CUPS driver, the OS already owns the cable, and ESC/POS
//! is already the universal language we generate. So the generic transport is the queue, not a
//! USB library.
//!
//! Deliberately NOT here:
//!   - **Windows** (`OpenPrinter`/`StartDocPrinter`/`WritePrinter`) — the same idea against the
//!     other spooler, but it cannot be verified without a real Windows machine, and a print path
//!     that is green in tests and broken at the counter must not ship.
//!   - **Android** — the Bluetooth SPP transport already covers the cheap printer there, and USB
//!     Host means Kotlin, an intent-granted permission and OTG cables.
//!   - **`libusb` / WebUSB / OPOS** — each one re-introduces the per-OS driver problem.
//!   - **A role (kitchen/bar/receipt) on a USB queue** — that is the device registry's job, where
//!     a queue is keyed by its whole `printer_id` (`DeviceRegistry::register_queue`): it has no
//!     MAC or `ip:port` to key on. Discovering and printing do not need the registry.

use std::collections::HashSet;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::discovery::{is_cups_queue_name, UsbTarget};
use crate::protocol::{default_printer_category, PrinterInfo};
use crate::{PeripheralError, Result};

/// The CUPS client that submits a job.
pub const CUPS_LP: &str = "lp";
/// The CUPS client that lists destinations and their device URIs.
pub const CUPS_LPSTAT: &str = "lpstat";
/// The CUPS client that prints a destination's attributes as `key=value`.
///
/// Same package as [`CUPS_LP`] and [`CUPS_LPSTAT`] (`cups-client`; macOS ships it), so asking it
/// costs the transport no dependency it did not already have. It is asked instead of `lpstat -p`
/// because `lpstat` answers in a SENTENCE and CUPS translates that sentence; `lpoptions` answers
/// with the IPP attributes themselves, and an enum is an enum in every language.
pub const CUPS_LPOPTIONS: &str = "lpoptions";

/// The CUPS client that takes a job back out of a queue.
///
/// Same package as the other three (`cups-client`), so it costs the transport no dependency it
/// did not already have. It is used only when a ticket did NOT come out: a job left behind is not
/// harmless paperwork, it is a receipt that prints by surprise hours later when someone reloads
/// the roll — with a number the till already reported as failed, next to a customer who left.
pub const CUPS_CANCEL: &str = "cancel";

/// `PrinterInfo.status` for a queue that will print the next job.
pub const QUEUE_STATUS_READY: &str = "ready";
/// `PrinterInfo.status` for a queue that would take the job and HOLD it — no paper, cover open,
/// cable pulled, paused.
pub const QUEUE_STATUS_STOPPED: &str = "stopped";
/// `PrinterInfo.status` for a queue whose state the OS did not tell us. Not a verdict: it is the
/// absence of one, so it is never shown as "ready".
pub const QUEUE_STATUS_UNKNOWN: &str = "unknown";

/// IPP `printer-state` (RFC 8011 section 5.4.11), the whole set — there is no fourth value.
const IPP_STATE_IDLE: &str = "3";
const IPP_STATE_PROCESSING: &str = "4";
const IPP_STATE_STOPPED: &str = "5";

/// The IPP severity suffix that means "this printer cannot print now" (RFC 8011 section 5.4.12).
/// `-warning` and `-report` are the other two, and they must NOT stop a till: a thermal roll
/// running low reports `media-low-warning` for hours while printing perfectly.
const REASON_ERROR_SUFFIX: &str = "-error";

/// How long a CUPS client gets before we give up on it.
///
/// Not a formality: when `cupsd` wedges, `lp` blocks on its socket and never returns. Without a
/// deadline that is a till frozen on the press that prints the ticket — the cashier's only way out
/// being to kill the app. Twenty seconds is far beyond a healthy submit (milliseconds) and still
/// short enough that the error reaches the screen while the customer is still standing there.
pub const CUPS_TIMEOUT: Duration = Duration::from_secs(20);

/// How often the wait wakes up to check on the child.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// How long a job the spooler ACCEPTED gets to leave the queue before the till calls it failed.
///
/// `lp` exiting 0 means the spooler wrote the job down, which is as far as it can see. The OS
/// only learns that the paper ran out or the cable is gone WHILE it is running a job, so at
/// submit time the fault the pre-flight check looks for does not exist yet: it appears
/// seconds later, by which point the till has already said "printed".
///
/// The budget is set by CUPS' own USB backend, not by the receipt: after the LAST byte is written
/// it still holds the job while it waits for its read thread — `WAIT_EOF_DELAY` = 7 s plus a 1 s
/// abort grace in both `backend/usb-libusb.c` (Linux) and `backend/usb-darwin.c` (macOS) — and
/// on Linux that thread sits in a 60 s bulk read on any bidirectional printer with nothing to say
/// (Epson TM 04b8:0202, Bixolon, Citizen; Star is quirked `unidir` and skips it). A ticket that
/// already came out is therefore "not completed" for ~8 s more. A window under that would cancel
/// it and tell the cashier it did not print — the duplicate receipt this transport must never
/// cause. 8 s of tail + a few seconds for a long kitchen ticket on a 100 mm/s printer = 15 s,
/// still far inside the hub's 90 s lease on the job. On the happy path none of it is waited for:
/// the job leaves the queue and the wait ends with it (165 ms measured against a queue that does
/// not hold).
pub const JOB_COMPLETION_WINDOW: Duration = Duration::from_secs(15);

/// How often the OS queue is re-read while waiting for the job to leave it. Sixty polls across
/// [`JOB_COMPLETION_WINDOW`]: cheap enough not to matter, tight enough that a receipt that came
/// out in 300 ms does not keep the cashier waiting for a whole second.
const JOB_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Job title, so a stuck ticket is identifiable in the OS queue window instead of being an
/// anonymous "Untitled" the user cannot connect to the till.
const JOB_TITLE: &str = "ERPlora";

/// The `printer_id` prefix a queue is addressed by (`usb:{queue}`), and the way back from the id
/// to the queue name the OS knows.
const USB_PRINTER_ID_PREFIX: &str = "usb:";

/// The device-URI scheme the CUPS USB backend uses. Queues on any other scheme are network
/// printers the sweep already finds, and listing them twice would turn one printer into two.
const USB_DEVICE_URI_SCHEME: &str = "usb://";

/// Thermal receipt width, in millimetres — the same assumption the network sweep makes.
const DEFAULT_PAPER_WIDTH: u32 = 80;

/// What a CUPS client command answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CupsOutput {
    /// Whether the process exited 0.
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Runs a CUPS client binary — the seam between this module and the machine.
///
/// It exists so the transport can be tested at all: a unit test that really spawned `lp` would
/// either print on the developer's desk or fail on every machine without a printer, so neither
/// outcome would mean anything. [`SystemCups`] is the real one.
pub trait CupsClient {
    /// Runs `program` with `args`, feeding `stdin`, and waits for it (bounded by [`CUPS_TIMEOUT`]).
    fn run(&self, program: &str, args: &[&str], stdin: &[u8]) -> Result<CupsOutput>;
}

/// The argv for submitting raw bytes to `queue`.
///
/// `-o raw` is the load-bearing flag: it marks the job `application/vnd.cups-raw`, which tells
/// CUPS to skip its filter chain and hand the bytes to the backend untouched. Without it the
/// spooler would try to *render* our ESC/POS as if it were a document, and what comes out of the
/// printer is a page of garbage instead of a receipt.
pub fn lp_args(queue: &str) -> Vec<&str> {
    vec!["-d", queue, "-t", JOB_TITLE, "-o", "raw"]
}

/// What the OS says about a print queue, in IPP's own vocabulary — never in its prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueState {
    /// Idle or printing, and taking work: a submit reaches the paper.
    Ready,
    /// The spooler would accept the job and HOLD it. `reason` carries the attributes that decided
    /// it, verbatim and untranslated, because that is what tells the cashier WHICH fault to fix.
    NotReady { reason: String },
    /// We could not ask. Deliberately not a refusal — see [`send_raw`].
    Unknown,
}

/// Reads [`QueueState`] out of one `lpoptions -p <queue>` line.
///
/// The line is `key=value` pairs separated by spaces, with single quotes around any value that has
/// a space in it (`printer-info='Brother HL-3150CDW series'`). Only three keys are looked at, and
/// none of their values can contain a space, so splitting on whitespace and matching whole keys is
/// enough — a quoted fragment never looks like one of them.
///
/// `printer-state` missing is [`QueueState::Unknown`], not a fault: `lpoptions` against a
/// destination CUPS does not know exits 0 and prints uninitialised bytes with no attribute in them
/// (measured on macOS).
pub fn parse_queue_state(options: &str) -> QueueState {
    let Some(state) = attribute(options, "printer-state") else {
        return QueueState::Unknown;
    };
    let reasons = attribute(options, "printer-state-reasons").unwrap_or("none");
    let accepting = attribute(options, "printer-is-accepting-jobs").unwrap_or("true");

    let stopped = state == IPP_STATE_STOPPED;
    let faulted = reasons
        .split(',')
        .any(|reason| reason.trim().ends_with(REASON_ERROR_SUFFIX));
    let refusing = accepting == "false";
    if !(stopped || faulted || refusing) {
        return QueueState::Ready;
    }

    // Every signal that fired goes in: one fixed fault out of two is still no ticket.
    let mut reported = Vec::new();
    if stopped {
        reported.push(format!("printer-state={}", state_name(state)));
    }
    if (stopped || faulted) && reasons != "none" {
        reported.push(format!("printer-state-reasons={reasons}"));
    }
    if refusing {
        reported.push("printer-is-accepting-jobs=false".to_string());
    }
    QueueState::NotReady { reason: reported.join(" ") }
}

/// The IPP enum as a word, so the error a cashier's manager reads is not a bare `5`. An
/// unforeseen value is passed through as it came rather than guessed at.
fn state_name(state: &str) -> &str {
    match state {
        IPP_STATE_IDLE => "idle",
        IPP_STATE_PROCESSING => "processing",
        IPP_STATE_STOPPED => "stopped",
        other => other,
    }
}

/// One `key=value` out of an `lpoptions -p` line.
///
/// Whole keys only: `printer-state` must not match `printer-state-reasons`, which is why the `=`
/// is required right after the key rather than the prefix being enough. Values with a space in
/// them are single-quoted by `lpoptions` (`printer-info='Brother HL-3150CDW series'`), and no key
/// read here has one, so splitting on whitespace cannot be fooled by a quoted fragment.
fn attribute<'a>(options: &'a str, key: &str) -> Option<&'a str> {
    options
        .split_whitespace()
        .find_map(|pair| pair.strip_prefix(key)?.strip_prefix('='))
}

/// The `printer-state*` attributes as they read RIGHT NOW, whatever their severity.
///
/// A different question from [`parse_queue_state`]'s, so deliberately a different rule: that one
/// asks "may we print?" and must ignore anything below `-error`, because a thermal roll running
/// low reports `media-low-warning` for hours while printing perfectly. This one asks "why did the
/// ticket that we already sent not come out?", and the answer is precisely the reasons the other
/// has to ignore: a USB backend never emits an `-error` reason at all — CUPS' `backend/runloop.c`
/// reports no paper as `media-empty-warning` and a pulled cable as `offline-report`, and it only
/// emits them WHILE it is running a job. Before the submit they are stale or absent; after the
/// wait they are the fault that ate the receipt.
pub fn parse_state_report(options: &str) -> Option<String> {
    let state = attribute(options, "printer-state")?;
    let mut reported = vec![format!("printer-state={}", state_name(state))];
    match attribute(options, "printer-state-reasons") {
        Some(reasons) if reasons != "none" => {
            reported.push(format!("printer-state-reasons={reasons}"))
        }
        _ => {}
    }
    if attribute(options, "printer-is-accepting-jobs") == Some("false") {
        reported.push("printer-is-accepting-jobs=false".to_string());
    }
    Some(reported.join(" "))
}

/// The job id `lp` answered with, taken out of its line WITHOUT reading a word of it.
///
/// `lp` prints `request id is <queue>-<n> (1 file(s))`, and that sentence is one of CUPS'
/// translatable strings (`_cupsLangPrintf` in `lp.c`), so on a Spanish Mac it is «el identificador
/// de la solicitud es …». Anchoring on the English prefix would find no id on exactly the tills
/// this transport exists for — the same trap `lpstat -v` sets. What no translation moves is the id
/// itself: the queue's own name, a dash, and a number.
pub fn parse_request_id(stdout: &str, queue: &str) -> Option<String> {
    let prefix = format!("{queue}-");
    stdout.split_whitespace().find_map(|token| {
        let rest = token.strip_prefix(&prefix)?;
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() {
            return None;
        }
        // A number that runs straight into more of the token is part of something else, not a job
        // id — and cancelling on a guess would take away a job that is not ours. Punctuation after
        // it is fine: CUPS puts the file count in brackets right behind.
        let tail = &rest[digits.len()..];
        if tail.starts_with(|c: char| c.is_alphanumeric() || c == '-' || c == '_') {
            return None;
        }
        Some(format!("{prefix}{digits}"))
    })
}

/// The argv that lists the jobs `queue` has NOT finished yet.
fn lpstat_queue_args(queue: &str) -> Vec<&str> {
    vec!["-W", "not-completed", "-o", queue]
}

/// Whether `job_id` is still sitting in an `lpstat -o` listing.
///
/// Matched on the FIRST COLUMN and nothing else. The rest of the line is the owner, the size and
/// a date — and the date is written in the system's language and format (`Sat Sep  5 17:17:50
/// 2026` on this Mac, `s\u{e1}b  5 sep 17:17:50 2026` on a Spanish one), so it is the one field a
/// parser must never touch. The id is what CUPS never translates.
pub fn job_is_queued(listing: &str, job_id: &str) -> bool {
    listing.lines().any(|line| line.split_whitespace().next() == Some(job_id))
}

/// Asks CUPS how `queue` is doing. One client spawn, bounded by [`CUPS_TIMEOUT`] like every other.
///
/// A failure is [`QueueState::Unknown`], logged, never an error: this is a guard in front of
/// printing, and a guard that fails closed on a machine whose CUPS answers something we did not
/// foresee would take USB printing away from a till that had it working.
fn ask_queue_state(client: &dyn CupsClient, queue: &str) -> QueueState {
    match client.run(CUPS_LPOPTIONS, &["-p", queue], &[]) {
        Ok(out) if out.success => parse_queue_state(&out.stdout),
        Ok(out) => {
            tracing::warn!(
                "usb: `{CUPS_LPOPTIONS} -p {queue}` failed ({}); printing without knowing the \
                 printer's state",
                first_meaningful_line(&out.stderr).unwrap_or("no reason given")
            );
            QueueState::Unknown
        }
        Err(e) => {
            tracing::warn!(
                "usb: could not ask CUPS about the queue `{queue}` ({e}); printing without knowing \
                 the printer's state"
            );
            QueueState::Unknown
        }
    }
}

/// Sends already-rendered ESC/POS to the printer behind an OS print queue.
///
/// Direct, not queued — the same shape Bluetooth has. [`crate::queue`] is the NETWORK path: its
/// jobs carry a socket target and its retry policy is written around a TCP connect. Widening it
/// to cover a spooler is a real change to the one piece of the print chain that already works,
/// so it is not smuggled in here. The failure stays visible either way: the
/// error comes straight back to the caller and the print host reports the job `failed`.
///
/// `lp` exits 0 the moment the SPOOLER takes the job, which is as far as it can see: a printer
/// that is out of paper, unplugged or paused still has a spooler, and it keeps the job HELD in the
/// OS queue — under the title `ERPlora`, so it can be told apart there. So the state is asked for
/// FIRST: a queue that would hold the ticket is refused here, with the IPP attributes
/// that decided it, and the print host reports the job `failed` while the customer is still
/// standing there instead of the receipt surfacing an hour later when someone reloads the paper.
///
/// The guard never fails closed: a state we could not read ([`QueueState::Unknown`]) submits
/// anyway and leaves the verdict to `lp`, which is already the one that reports the spooler
/// refusing — a queue that does not exist, is disabled or is rejecting jobs, with its own sentence
/// in it.
///
/// And asking first is not enough on its own: the pre-flight can only report a fault
/// the OS already knows about, and with the queue at rest it does not know. A USB backend finds
/// out that the roll ran out or the cable is gone only while it is *running* a job, so the FIRST
/// ticket after the paper runs out passes the guard, is accepted by `lp`, and is held. So the job
/// is also followed AFTERWARDS: it has [`JOB_COMPLETION_WINDOW`] to leave the queue, and if it is
/// still there it is cancelled and reported failed with whatever the backend is reporting by
/// then. That second half is what makes a USB printer behave like the network one — the till says
/// the ticket did not come out, instead of saying it did.
pub fn send_raw(client: &dyn CupsClient, target: &UsbTarget, payload: &[u8]) -> Result<()> {
    send_raw_within(client, target, payload, JOB_COMPLETION_WINDOW)
}

/// [`send_raw`] with the wait spelled out, so a test can exercise the timeout without spending
/// [`JOB_COMPLETION_WINDOW`] of real seconds on it.
fn send_raw_within(
    client: &dyn CupsClient,
    target: &UsbTarget,
    payload: &[u8],
    window: Duration,
) -> Result<()> {
    if let QueueState::NotReady { reason } = ask_queue_state(client, &target.queue) {
        return Err(PeripheralError::Unreachable(format!(
            "the printer on the OS print queue `{}` is not ready ({reason}); the job was NOT sent",
            target.queue
        )));
    }
    let out = client.run(CUPS_LP, &lp_args(&target.queue), payload)?;
    if !out.success {
        // `lp` says WHY on stderr ("paper out", "printer disabled", "does not exist"). Dropping it
        // would leave the cashier with a ticket that never printed and no way to know it.
        let reason = first_meaningful_line(&out.stderr)
            .or_else(|| first_meaningful_line(&out.stdout))
            .unwrap_or("no reason given");
        return Err(PeripheralError::Unreachable(format!(
            "the OS print queue `{}` refused the job: {reason}",
            target.queue
        )));
    }
    let Some(job_id) = parse_request_id(&out.stdout, &target.queue) else {
        // Same rule as the pre-flight, one step later: an answer we could not read is not a
        // verdict. Without the id there is nothing to follow and nothing safe to cancel, so the
        // job stands as sent and the reason is logged rather than swallowed.
        tracing::warn!(
            "usb: `{CUPS_LP}` took the job for `{}` without an id we could read ({:?}); it counts \
             as sent without confirming the ticket came out",
            target.queue,
            first_meaningful_line(&out.stdout).unwrap_or("nothing on stdout")
        );
        return Ok(());
    };
    confirm_job_left_queue(client, &target.queue, &job_id, window)
}

/// Waits for a job the spooler already ACCEPTED to actually leave the queue, and reports it failed
/// — after taking it back out — if it does not.
///
/// Being unable to ask is never a failure, for the same reason the pre-flight submits on
/// [`QueueState::Unknown`]: this runs on every ticket, and turning "we could not read the queue"
/// into "your receipt did not print" would cancel perfectly good tickets on every machine whose
/// CUPS answers something we did not foresee.
fn confirm_job_left_queue(
    client: &dyn CupsClient,
    queue: &str,
    job_id: &str,
    window: Duration,
) -> Result<()> {
    let deadline = Instant::now() + window;
    loop {
        match client.run(CUPS_LPSTAT, &lpstat_queue_args(queue), &[]) {
            Ok(out) if out.success => {
                if !job_is_queued(&out.stdout, job_id) {
                    return Ok(());
                }
            }
            Ok(out) => {
                tracing::warn!(
                    "usb: `{CUPS_LPSTAT} -o {queue}` failed ({}); `{job_id}` counts as sent \
                     without confirming the ticket came out",
                    first_meaningful_line(&out.stderr).unwrap_or("no reason given")
                );
                return Ok(());
            }
            Err(e) => {
                tracing::warn!(
                    "usb: could not read the queue `{queue}` ({e}); `{job_id}` counts as sent \
                     without confirming the ticket came out"
                );
                return Ok(());
            }
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        std::thread::sleep(JOB_POLL_INTERVAL.min(left));
    }

    // The reason is read BEFORE the job is cancelled, and that order is load-bearing: the backend
    // clears `media-empty-warning` / `offline-report` as it is torn down, so cancelling first
    // would leave the cashier with "the ticket did not print" and not a word about the paper.
    let reason = match client.run(CUPS_LPOPTIONS, &["-p", queue], &[]) {
        Ok(out) if out.success => parse_state_report(&out.stdout),
        _ => None,
    }
    .unwrap_or_else(|| "no reason given".to_string());

    match client.run(CUPS_CANCEL, &[job_id], &[]) {
        Ok(out) if out.success => {}
        Ok(out) => tracing::warn!(
            "usb: `{CUPS_CANCEL} {job_id}` failed ({}); the ticket the till just reported as \
             failed may still print later",
            first_meaningful_line(&out.stderr).unwrap_or("no reason given")
        ),
        Err(e) => tracing::warn!(
            "usb: could not cancel `{job_id}` ({e}); the ticket the till just reported as failed \
             may still print later"
        ),
    }

    Err(PeripheralError::Unreachable(format!(
        "the printer on the OS print queue `{queue}` still had the ticket {}s after it was sent \
         ({reason}); it was cancelled, so it did NOT come out and will not appear later",
        window.as_secs()
    )))
}

/// The first line with something in it — `lp` puts the useful sentence first and can pad the rest.
fn first_meaningful_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).find(|l| !l.is_empty())
}

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

/// The real [`CupsClient`]: spawns the client binary and waits for it, bounded by
/// [`CUPS_TIMEOUT`].
pub struct SystemCups;

impl CupsClient for SystemCups {
    fn run(&self, program: &str, args: &[&str], stdin: &[u8]) -> Result<CupsOutput> {
        run_bounded(program, args, stdin, CUPS_TIMEOUT)
    }
}

/// [`SystemCups::run`] with the deadline spelled out, so the give-up path can be exercised in
/// milliseconds instead of making the suite wait out a real [`CUPS_TIMEOUT`].
fn run_bounded(program: &str, args: &[&str], stdin: &[u8], timeout: Duration) -> Result<CupsOutput> {
    {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                PeripheralError::Unreachable(format!(
                    "could not run `{program}`: {e} — the CUPS client tools have to be installed \
                     for USB printing (macOS ships them; on Linux they are the `cups-client` package)"
                ))
            })?;

        // The payload goes in on its own thread. `lp` reads stdin to EOF, and a receipt with a
        // logo can outgrow the pipe buffer (~64 KB): writing it inline would deadlock us against a
        // child that is waiting for us to finish before it starts reading. Dropping the handle at
        // the end of the closure is the EOF `lp` waits for.
        let mut sink = child.stdin.take().ok_or_else(|| {
            PeripheralError::Unreachable(format!("`{program}` gave no stdin to write to"))
        })?;
        let payload = stdin.to_vec();
        let writer = std::thread::spawn(move || sink.write_all(&payload).and_then(|()| sink.flush()));

        // Both output pipes are drained on their own threads for the same reason the payload is
        // written on one: a child that fills a pipe buffer nobody is reading blocks there forever.
        // Draining only after the exit looks safe as long as the client answers in a line or two —
        // and then the deadline below reports "cupsd is wedged" when what actually wedged is us.
        // Measured: 512 KB through `cat` timed out at 20 s until these two threads existed.
        let out_pipe = child.stdout.take();
        let err_pipe = child.stderr.take();
        let out_reader = std::thread::spawn(move || drain(out_pipe));
        let err_reader = std::thread::spawn(move || drain(err_pipe));

        let deadline = Instant::now() + timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() >= deadline => {
                    // Killing it closes the pipe, which releases the writer thread too.
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(PeripheralError::Unreachable(format!(
                        "`{program}` did not answer in {:?} (is cupsd running?); the job was NOT sent",
                        timeout
                    )));
                }
                Ok(None) => std::thread::sleep(POLL_INTERVAL),
                Err(e) => {
                    return Err(PeripheralError::Unreachable(format!(
                        "could not wait for `{program}`: {e}"
                    )))
                }
            }
        };

        // The child is gone, so both pipes are at EOF and these joins return at once. A reader
        // that somehow panicked costs a diagnostic sentence, never the outcome — that is decided
        // by the exit status.
        let stdout = out_reader.join().unwrap_or_default();
        let stderr = err_reader.join().unwrap_or_default();

        // Precedence matters. When the child rejected the job it closed the pipe, so the writer
        // reports a broken pipe — reporting THAT would bury `lp`'s actual sentence ("paper out").
        // The write error is only the real story when the child itself was happy.
        if status.success() {
            match writer.join() {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    return Err(PeripheralError::Unreachable(format!(
                        "`{program}` accepted the job but the document could not be handed over: {e}"
                    )))
                }
                Err(_) => {
                    return Err(PeripheralError::Unreachable(format!(
                        "the thread feeding `{program}` panicked; the job may be incomplete"
                    )))
                }
            }
        }

        Ok(CupsOutput {
            success: status.success(),
            stdout,
            stderr,
        })
    }
}

/// Reads a child pipe to the end. A pipe we cannot read is reported as empty: it costs us a
/// diagnostic sentence, never the outcome, which is decided by the exit status.
fn drain(pipe: Option<impl Read>) -> String {
    let mut text = String::new();
    if let Some(mut pipe) = pipe {
        if let Err(e) = pipe.read_to_string(&mut text) {
            tracing::warn!("usb: could not read a CUPS client pipe ({e}); its output is lost");
        }
    }
    text
}

#[cfg(test)]
mod tests;
