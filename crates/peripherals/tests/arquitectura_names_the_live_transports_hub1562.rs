//! The hub's own architecture doc cannot bury a printer transport this crate still ships — hub#1562.
//!
//! `ARQUITECTURA.md` kept saying "USB se sigue descartando" (§2.7) and "Transporte: solo RED/LAN
//! (USB/Bluetooth descartados por drivers/mantenimiento)" (§14) long after hub#1083 landed the OS
//! print queue. That doc is what a person — or an agent — reads BEFORE touching printing, so a
//! stale sentence there does not merely age: it teaches the opposite of what the code does, and the
//! reader has no reason to doubt it.
//!
//! The guard is a cross-check between two independent artifacts, never a copy of the prose. The
//! transports come from [`parse_print_target`], the crate's real front door, and the doc has to
//! name every shape that door still accepts while declaring none of them dead. Delete the USB
//! transport for real and its sample id stops parsing, so the doc is free to bury it again.
//!
//! What it catches is the BLANKET verdict — "USB se sigue descartando", with nothing qualifying it.
//! Saying USB is out on Android, on Windows, or through `libusb`/WebUSB/OPOS is true today and has
//! to stay writable, so a sentence that names one of those is left alone. `escritorio`/`macOS`/
//! `Linux` are deliberately NOT on that list: that is where the queue prints, so burying USB there
//! is the regression itself. When hub#1269 lands USB on Windows, `Windows` comes off the list in
//! the same commit.

use erplora_peripherals::discovery::{parse_print_target, PrintTarget};

/// The doc, pulled in at compile time: moving or renaming it cannot leave this test green.
///
/// In ERPlora/hub this was the monorepo's `ARQUITECTURA.md` (§2.7), which did not move with the
/// app. In this repo the doc a reader opens before touching printing is this crate's README, whose
/// transports table is the same §2.7 table (README.md of the repo root, «Tests adaptados»).
const ARQUITECTURA: &str = include_str!("../README.md");

/// One `printer_id` per transport the crate is meant to accept today.
const SAMPLE_IDS: [&str; 3] = [
    "network:192.168.1.50:9100",
    "bluetooth:00:11:22:33:44:55",
    "usb:Star_TSP100_Cutter",
];

/// Words this doc uses to declare a transport dead. `descart` covers descartado/descartados/
/// descartando; the other two are how §14 used to phrase the same verdict.
const BURIAL_WORDS: [&str; 3] = ["descart", "red-only", "solo red"];

/// What narrows a burial down to something that is TRUE, and so may keep being written.
///
/// Every one of these is a place USB really does not reach: Android (Kotlin, an intent-granted
/// permission and OTG cables), Windows (hub#1269, no machine to verify it on), iOS, and the three
/// library routes that would each bring back a driver per OS. Desktop is absent on purpose.
///
/// Matched as WHOLE WORDS, never as substrings: this doc is in Spanish, and `ios` sits inside
/// escenarios, cambios, servicios, varios, negocios… A substring match handed a free pass to
/// roughly any sentence in the file, which is the opposite of a guard.
const LEGITIMATE_QUALIFIERS: [&str; 6] = ["android", "windows", "ios", "libusb", "webusb", "opos"];

/// How the doc has to name the transport behind a parsed target.
///
/// Exhaustive on purpose: a fourth `PrintTarget` variant stops this file compiling until someone
/// says how `ARQUITECTURA.md` names it. That is the point — the doc gets updated with the code, not
/// a month later.
fn documented_shape(target: &PrintTarget) -> &'static str {
    match target {
        PrintTarget::Network(_) => "network:{ip}:{port}",
        PrintTarget::Bluetooth(_) => "bluetooth:{mac}",
        PrintTarget::Usb(_) => "usb:{queue}",
    }
}

/// The doc as CLAIMS: hard wrapping undone inside a block, but never across blocks.
///
/// Undoing the wrapping is what makes a sentence split over two lines one claim. Doing it over the
/// WHOLE file is what made §2.7 exempt itself: joining every newline glued the transports table to
/// the prose around it into a single 858-char "sentence", and because the Bluetooth row says
/// "solo Android", any burial written anywhere in that section inherited a qualifier it was never
/// talking about. A qualifier only excuses the claim it sits in, so a block boundary — a blank
/// line, a table row, a list item, a heading — ends the claim.
fn claims(doc: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();

    for line in doc.lines() {
        // A blockquote is prose that happens to be quoted: unwrap it like any other paragraph.
        let line = line.trim().trim_start_matches('>').trim();

        if line.is_empty() || starts_a_new_block(line) {
            flush(&mut paragraph, &mut out);
        }

        if line.starts_with('|') {
            // Every cell is its own claim. A row that names Android in one column says nothing
            // about what the next column claims.
            out.extend(line.split('|').flat_map(split_into_sentences));
        } else if !line.is_empty() {
            paragraph.push(line);
        }
    }
    flush(&mut paragraph, &mut out);
    out
}

/// Markdown that opens a new claim rather than continuing the previous one.
fn starts_a_new_block(line: &str) -> bool {
    line.starts_with('#')
        || line.starts_with('|')
        || line.starts_with("- ")
        || line.starts_with("* ")
        || line
            .split_once(". ")
            .is_some_and(|(head, _)| !head.is_empty() && head.chars().all(|c| c.is_ascii_digit()))
}

fn flush(paragraph: &mut Vec<&str>, out: &mut Vec<String>) {
    if !paragraph.is_empty() {
        out.extend(split_into_sentences(&paragraph.join(" ")));
        paragraph.clear();
    }
}

fn split_into_sentences(block: &str) -> Vec<String> {
    let collapsed = block.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return Vec::new();
    }
    collapsed.split(". ").map(str::to_owned).collect()
}

/// Whether `haystack` contains `needle` as a whole word, with ASCII-alphanumeric boundaries.
fn contains_word(haystack: &str, needle: &str) -> bool {
    let bytes = haystack.as_bytes();
    haystack.match_indices(needle).any(|(start, _)| {
        let before_ok = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
        let end = start + needle.len();
        let after_ok = end >= bytes.len() || !bytes[end].is_ascii_alphanumeric();
        before_ok && after_ok
    })
}

/// Whether `text` talks about USB the transport, and not about `libusb` or `WebUSB` — both of which
/// §2.7 names precisely to say they stay out, and neither of which is what we ship.
fn mentions_usb(text: &str) -> bool {
    contains_word(text, "USB")
}

/// Every claim in `doc` that declares the USB transport dead without naming where.
fn buried_claims(doc: &str) -> Vec<String> {
    claims(doc)
        .into_iter()
        .filter(|claim| {
            let lowered = claim.to_lowercase();
            mentions_usb(claim)
                && BURIAL_WORDS.iter().any(|word| lowered.contains(word))
                && !LEGITIMATE_QUALIFIERS
                    .iter()
                    .any(|qualifier| contains_word(&lowered, qualifier))
        })
        .collect()
}

/// hub#1562: no sentence may declare USB dead while `parse_print_target` still takes a queue.
#[test]
fn hub1562_arquitectura_does_not_bury_the_usb_transport() {
    let usb_is_alive = parse_print_target("usb:Star_TSP100_Cutter").is_ok();
    assert!(
        usb_is_alive,
        "the crate no longer parses a `usb:{{queue}}` printer id. If the OS-queue transport was \
         removed on purpose, this test and crates/peripherals/README.md have to be retired together — \
         which is the whole point of asking the code first"
    );

    let buried = buried_claims(ARQUITECTURA);

    assert!(
        buried.is_empty(),
        "crates/peripherals/README.md declares USB discarded while `crates/peripherals/src/usb.rs` prints \
         through the OS queue (hub#1083). Offending sentence(s):\n  - {}\n\nA blanket verdict is \
         what this catches. If USB is out somewhere in particular, name it — Android, Windows, \
         `libusb`/WebUSB/OPOS — and the sentence is left alone. Desktop is not on that list.",
        buried
            .iter()
            .map(|sentence| sentence.trim())
            .collect::<Vec<_>>()
            .join("\n  - ")
    );
}

/// hub#1562: every transport the front door still accepts has to be named in the doc.
#[test]
fn hub1562_arquitectura_names_every_transport_the_crate_accepts() {
    let missing: Vec<&str> = SAMPLE_IDS
        .iter()
        .filter_map(|id| parse_print_target(id).ok())
        .map(|target| documented_shape(&target))
        .filter(|shape| !ARQUITECTURA.contains(shape))
        .collect();

    assert!(
        missing.is_empty(),
        "`parse_print_target` accepts these transports and crates/peripherals/README.md never names them: {}. \
         The doc is what gets read before touching printing, so a transport it does not mention \
         does not exist for the reader.",
        missing.join(", ")
    );
}

/// The guard has to catch a burial WHEREVER it is written — hub#1562.
///
/// The first version of this file did not. Two holes, both found by writing the very sentence the
/// issue is about back into `ARQUITECTURA.md` and watching the test stay green:
///
///   1. the whole file was flattened into one string before splitting on ". ", so the transports
///      table of §2.7 and the prose around it became a single claim; the Bluetooth row's "solo
///      Android" then excused every burial in the section — including the exact spot the stale
///      sentence used to live;
///   2. the qualifiers were matched as substrings, and this doc is in Spanish: `ios` is inside
///      escenarios, cambios, servicios, varios, negocios.
///
/// So the detector is exercised against documents written on purpose, not against the doc itself:
/// a guard checked only by the text it guards passes the day that text drifts.
#[test]
fn hub1562_a_burial_is_caught_wherever_it_is_written() {
    // (case, doc, expected number of buried claims)
    let cases: [(&str, &str, usize); 8] = [
        (
            "blanket burial in the prose that follows the transports table",
            "| `bluetooth:{mac}` | SPP | **solo Android** |\n\
             USB se sigue descartando. El resto del párrafo sigue aquí.\n",
            1,
        ),
        (
            "blanket burial next to a Spanish word that merely contains `ios`",
            "El USB se descarta en todos los escenarios previstos.\n",
            1,
        ),
        (
            "blanket burial next to a Spanish word that merely contains `ios`, second flavour",
            "USB queda descartado y no hay cambios previstos.\n",
            1,
        ),
        (
            "burial inside a table cell",
            "| Transporte | solo red, USB descartado | hoy |\n",
            1,
        ),
        (
            "burial qualified with `escritorio`, which is where the queue prints",
            "El USB queda descartado en escritorio.\n",
            1,
        ),
        (
            "a qualified burial does not excuse the blanket one in the NEXT list item",
            "- El USB en Android queda descartado, y seguirá fuera.\n\
             - USB se sigue descartando.\n",
            1,
        ),
        (
            "legitimate: the library routes really are out",
            "Las vías `libusb`/WebUSB/OPOS quedan descartadas.\n",
            0,
        ),
        (
            "legitimate: USB really is out on Android and on Windows",
            "El USB en Android queda descartado, y en Windows sigue descartado.\n",
            0,
        ),
    ];

    for (case, doc, expected) in cases {
        let buried = buried_claims(doc);
        assert_eq!(
            buried.len(),
            expected,
            "{case}: expected {expected} buried claim(s), got {}: {buried:?}",
            buried.len()
        );
    }
}
