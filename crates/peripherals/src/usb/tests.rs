use super::*;
use std::cell::RefCell;

/// `(program, args, stdin)` of every call the fake was asked to make.
type RecordedCalls = Vec<(String, Vec<String>, Vec<u8>)>;

/// `(args, answers)` for an argv asked more than once — see [`FakeCups::sequenced`].
type ScriptedSequence = Vec<(Vec<&'static str>, Vec<CupsOutput>)>;

/// Records what would have been run, and answers whatever the test wants it to — per argv,
/// so a discovery that asks two questions (`-e`, then `-v`) can be given two answers.
struct FakeCups {
    /// `(args, answer)`; an empty `args` answers any call.
    answers: Vec<(Vec<&'static str>, CupsOutput)>,
    /// `(args, answers)` for an argv that is asked MORE THAN ONCE and has to answer
    /// differently each time — the shape the completion wait needs, where the queue listing
    /// must change between polls for the wait to be provably a wait and not a single look.
    /// Consumed in order, the last one keeps answering, and it is checked before
    /// [`FakeCups::answers`].
    sequenced: RefCell<ScriptedSequence>,
    calls: RefCell<RecordedCalls>,
}

impl Default for FakeCups {
    fn default() -> Self {
        Self {
            answers: Vec::new(),
            sequenced: RefCell::new(Vec::new()),
            calls: RefCell::new(Vec::new()),
        }
    }
}

impl FakeCups {
    fn answering(answer: CupsOutput) -> Self {
        Self { answers: vec![(Vec::new(), answer)], ..Default::default() }
    }
    fn ok() -> Self {
        Self::answering(CupsOutput {
            success: true,
            stdout: "request id is Star_TSP143-7 (1 file(s))".into(),
            stderr: String::new(),
        })
    }
    /// A CUPS whose `lpoptions -p <queue>` answers `state` and whose `lp` accepts the job.
    /// The pre-flight asks the first question before it dares the second.
    fn queue_state(queue: &'static str, state: &str) -> Self {
        let ok = |stdout: &str| CupsOutput {
            success: true,
            stdout: stdout.into(),
            stderr: String::new(),
        };
        Self {
            answers: vec![
                (vec!["-p", queue], ok(state)),
                // And the job leaves the queue, which is what "it printed" means.
                (vec!["-W", "not-completed", "-o", queue], ok("")),
                (Vec::new(), ok("request id is Star_TSP143-7 (1 file(s))")),
            ],
            ..Default::default()
        }
    }

    /// The whole conversation for one submit and its wait: `lpoptions -p <queue>` answers
    /// `states` in order (idle at the pre-flight, then whatever the backend reports once it
    /// has *tried* to print), `lp` accepts the job as `<queue>-7`, and
    /// `lpstat -W not-completed -o <queue>` answers `listings` in order, one per poll. The
    /// last of each keeps answering, so a listing that never clears is a job that never left.
    fn spooling(
        queue: &'static str,
        states: Vec<&'static str>,
        listings: Vec<&'static str>,
    ) -> Self {
        let ok = |stdout: &str| CupsOutput {
            success: true,
            stdout: stdout.into(),
            stderr: String::new(),
        };
        let sequence =
            |outs: Vec<&'static str>| outs.into_iter().map(ok).collect::<Vec<_>>();
        Self {
            answers: vec![(
                Vec::new(),
                ok(&format!("request id is {queue}-7 (1 file(s))")),
            )],
            sequenced: RefCell::new(vec![
                (vec!["-p", queue], sequence(states)),
                (vec!["-W", "not-completed", "-o", queue], sequence(listings)),
            ]),
            ..Default::default()
        }
    }

    /// A healthy CUPS whose `lpstat -e` prints `destinations` and whose `lpstat -v` prints
    /// `devices`.
    fn listing(destinations: &str, devices: &str) -> Self {
        let ok = |stdout: &str| CupsOutput {
            success: true,
            stdout: stdout.into(),
            stderr: String::new(),
        };
        Self {
            answers: vec![(vec!["-e"], ok(destinations)), (vec!["-v"], ok(devices))],
            ..Default::default()
        }
    }
}

impl CupsClient for FakeCups {
    fn run(&self, program: &str, args: &[&str], stdin: &[u8]) -> Result<CupsOutput> {
        self.calls.borrow_mut().push((
            program.to_string(),
            args.iter().map(|a| a.to_string()).collect(),
            stdin.to_vec(),
        ));
        if let Some((_, queued)) = self
            .sequenced
            .borrow_mut()
            .iter_mut()
            .find(|(for_args, _)| for_args.as_slice() == args)
        {
            let answer = queued.first().cloned().expect("a sequence answers at least once");
            if queued.len() > 1 {
                queued.remove(0);
            }
            return Ok(answer);
        }
        let scripted = self
            .answers
            .iter()
            .find(|(for_args, _)| for_args.is_empty() || for_args.as_slice() == args)
            .map(|(_, answer)| answer.clone());
        Ok(scripted.unwrap_or_else(|| CupsOutput {
            success: false,
            stdout: String::new(),
            stderr: format!("fake cups: nothing scripted for `{program} {args:?}`"),
        }))
    }
}

fn a_queue() -> UsbTarget {
    UsbTarget { queue: "Star_TSP143".into() }
}

/// A successful CUPS answer carrying `stdout`.
fn cups_said(stdout: &str) -> CupsOutput {
    CupsOutput { success: true, stdout: stdout.into(), stderr: String::new() }
}

#[test]
fn a_raw_cups_queue_receives_the_escpos_bytes() {
    let cups = FakeCups::ok();
    // A cut command: the bytes must arrive byte-for-byte, not re-encoded as text.
    let payload = b"\x1b@Hello\n\x1dV\x00".to_vec();

    send_raw(&cups, &a_queue(), &payload).expect("a healthy queue accepts the job");

    let calls = cups.calls.borrow();
    let submits: Vec<_> = calls.iter().filter(|(program, _, _)| program == CUPS_LP).collect();
    assert_eq!(submits.len(), 1, "exactly one submit per document");
    let (program, args, stdin) = submits[0];
    assert_eq!(program, CUPS_LP);
    assert_eq!(stdin, &payload, "the ESC/POS must reach the spooler untouched");
    assert!(
        args.windows(2).any(|w| w == ["-d", "Star_TSP143"]),
        "the job must be addressed to the queue, got {args:?}"
    );
    assert!(
        args.windows(2).any(|w| w == ["-o", "raw"]),
        "without `-o raw` CUPS renders the ESC/POS as a document and prints garbage, got {args:?}"
    );
}

#[test]
fn a_refused_job_comes_back_as_an_error_naming_the_queue_and_the_reason() {
    // A queue that does not exist, is disabled or is rejecting jobs: `lp` exits non-zero and
    // says why, and that sentence is the only thing standing between the cashier and a ticket
    // that silently never printed, so it has to survive into the error. (Paper out or a pulled
    // cable are NOT this case: the spooler accepts the job and holds it — see `send_raw`.)
    let cups = FakeCups::answering(CupsOutput {
        success: false,
        stdout: String::new(),
        stderr: "lp: Error - The printer or class does not exist.".into(),
    });

    let err = send_raw(&cups, &a_queue(), b"ticket").expect_err("a refusal must not read as sent");

    let shown = err.to_string();
    assert!(shown.contains("Star_TSP143"), "must name the queue, got: {shown}");
    assert!(shown.contains("does not exist"), "must carry lp's reason, got: {shown}");
}

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

// ── The pre-flight ───────────────────────────────────────────────────────────────────────
//
// `lp` exits 0 the moment the SPOOLER takes the job, and a printer that is out of paper,
// paused or unplugged still has a spooler. Measured on macOS against a real CUPS
// queue (`cupsdisable -r "out of paper"`): `lp -d … -o raw` answered `request id is …-9` with
// exit 0 and the ticket sat in the OS queue. So the state has to be ASKED for, before.

/// A CUPS answer for a queue that is idle and taking work.
const LPOPTIONS_IDLE: &str =
    "printer-is-accepting-jobs=true printer-state=3 printer-state-reasons=none";

#[test]
fn a_stopped_queue_is_refused_instead_of_letting_the_os_hold_the_ticket() {
    // The bug itself: paper out, cover open or the cable pulled leaves the queue stopped, `lp`
    // says yes, and the cashier watches a ticket that never comes. Refusing here is what turns
    // it into a `failed` job the print host reports while the customer is still standing there.
    let cups = FakeCups::queue_state(
        "Star_TSP143",
        "printer-is-accepting-jobs=true printer-state=5 printer-state-reasons=media-empty-error",
    );

    let err = send_raw(&cups, &a_queue(), b"ticket")
        .expect_err("a printer with no paper has not printed anything");

    let shown = err.to_string();
    assert!(shown.contains("Star_TSP143"), "must name the queue, got: {shown}");
    assert!(
        shown.contains("media-empty-error"),
        "the IPP keyword is the reason, untranslated — it is what says WHY, got: {shown}"
    );
    assert!(shown.contains("stopped"), "the state has to be readable, got: {shown}");
    assert!(
        !cups.calls.borrow().iter().any(|(program, _, _)| program == CUPS_LP),
        "the whole point is that the job never reaches the spooler: {:?}",
        cups.calls.borrow()
    );
}

#[test]
fn a_queue_that_is_not_accepting_jobs_is_refused_before_the_submit() {
    // `lp` would refuse this one too, in the system's language. Catching it here gives the same
    // machine-readable answer as every other not-ready case instead of two different shapes.
    let cups = FakeCups::queue_state(
        "Star_TSP143",
        "printer-is-accepting-jobs=false printer-state=3 printer-state-reasons=none",
    );

    let err = send_raw(&cups, &a_queue(), b"ticket").expect_err("a rejecting queue has not printed");

    let shown = err.to_string();
    assert!(
        shown.contains("printer-is-accepting-jobs=false"),
        "must carry the attribute that decided it, got: {shown}"
    );
    assert!(!cups.calls.borrow().iter().any(|(program, _, _)| program == CUPS_LP));
}

#[test]
fn an_error_severity_reason_blocks_even_while_the_queue_still_reads_idle() {
    // A backend can flag the fault before cupsd stops the queue. The `-error` suffix is the
    // IPP severity that means "this printer cannot print now" (RFC 8011 section 5.4.12), and it
    // is a keyword, so it says the same thing on a Spanish Mac as on an English one.
    let cups = FakeCups::queue_state(
        "Star_TSP143",
        "printer-is-accepting-jobs=true printer-state=3 printer-state-reasons=cover-open-error",
    );

    let err = send_raw(&cups, &a_queue(), b"ticket").expect_err("an open cover prints nothing");

    assert!(err.to_string().contains("cover-open-error"), "got: {err}");
    assert!(!cups.calls.borrow().iter().any(|(program, _, _)| program == CUPS_LP));
}

#[test]
fn a_warning_or_a_report_does_not_stop_a_printer_that_still_prints() {
    // The other half of the severity rule, and the one that decides whether this guard is
    // usable at all: a till whose thermal roll is running low must keep printing. Blocking on
    // every reason would turn a healthy printer into a dead one, which is worse than the bug.
    let cups = FakeCups::queue_state(
        "Star_TSP143",
        "printer-is-accepting-jobs=true printer-state=3 \
         printer-state-reasons=media-low-warning,toner-low-warning,cups-waiting-for-job-completed",
    );

    send_raw(&cups, &a_queue(), b"ticket").expect("a low roll still prints");

    assert!(cups.calls.borrow().iter().any(|(program, _, _)| program == CUPS_LP), "it must submit");
}

#[test]
fn a_queue_in_the_middle_of_a_job_is_not_mistaken_for_a_stopped_one() {
    // `printer-state=4` is "processing": the printer is working. Two tickets in a row is the
    // normal case at a till, so reading it as not-ready would refuse every second receipt.
    let cups = FakeCups::queue_state(
        "Star_TSP143",
        "printer-is-accepting-jobs=true printer-state=4 printer-state-reasons=none",
    );

    send_raw(&cups, &a_queue(), b"ticket").expect("a busy printer takes the next ticket");

    assert!(cups.calls.borrow().iter().any(|(program, _, _)| program == CUPS_LP));
}

#[test]
fn a_state_we_could_not_read_still_gets_the_ticket() {
    // `lpoptions -p <queue>` against a destination CUPS does not know exits 0 and prints
    // uninitialised bytes with no attribute in them (measured on macOS). "We could
    // not ask" is not "it is broken": turning it into a refusal would take USB printing away
    // from every till whose CUPS answers something we did not foresee. It submits, and `lp`
    // remains the one that decides — it is the one that already reports a queue that is gone.
    for unreadable in ["", "Ph\u{fffd}\u{fffd}", "copies=1 number-up=1"] {
        let cups = FakeCups::queue_state("Star_TSP143", unreadable);

        send_raw(&cups, &a_queue(), b"ticket")
            .unwrap_or_else(|e| panic!("an unreadable state must not block the till: {e}"));

        assert!(
            cups.calls.borrow().iter().any(|(program, _, _)| program == CUPS_LP),
            "no submit for {unreadable:?}"
        );
    }
}

#[test]
fn the_preflight_asks_lpoptions_about_that_one_queue() {
    // Pinned because the WHICH matters: `lpoptions` is in `cups-client`, the same package as
    // `lp` and `lpstat`, so the guard adds no dependency the transport did not already have —
    // and its answer is `key=value`, which is the only reason this is not another prose parser.
    let cups = FakeCups::queue_state("Star_TSP143", LPOPTIONS_IDLE);

    send_raw(&cups, &a_queue(), b"ticket").expect("an idle queue prints");

    let calls = cups.calls.borrow();
    let asked = &calls[0];
    assert_eq!(asked.0, CUPS_LPOPTIONS, "the state is asked for FIRST, got {calls:?}");
    assert_eq!(asked.1, vec!["-p".to_string(), "Star_TSP143".to_string()]);
}

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

// ── The ticket has to LEAVE the queue ────────────────────────────────────────────────────

#[test]
fn a_ticket_the_os_queue_still_holds_is_not_reported_as_printed() {
    // The bug: the OS only finds out that the paper ran out WHILE it is running a job, so at
    // submit time the pre-flight sees an idle queue and says yes. `lp` exits 0,
    // the till says "printed", and the ticket sits in the OS queue until someone reloads the
    // roll hours later and it prints by surprise.
    let cups = FakeCups::spooling(
        "Star_TSP143",
        vec![
            "printer-is-accepting-jobs=true printer-state=3 printer-state-reasons=none",
            "printer-is-accepting-jobs=true printer-state=4 \
             printer-state-reasons=media-empty-warning",
        ],
        vec!["Star_TSP143-7 cashier 1024 Sat Sep  5 17:17:50 2026"],
    );

    let err = send_raw_within(&cups, &a_queue(), b"ticket", Duration::ZERO)
        .expect_err("a ticket still sitting in the queue has not come out of the printer");

    let shown = err.to_string();
    assert!(shown.contains("Star_TSP143"), "must name the queue, got: {shown}");
    assert!(
        shown.contains("media-empty-warning"),
        "the reason the backend is reporting NOW is what says WHY, got: {shown}"
    );
    // And the held job is taken back out: leaving it there is a receipt that prints hours
    // later, with a number the till already reported as failed.
    assert!(
        cups.calls
            .borrow()
            .iter()
            .any(|(program, args, _)| program == CUPS_CANCEL && args == &["Star_TSP143-7"]),
        "the held job must be cancelled, got {:?}",
        cups.calls.borrow()
    );
}

#[test]
fn a_pulled_cable_is_named_by_the_reason_the_backend_reports_while_it_tries() {
    // The other reason a USB backend emits (CUPS `backend/runloop.c`), and the other half of
    // why the pre-flight cannot catch this: `offline-report` is severity *report*, so refusing
    // on it up front would fail closed — the backend only clears it while running a job, so a
    // till with the cable already back in would never print again.
    let cups = FakeCups::spooling(
        "Star_TSP143",
        vec![
            "printer-is-accepting-jobs=true printer-state=3 printer-state-reasons=none",
            "printer-is-accepting-jobs=true printer-state=4 \
             printer-state-reasons=offline-report,connecting-to-device",
        ],
        vec!["Star_TSP143-7 cashier 1024 Sat Sep  5 17:17:50 2026"],
    );

    let err = send_raw_within(&cups, &a_queue(), b"ticket", Duration::ZERO)
        .expect_err("an unplugged printer prints nothing");

    assert!(err.to_string().contains("offline-report"), "got: {err}");
}

#[test]
fn a_ticket_that_leaves_the_queue_is_a_printed_ticket() {
    // The healthy path, and the one that decides whether this is usable at all: a receipt that
    // came out must not be cancelled, reported failed, or reprinted.
    let cups = FakeCups::spooling(
        "Star_TSP143",
        vec!["printer-is-accepting-jobs=true printer-state=3 printer-state-reasons=none"],
        vec![""],
    );

    send_raw(&cups, &a_queue(), b"ticket").expect("the job left the queue: it printed");

    assert!(
        !cups.calls.borrow().iter().any(|(program, _, _)| program == CUPS_CANCEL),
        "a printed ticket must never be cancelled: {:?}",
        cups.calls.borrow()
    );
}

#[test]
fn the_wait_keeps_looking_until_the_ticket_is_gone() {
    // A receipt is not out of the queue the instant `lp` returns; the backend still has to
    // open the device and push the bytes. Looking once and giving up would report every
    // ticket failed and cancel it mid-print, which is worse than the bug. This one goes
    // through `send_raw` itself, so the real `JOB_COMPLETION_WINDOW` wiring is exercised too.
    let cups = FakeCups::spooling(
        "Star_TSP143",
        vec!["printer-is-accepting-jobs=true printer-state=3 printer-state-reasons=none"],
        vec!["Star_TSP143-7 cashier 1024 Sat Sep  5 17:17:50 2026", ""],
    );

    send_raw(&cups, &a_queue(), b"ticket").expect("the second look found it gone");

    let looks = cups
        .calls
        .borrow()
        .iter()
        .filter(|(program, args, _)| program == CUPS_LPSTAT && args.contains(&"-o".to_string()))
        .count();
    assert!(looks >= 2, "it has to keep asking, it only asked {looks} time(s)");
    assert!(!cups.calls.borrow().iter().any(|(program, _, _)| program == CUPS_CANCEL));
}

#[test]
fn the_fault_is_read_before_the_job_is_taken_out_of_the_queue() {
    // Load-bearing order: the backend clears `media-empty-warning` as it is torn down, so
    // cancelling first would leave the cashier with "it did not print" and no word about the
    // paper — which is the one thing they need to fix it.
    let cups = FakeCups::spooling(
        "Star_TSP143",
        vec![
            "printer-is-accepting-jobs=true printer-state=3 printer-state-reasons=none",
            "printer-is-accepting-jobs=true printer-state=4 \
             printer-state-reasons=media-empty-warning",
        ],
        vec!["Star_TSP143-7 cashier 1024 Sat Sep  5 17:17:50 2026"],
    );

    send_raw_within(&cups, &a_queue(), b"ticket", Duration::ZERO).expect_err("held");

    let calls = cups.calls.borrow();
    let asked = calls.iter().rposition(|(program, _, _)| program == CUPS_LPOPTIONS);
    let cancelled = calls.iter().position(|(program, _, _)| program == CUPS_CANCEL);
    assert!(
        asked < cancelled,
        "the reason has to be read while the backend is still reporting it: {calls:?}"
    );
}

#[test]
fn a_queue_we_could_not_read_does_not_turn_a_printed_ticket_into_a_failure() {
    // This runs on EVERY ticket, so "we could not ask" must not become "your receipt did not
    // print": that would cancel good jobs on every machine whose CUPS answers something we
    // did not foresee — the same rule the pre-flight follows one step earlier.
    let cups = FakeCups {
        answers: vec![
            (
                vec!["-p", "Star_TSP143"],
                cups_said("printer-is-accepting-jobs=true printer-state=3 printer-state-reasons=none"),
            ),
            (vec!["-d", "Star_TSP143", "-t", JOB_TITLE, "-o", "raw"], cups_said("request id is Star_TSP143-7 (1 file(s))")),
            // Nothing scripted for the listing: the fake answers a failure, like a CUPS that
            // cannot be asked.
        ],
        ..Default::default()
    };

    send_raw_within(&cups, &a_queue(), b"ticket", Duration::ZERO)
        .expect("an unreadable queue must not fail a ticket that probably printed");

    assert!(!cups.calls.borrow().iter().any(|(program, _, _)| program == CUPS_CANCEL));
}

#[test]
fn an_answer_from_lp_without_an_id_we_can_read_still_counts_as_sent() {
    // Without the id there is nothing to follow and nothing safe to cancel — cancelling on a
    // guess would take out a job that is not ours. It stands as sent, and the log says why.
    let cups = FakeCups {
        answers: vec![
            (
                vec!["-p", "Star_TSP143"],
                cups_said("printer-is-accepting-jobs=true printer-state=3 printer-state-reasons=none"),
            ),
            (Vec::new(), cups_said("")),
        ],
        ..Default::default()
    };

    send_raw_within(&cups, &a_queue(), b"ticket", Duration::ZERO).expect("nothing to follow");

    let calls = cups.calls.borrow();
    assert!(
        !calls.iter().any(|(program, args, _)| program == CUPS_LPSTAT
            || program == CUPS_CANCEL
            || args.contains(&"not-completed".to_string())),
        "there is no job to look for: {calls:?}"
    );
}

#[test]
fn the_job_id_is_found_whatever_language_lp_answers_in() {
    // `request id is %s-%d (%d file(s))` is a translatable CUPS string (`_cupsLangPrintf` in
    // `lp.c`), so a parser anchored on the English words finds no id on a Spanish till — the
    // exact trap `lpstat -v` sets. The id itself is never translated.
    for answer in [
        "request id is Star_TSP143-7 (1 file(s))",
        "el identificador de la solicitud es Star_TSP143-7 (1 archivo(s))",
        "l\u{2019}identifiant de la requ\u{ea}te est Star_TSP143-7 (1 fichier(s))",
        "\u{30ea}\u{30af}\u{30a8}\u{30b9}\u{30c8} ID \u{306f} Star_TSP143-7 (1 \u{30d5}\u{30a1}\u{30a4}\u{30eb})",
    ] {
        assert_eq!(
            parse_request_id(answer, "Star_TSP143").as_deref(),
            Some("Star_TSP143-7"),
            "no id in {answer:?}"
        );
    }
    // Nothing to follow rather than a guess: a wrong id would cancel someone else's job.
    assert_eq!(parse_request_id("", "Star_TSP143"), None);
    assert_eq!(parse_request_id("lp: accepted", "Star_TSP143"), None);
    assert_eq!(parse_request_id("request id is Star_TSP143-", "Star_TSP143"), None);
    assert_eq!(parse_request_id("about Star_TSP143-7b", "Star_TSP143"), None);
    // A queue whose own name ends in a number still yields its id, not a truncation of it.
    assert_eq!(
        parse_request_id("request id is Bar-2-31 (1 file(s))", "Bar-2").as_deref(),
        Some("Bar-2-31")
    );
}

#[test]
fn the_queue_listing_is_matched_on_the_id_never_on_the_translated_date() {
    // The listing is `<id> <owner> <size> <date>`, and only the date is written in the
    // system's language and format. Matching anywhere but the first column is how this parser
    // would start lying on a Spanish Mac.
    let english = "Star_TSP143-7 cashier 1024 Sat Sep  5 17:17:50 2026\n";
    let spanish = "Star_TSP143-7 cajera 1024 s\u{e1}b  5 sep 17:17:50 2026\n";
    for listing in [english, spanish] {
        assert!(job_is_queued(listing, "Star_TSP143-7"), "not found in {listing:?}");
    }
    assert!(!job_is_queued("", "Star_TSP143-7"), "an empty queue holds nothing");
    // Another till's job on the same queue is not ours, and a prefix is not an id.
    assert!(!job_is_queued("Star_TSP143-8 cashier 1024 Sat Sep  5 17:17:50 2026\n", "Star_TSP143-7"));
    assert!(!job_is_queued("Star_TSP143-70 cashier 1024 Sat Sep  5 17:17:50 2026\n", "Star_TSP143-7"));
    // The id has to be the FIRST column: naming it anywhere else is not being in the queue.
    assert!(!job_is_queued("cancelled Star_TSP143-7 by cashier\n", "Star_TSP143-7"));
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

#[test]
fn the_real_lp_and_lpstat_output_of_this_machine_parse() {
    // Captured verbatim on macOS against the Mac's own CUPS queue, with a job
    // parked by `lp -H hold` and then removed with `cancel`. Pinning REAL output is what keeps
    // these parsers honest about the shape CUPS emits — the double space in `Sep  5` and the
    // `(1 file(s))` tail are the kind of detail a hand-written fixture does not think of.
    let accepted = "request id is Brother_HL_3150CDW_series-10 (1 file(s))\n";
    let listing = "Brother_HL_3150CDW_series-10 ioan.beilic       1024   Sat Sep  5 17:17:50 2026\n";

    let id = parse_request_id(accepted, "Brother_HL_3150CDW_series")
        .expect("`lp` names the job it just took");
    assert_eq!(id, "Brother_HL_3150CDW_series-10");
    assert!(job_is_queued(listing, &id), "the job it just took is the one in the queue");
    // `cancel` empties it; `lpstat` then prints nothing at all and exits 0 (measured).
    assert!(!job_is_queued("", &id));
}

#[test]
fn the_window_outlasts_the_backends_own_wait_after_the_last_byte() {
    // CUPS' USB backend does not release a job when the last byte is written. On Linux
    // (`backend/usb-libusb.c`) it signals its read thread and waits `WAIT_EOF_DELAY` = 7 s for
    // it, then one more second — and that thread is sitting in a 60 s bulk read on a printer
    // that has nothing to say, which is every bidirectional receipt printer without the
    // `unidir` quirk (Epson TM 04b8:0202, Bixolon, Citizen; Star is quirked `unidir`). macOS
    // (`backend/usb-darwin.c`) waits the same 7 + 1 s. So a ticket that already came out of
    // the printer is still "not completed" ~8 s later, and a window under that cancels it and
    // tells the cashier it did not print: the duplicate receipt this transport must never
    // cause. The hub is shipped on Linux (AppImage/deb), so this is not a hypothetical till.
    const CUPS_USB_POST_WRITE_TAIL: Duration = Duration::from_secs(8);
    // ...plus the print itself: a long kitchen ticket or a receipt with a logo on a 100 mm/s
    // printer is a few seconds of the backend blocked on the printer's buffer.
    const A_LONG_RECEIPT: Duration = Duration::from_secs(5);

    assert!(
        JOB_COMPLETION_WINDOW >= CUPS_USB_POST_WRITE_TAIL + A_LONG_RECEIPT,
        "{JOB_COMPLETION_WINDOW:?} is shorter than the backend's own tail after the last byte \
         plus a long receipt: a printed ticket would be cancelled and reported failed"
    );
    // And still far inside the hub's 90 s lease on the job (`print_queue::DEFAULT_LEASE_SECONDS`):
    // a host that blocks past the lease is a ticket printed twice by another host.
    assert!(JOB_COMPLETION_WINDOW < Duration::from_secs(45), "{JOB_COMPLETION_WINDOW:?}");
}

#[test]
fn a_low_roll_reported_while_the_ticket_prints_does_not_cancel_it() {
    // The false positive that matters most: a thermal roll running low reports
    // `media-low-warning` for hours while printing perfectly. It is there at the pre-flight,
    // it is there while the backend runs the job, and neither may turn a ticket that LEAVES the
    // queue into a cancelled one. Only a ticket that never leaves is a failure — the reason is
    // read then, and only then.
    let cups = FakeCups::spooling(
        "Star_TSP143",
        vec![
            "printer-is-accepting-jobs=true printer-state=3 \
             printer-state-reasons=media-low-warning",
            "printer-is-accepting-jobs=true printer-state=4 \
             printer-state-reasons=media-low-warning,cups-waiting-for-job-completed",
        ],
        vec!["Star_TSP143-7 cashier 1024 Sat Sep  5 17:17:50 2026", ""],
    );

    send_raw(&cups, &a_queue(), b"ticket").expect("a low roll still prints, and this one did");

    let calls = cups.calls.borrow();
    assert!(
        !calls.iter().any(|(program, _, _)| program == CUPS_CANCEL),
        "a warning must not cancel a ticket that came out: {calls:?}"
    );
    assert!(calls.iter().any(|(program, _, _)| program == CUPS_LP), "it must submit");
}
