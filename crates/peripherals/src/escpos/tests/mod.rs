use super::*;
use oem_cp::Cp437;
use serde_json::json;

mod cash_report;
mod document;
mod invoice;
mod item_lines;
mod kitchen;
mod paper_rules;
mod prebill;
mod receipt;

/// Strips the ESC/POS commands and leaves the text a cook reads.
///
/// Filtering out «control bytes» alone does NOT work, and it is a trap: in `ESC a 0` only the
/// `ESC` is a control byte — the `a` and the `0` are printable ASCII, so the chit would read as
/// `aE!0COCINA` and any `contains` would pass by accident over garbage.
///
/// The chit's commands are all three bytes long (`ESC a n`, `ESC E n`, `ESC M n`, `GS ! n`,
/// `GS V n`): there is no QR or barcode here, which are the variable-length ones. Should one
/// ever come in, this helper flags it instead of swallowing it.
fn strip_escpos(bytes: &[u8]) -> String {
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            0x1b | 0x1d => {
                assert!(
                    i + 2 < bytes.len(),
                    "ESC/POS command truncated at {i}: the renderer emitted half a command"
                );
                assert!(
                    !matches!((bytes[i], bytes[i + 1]), (0x1d, 0x28) | (0x1d, 0x6b)),
                    "variable-length command (QR/barcode) in a kitchen chit: this helper \
                     only knows 3-byte commands and would be cutting it wrong"
                );
                i += 3;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// How each line is PRINTED, not just what it says: `(text, bold, double_height)`.
///
/// The hierarchy of the paper is decided in the ESC/POS modes, not in the text, so a test that
/// only looks at the text cannot tell a discreet header from one that steals the eye from the
/// dish — which is exactly what the chit has to guarantee.
fn lines_with_modes(bytes: &[u8]) -> Vec<(String, bool, bool)> {
    let (mut bold, mut double_h) = (false, false);
    let (mut out, mut cur) = (Vec::new(), Vec::new());
    // The mode in force when the line STARTS is the one that prints it.
    let (mut line_bold, mut line_double) = (false, false);
    let mut fresh = true;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            0x1b if bytes.get(i + 1) == Some(&0x45) => {
                bold = bytes[i + 2] == 1;
                i += 3;
            }
            0x1d if bytes.get(i + 1) == Some(&0x21) => {
                double_h = bytes[i + 2] & 0x10 != 0;
                i += 3;
            }
            0x1b | 0x1d => i += 3,
            b'\n' => {
                out.push((
                    String::from_utf8_lossy(&cur).into_owned(),
                    line_bold,
                    line_double,
                ));
                cur.clear();
                fresh = true;
                i += 1;
            }
            b => {
                if fresh {
                    line_bold = bold;
                    line_double = double_h;
                    fresh = false;
                }
                cur.push(b);
                i += 1;
            }
        }
    }
    out
}

/// Whether the line `txt` was printed in double width (`GS ! n`, bit 0x20).
fn double_width_of(bytes: &[u8], txt: &str) -> bool {
    let (mut wide, mut i) = (false, 0usize);
    while i < bytes.len() {
        match bytes[i] {
            0x1d if bytes.get(i + 1) == Some(&0x21) => {
                wide = bytes[i + 2] & 0x20 != 0;
                i += 3;
            }
            0x1b | 0x1d => i += 3,
            _ => {
                if bytes[i..].starts_with(txt.as_bytes()) {
                    return wide;
                }
                i += 1;
            }
        }
    }
    panic!("«{txt}» is not on the paper");
}

/// The two readers above are tested themselves, for the same reason as `strip_escpos`: if they
/// always returned `false`, the hierarchy assertions would pass on anything.
#[test]
fn the_mode_reader_tells_a_loud_line_from_a_quiet_one() {
    let mut b = EscposBuilder::new();
    b.set(Align::Left, true, true, true).text("PLATO\n");
    b.set(Align::Left, false, false, false).text("nota\n");
    let bytes = b.finish();
    assert_eq!(
        lines_with_modes(&bytes),
        vec![
            ("PLATO".to_string(), true, true),
            ("nota".to_string(), false, false),
        ]
    );
    assert!(double_width_of(&bytes, "PLATO"));
    assert!(!double_width_of(&bytes, "nota"));
}

/// The helper above holds up every assertion about the chit, so it is tested itself: if it left
/// residue, a `contains("Mesa 4")` would still pass on `aE!0Mesa 4` and the tests of this block
/// would be worth nothing.
#[test]
fn the_paper_helper_leaves_no_command_residue() {
    let mut b = EscposBuilder::new();
    b.set(Align::Center, true, true, true).text("COCINA\n");
    b.set(Align::Left, false, false, false).text("2x Croquetas\n");
    assert_eq!(strip_escpos(&b.finish()), "COCINA\n2x Croquetas\n");
}

/// What makes an invoice a FULL invoice (RD 1619/2012 art. 6): both parties identified and
/// the VAT broken down per rate. Two rates on purpose: a single-rate sale is the case where a
/// breakdown and a lone `tax_amount` look the same on paper.
fn full_invoice() -> serde_json::Value {
    json!({
        "business_name": "Bar Manolo",
        "vat_number": "B11111111",
        "receipt_id": "F-2026-000007",
        "customer_name": "Talleres Paco SL",
        "customer_tax_id": "B12345678",
        "customer_address": "Calle Mayor 3, Madrid",
        "items": [
            { "name": "Menu", "quantity": 1, "total": 11.0 },
            { "name": "Vino", "quantity": 1, "total": 12.1 }
        ],
        "tax_breakdown": [
            { "rate": 10, "base": 10.0, "tax": 1.0 },
            { "rate": 21, "base": 10.0, "tax": 2.1 }
        ],
        "subtotal": 20.0,
        "tax_amount": 3.1,
        "total": 23.1,
    })
}

/// How many QR symbols reached the paper. One symbol is FIVE `GS ( k` commands (set model,
/// module size, error correction, store data, print), so counting the raw prefix would answer
/// five when the honest answer is one.
fn qr_symbols(bytes: &[u8]) -> usize {
    bytes
        .windows(3)
        .filter(|w| *w == [0x1d, 0x28, 0x6b])
        .count()
        / 5
}

/// Renders any document and returns the paper without control bytes.
fn printed(doc: DocumentType, data: &serde_json::Value) -> String {
    strip_escpos(&render_document(doc, data).expect("a valid document"))
}

const PROMO_URL: &str = "https://g.page/r/bar-manolo/review";

/// The paper's text lines, whatever the document type.
fn money_paper(doc: DocumentType, data: &serde_json::Value) -> Vec<String> {
    let bytes = render_document(doc, data).expect("a valid document");
    lines_with_modes(&bytes).into_iter().map(|(text, _, _)| text).collect()
}

/// The line that starts with `label`, trimmed — the amount is what is right of it.
fn line_of<'a>(lines: &'a [String], label: &str) -> &'a str {
    lines
        .iter()
        .map(|l| l.trim())
        .find(|l| l.starts_with(label))
        .unwrap_or_else(|| panic!("no line starting with {label:?} in {lines:#?}"))
}

/// The paper as the printer receives it, one entry per line, decoded from cp437 so accented
/// text reads as written. Unlike [`strip_escpos`] it steps over the QR commands
/// (`GS ( k pL pH …`), which the ticket carries around its notes.
fn roll_lines(bytes: &[u8]) -> Vec<String> {
    use oem_cp::StringExt;
    let (mut out, mut cur) = (Vec::new(), Vec::new());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            0x1d if bytes.get(i + 1) == Some(&0x28) => {
                let len = bytes[i + 3] as usize + ((bytes[i + 4] as usize) << 8);
                i += 5 + len;
            }
            0x1b | 0x1d => i += 3,
            b'\n' => {
                out.push(String::from_cp_lossy::<Cp437>(&cur));
                cur.clear();
                i += 1;
            }
            b => {
                cur.push(b);
                i += 1;
            }
        }
    }
    out
}
