use super::*;
use super::cups::CupsOutput;
use super::fake_cups::{cups_said, FakeCups, LPOPTIONS_IDLE};

fn a_queue() -> UsbTarget {
    UsbTarget { queue: "Star_TSP143".into() }
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

// ── The pre-flight ───────────────────────────────────────────────────────────────────────
//
// `lp` exits 0 the moment the SPOOLER takes the job, and a printer that is out of paper,
// paused or unplugged still has a spooler. Measured on macOS against a real CUPS
// queue (`cupsdisable -r "out of paper"`): `lp -d … -o raw` answered `request id is …-9` with
// exit 0 and the ticket sat in the OS queue. So the state has to be ASKED for, before.

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
