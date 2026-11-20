//! What a print queue says about itself, read from the IPP attributes `lpoptions` prints.

use super::cups::{CupsClient, CUPS_LPOPTIONS};
use super::first_meaningful_line;

/// IPP `printer-state` (RFC 8011 section 5.4.11), the whole set — there is no fourth value.
const IPP_STATE_IDLE: &str = "3";
const IPP_STATE_PROCESSING: &str = "4";
const IPP_STATE_STOPPED: &str = "5";

/// The IPP severity suffix that means "this printer cannot print now" (RFC 8011 section 5.4.12).
/// `-warning` and `-report` are the other two, and they must NOT stop a till: a thermal roll
/// running low reports `media-low-warning` for hours while printing perfectly.
const REASON_ERROR_SUFFIX: &str = "-error";

/// What the OS says about a print queue, in IPP's own vocabulary — never in its prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueState {
    /// Idle or printing, and taking work: a submit reaches the paper.
    Ready,
    /// The spooler would accept the job and HOLD it. `reason` carries the attributes that decided
    /// it, verbatim and untranslated, because that is what tells the cashier WHICH fault to fix.
    NotReady { reason: String },
    /// We could not ask. Deliberately not a refusal — see [`send_raw`].
    ///
    /// [`send_raw`]: super::send_raw
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

/// Asks CUPS how `queue` is doing. One client spawn, bounded by [`CUPS_TIMEOUT`] like every other.
///
/// A failure is [`QueueState::Unknown`], logged, never an error: this is a guard in front of
/// printing, and a guard that fails closed on a machine whose CUPS answers something we did not
/// foresee would take USB printing away from a till that had it working.
///
/// [`CUPS_TIMEOUT`]: super::cups::CUPS_TIMEOUT
pub(super) fn ask_queue_state(client: &dyn CupsClient, queue: &str) -> QueueState {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_real_lpoptions_output_of_this_machine_parses() {
        // Both captured verbatim on macOS: the first from the Mac's own printer, the
        // second from a scratch CUPS queue stopped with `cupsdisable -r "out of paper"`. Pinning
        // REAL output is what keeps the parser honest about the shape CUPS emits — note the
        // single-quoted values with spaces (`printer-info='Brother HL-3150CDW series'`), which a
        // hand-written fixture would not have thought of.
        let idle = "copies=1 device-uri=dnssd://Brother%20HL-3150CDW%20series._ipp._tcp.local./?uuid=e3248000-80ce-11db-8000-3c2af45a10ef finishings=3 job-cancel-after=10800 job-hold-until=no-hold job-priority=50 job-sheets=none,none marker-change-time=1786119370 marker-high-levels=100,100,100,100 marker-levels=100,50,50,100 marker-low-levels=10,10,10,10 marker-types=toner,toner,toner,toner number-up=1 printer-commands=none printer-info='Brother HL-3150CDW series' printer-is-accepting-jobs=true printer-is-shared=false printer-is-temporary=false printer-location printer-make-and-model='Brother HL-3150CDW series-AirPrint' printer-state=3 printer-state-change-time=1786120244 printer-state-reasons=none printer-type=2134044 printer-uri-supported=ipp://localhost/printers/Brother_HL_3150CDW_series";
        let stopped = "copies=1 device-uri=file:///dev/null finishings=3 job-cancel-after=10800 job-hold-until=no-hold job-priority=50 job-sheets=none,none marker-change-time=0 number-up=1 printer-commands=AutoConfigure,Clean,PrintSelfTestPage printer-info=erplora_probe_1541 printer-is-accepting-jobs=true printer-is-shared=true printer-is-temporary=false printer-location printer-make-and-model='Generic PostScript Printer' printer-state=5 printer-state-change-time=1788609721 printer-state-reasons=paused printer-type=8400972 printer-uri-supported=ipp://localhost/printers/erplora_probe_1541";

        assert_eq!(parse_queue_state(idle), QueueState::Ready);
        assert_eq!(
            parse_queue_state(stopped),
            QueueState::NotReady {
                reason: "printer-state=stopped printer-state-reasons=paused".into()
            },
            "a stopped queue holds the ticket, whatever the OS says in prose"
        );
    }

    #[test]
    fn the_state_is_read_as_keywords_never_as_the_sentence_lpstat_prints() {
        // The trap this transport keeps walking into: `lpstat -p` says "printer X is idle" in
        // English, «la impresora X está inactiva» in Spanish, and macOS picks that language from
        // `AppleLanguages`, so `LC_ALL=C` does not help. `lpoptions` prints the IPP attributes as
        // `key=value` — the enum and the keywords are the same in every language, which is the
        // whole reason it is the one being asked.
        assert_eq!(
            parse_queue_state("printer-is-accepting-jobs=true printer-state=5 printer-state-reasons=media-empty-error,cover-open-error"),
            QueueState::NotReady {
                reason: "printer-state=stopped printer-state-reasons=media-empty-error,cover-open-error".into()
            },
            "every reason has to reach the cashier: one fixed fault out of two is still no ticket"
        );
        assert_eq!(parse_queue_state("printer-state=3"), QueueState::Ready);
        assert_eq!(parse_queue_state("printer-state-reasons=none"), QueueState::Unknown);
        assert_eq!(parse_queue_state(""), QueueState::Unknown);
    }

    #[test]
    fn the_report_carries_exactly_the_reasons_the_pre_flight_has_to_ignore() {
        // The two rules are deliberately different, and this is the pair that shows why: the same
        // attributes mean "keep printing" before the submit and "this is what ate the ticket"
        // after the wait. A USB backend emits no `-error` reason at all, so a post-mortem written
        // with the pre-flight's rule would report every held ticket as "no reason given".
        let printing_with_no_paper =
            "printer-is-accepting-jobs=true printer-state=4 printer-state-reasons=media-empty-warning";

        assert_eq!(
            parse_queue_state(printing_with_no_paper),
            QueueState::Ready,
            "a warning must never stop a till before it has even tried"
        );
        assert_eq!(
            parse_state_report(printing_with_no_paper).as_deref(),
            Some("printer-state=processing printer-state-reasons=media-empty-warning")
        );
        assert_eq!(
            parse_state_report("printer-is-accepting-jobs=false printer-state=5 printer-state-reasons=none").as_deref(),
            Some("printer-state=stopped printer-is-accepting-jobs=false")
        );
        // Nothing to report is not "everything is fine": it is the absence of an answer, and the
        // caller says "no reason given" rather than inventing one.
        assert_eq!(parse_state_report("copies=1 number-up=1"), None);
        assert_eq!(parse_state_report(""), None);
    }
}
