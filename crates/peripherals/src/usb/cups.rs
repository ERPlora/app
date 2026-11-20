//! The CUPS client tools USB printing runs, and the runner that spawns them with a deadline.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

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

/// How long a CUPS client gets before we give up on it.
///
/// Not a formality: when `cupsd` wedges, `lp` blocks on its socket and never returns. Without a
/// deadline that is a till frozen on the press that prints the ticket — the cashier's only way out
/// being to kill the app. Twenty seconds is far beyond a healthy submit (milliseconds) and still
/// short enough that the error reaches the screen while the customer is still standing there.
pub const CUPS_TIMEOUT: Duration = Duration::from_secs(20);

/// How often the wait wakes up to check on the child.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

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
mod tests {
    use super::*;

    // ── The REAL runner ──────────────────────────────────────────────────────────────────────
    //
    // `SystemCups` is the half no fake can vouch for: spawning, feeding stdin, the deadline, a
    // binary that is not there. Exercised against `cat`/`sh`/`sleep` rather than `lp`, because a
    // test that really submitted a job would either print on the developer's desk or fail on every
    // machine without a printer — and neither outcome would mean anything.

    #[test]
    fn the_real_runner_feeds_stdin_and_brings_the_answer_back() {
        let out = run_bounded("cat", &[], b"\x1b@ticket", CUPS_TIMEOUT).expect("`cat` is everywhere");

        assert!(out.success);
        assert_eq!(out.stdout.as_bytes(), b"\x1b@ticket", "the bytes must pass through untouched");
    }

    #[test]
    fn a_payload_bigger_than_the_pipe_buffer_does_not_deadlock() {
        // THE reason the payload is written on its own thread. A receipt carrying a logo outgrows
        // the ~64 KB pipe buffer; writing it inline would block us against a child that is itself
        // blocked waiting for us to stop writing, and the till would hang on the press that
        // prints. 512 KB is comfortably past any platform's buffer.
        let payload = vec![b'A'; 512 * 1024];

        let out = run_bounded("cat", &[], &payload, CUPS_TIMEOUT).expect("must not hang");

        assert!(out.success);
        assert_eq!(out.stdout.len(), payload.len(), "the whole document has to get through");
    }

    #[test]
    fn a_nonzero_exit_is_reported_instead_of_passing_for_sent() {
        let out = run_bounded("sh", &["-c", "echo nope >&2; exit 3"], b"", CUPS_TIMEOUT)
            .expect("the process ran, it just refused");

        assert!(!out.success, "exit 3 is not a sent job");
        assert!(out.stderr.contains("nope"), "the reason has to survive, got: {:?}", out.stderr);
    }

    #[test]
    fn a_wedged_client_is_given_up_on_rather_than_hanging_the_till() {
        // When `cupsd` wedges, `lp` blocks on its socket and never returns. Without the deadline
        // that is a till frozen on the press that prints, with no way out but killing the app.
        let started = Instant::now();

        let err = run_bounded("sleep", &["30"], b"", Duration::from_millis(150))
            .expect_err("a client that never answers must not read as success");

        assert!(started.elapsed() < Duration::from_secs(5), "it waited out the whole sleep");
        assert!(err.to_string().contains("NOT sent"), "must be explicit, got: {err}");
    }

    #[test]
    fn a_missing_cups_client_says_so_instead_of_blaming_the_printer() {
        // Linux without `cups-client` installed. "Install the CUPS tools" and "check the printer"
        // are opposite instructions, so the error has to point at the right one.
        let err = run_bounded("erplora-no-such-binary", &[], b"", CUPS_TIMEOUT)
            .expect_err("a binary that is not there cannot have printed");

        assert!(err.to_string().contains("cups-client"), "must name the fix, got: {err}");
    }
}
