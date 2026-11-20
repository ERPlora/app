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
//!
//! The pieces: [`cups`] runs the CUPS client tools, [`state`] reads what a queue says about
//! itself, [`discovery`] lists the USB queues, and this module sends a ticket and follows it
//! until it has left the queue.

pub mod cups;
pub mod discovery;
pub mod state;

#[cfg(test)]
mod fake_cups;

use std::time::{Duration, Instant};

use self::cups::{CupsClient, CUPS_CANCEL, CUPS_LP, CUPS_LPOPTIONS, CUPS_LPSTAT};
use self::state::{ask_queue_state, parse_state_report, QueueState};
use crate::print_target::UsbTarget;
use crate::{PeripheralError, Result};

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

/// The argv for submitting raw bytes to `queue`.
///
/// `-o raw` is the load-bearing flag: it marks the job `application/vnd.cups-raw`, which tells
/// CUPS to skip its filter chain and hand the bytes to the backend untouched. Without it the
/// spooler would try to *render* our ESC/POS as if it were a document, and what comes out of the
/// printer is a page of garbage instead of a receipt.
pub fn lp_args(queue: &str) -> Vec<&str> {
    vec!["-d", queue, "-t", JOB_TITLE, "-o", "raw"]
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

#[cfg(test)]
mod tests;
