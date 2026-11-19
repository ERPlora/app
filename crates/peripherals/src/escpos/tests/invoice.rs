use super::*;

// ── A reprint says «duplicado» (RD 1619/2012 art. 14.4) ─────────────────────────────────────

fn fiscal_paper(doc: DocumentType, extra: serde_json::Value) -> Vec<(String, bool, bool)> {
    let mut data = json!({
        "business_name": "Bar Manolo",
        "receipt_id": "T-42",
        "items": [{ "name": "Cafe", "quantity": 2, "total": 2.4 }],
        "total": 2.4,
    });
    // A full invoice is refused without its parties and its breakdown, so the
    // invoice gets them here: what these tests look at is the mark, not the refusal.
    if doc == DocumentType::Invoice {
        for (k, v) in full_invoice().as_object().expect("an object") {
            if data.get(k).is_none() {
                data[k] = v.clone();
            }
        }
    }
    for (k, v) in extra.as_object().expect("extra fields are an object") {
        data[k] = v.clone();
    }
    let bytes = render_document(doc, &data).expect("a well-formed fiscal document renders");
    lines_with_modes(&bytes)
}

fn says(lines: &[(String, bool, bool)], word: &str) -> bool {
    lines.iter().any(|(text, _, _)| text.contains(word))
}

/// Only one original of an invoice may exist, and every other copy has to say «duplicado»
/// (art. 14.4). A simplified invoice (the ticket) is an invoice too, so both documents carry it,
/// in bold above the ticket data — where whoever gets the paper reads it first.
#[test]
fn a_duplicate_ticket_or_invoice_says_so_above_its_data() {
    for doc in [DocumentType::Receipt, DocumentType::Invoice] {
        let lines = fiscal_paper(doc, json!({ "duplicate": true }));
        let mark = lines
            .iter()
            .position(|(text, _, _)| text.trim() == "DUPLICADO")
            .unwrap_or_else(|| panic!("{doc:?}: the duplicate carries the mark, got {lines:?}"));
        assert!(lines[mark].1, "{doc:?}: the mark is printed in bold");
        let ticket = lines
            .iter()
            .position(|(text, _, _)| text.contains("T-42"))
            .expect("the document number is on the paper");
        assert!(mark < ticket, "{doc:?}: the mark comes before the ticket data");
    }
}

/// The original — the first print, the automatic one at checkout — carries no mark, and
/// neither does a document that says `duplicate: false` or something that is not a boolean:
/// the word is a legal statement, so only an explicit `true` prints it.
#[test]
fn the_original_carries_no_duplicate_mark() {
    for extra in [json!({}), json!({ "duplicate": false }), json!({ "duplicate": "yes" })] {
        let lines = fiscal_paper(DocumentType::Receipt, extra.clone());
        assert!(!says(&lines, "DUPLICADO"), "{extra}: no mark on the original, got {lines:?}");
    }
}

/// The mark speaks the language of the paper, like every other label.
#[test]
fn the_duplicate_mark_speaks_the_language_of_the_paper() {
    let lines = fiscal_paper(DocumentType::Receipt, json!({ "duplicate": true, "locale": "en" }));
    assert!(says(&lines, "DUPLICATE"), "an English paper says DUPLICATE, got {lines:?}");
    assert!(!says(&lines, "DUPLICADO"), "and not the Spanish word");
}

/// The kitchen chit prints the same mark, so its bytes are pinned here: a copy is the original
/// plus the centred bold word, and nothing else on the paper moves.
#[test]
fn a_copy_is_the_original_plus_the_centred_bold_mark_and_nothing_else() {
    let mark: Vec<u8> = [&[0x1b, 0x61, 1, 0x1b, 0x45, 1, 0x1d, 0x21, 0][..], b"DUPLICADO\n"].concat();
    let render = |doc, extra: serde_json::Value| {
        let mut data = json!({ "business_name": "Bar Manolo", "receipt_id": "T-42", "items": [] });
        if doc == DocumentType::Invoice {
            data = full_invoice();
        }
        data["duplicate"] = extra;
        render_document(doc, &data).expect("a well-formed document renders")
    };
    for doc in [DocumentType::Receipt, DocumentType::Invoice] {
        let original = render(doc, json!(false));
        let copy = render(doc, json!(true));
        let at = copy.windows(mark.len()).position(|w| w == mark).unwrap_or_else(|| panic!("{doc:?}"));
        let without_mark = [&copy[..at], &copy[at + mark.len()..]].concat();
        assert_eq!(without_mark, original, "{doc:?}");
    }
    let chit = render(DocumentType::KitchenOrder, json!(true));
    assert!(chit.windows(mark.len()).any(|w| w == mark), "the chit carries the very same bytes");
}

// ── A full invoice on the thermal roll ─────────────────────────────────────────────────────

fn invoice_paper(data: &serde_json::Value) -> Vec<(String, bool, bool)> {
    let bytes = render_document(DocumentType::Invoice, data).expect("a full invoice renders");
    lines_with_modes(&bytes)
}

fn line_index(lines: &[(String, bool, bool)], pred: impl Fn(&str) -> bool, what: &str) -> usize {
    lines
        .iter()
        .position(|(text, _, _)| pred(text))
        .unwrap_or_else(|| panic!("{what} is not on the paper: {lines:#?}"))
}

/// The paper says what it is: «FACTURA», bold and double height, above the document data —
/// the first thing whoever gets it reads, and what tells it from a ticket at a glance.
#[test]
fn a_full_invoice_is_titled_factura_above_its_data() {
    let lines = invoice_paper(&full_invoice());
    let title = line_index(&lines, |l| l.trim() == "FACTURA", "the title");
    assert!(lines[title].1 && lines[title].2, "the title is bold and double height: {lines:#?}");
    let number = line_index(&lines, |l| l.contains("F-2026-000007"), "the invoice number");
    assert!(title < number, "the title comes before the number");
    assert!(
        lines[number].0.starts_with("Factura: "),
        "the number is the invoice's, not a ticket's: {:?}",
        lines[number].0
    );
    assert!(!says(&lines, "Ticket: "), "an invoice does not call itself a ticket");
}

/// **A corrective invoice says so in its title.** An R1–R4 prints with a negative total and,
/// titled like any other invoice, reads at a glance as one more invoice; the A4 invoice and the
/// market (Holded, Odoo, Business Central) title it «Factura rectificativa». Same place,
/// same weight as the plain title, and it fits the 80 mm line.
#[test]
fn a_corrective_invoice_is_titled_factura_rectificativa() {
    for (locale, expected, plain) in [
        ("es", "FACTURA RECTIFICATIVA", "FACTURA"),
        ("en", "CORRECTIVE INVOICE", "INVOICE"),
    ] {
        let mut data = full_invoice();
        data["rectifying"] = json!(true);
        data["locale"] = json!(locale);
        let lines = invoice_paper(&data);
        let title = line_index(&lines, |l| l.trim() == expected, "the corrective title");
        assert!(lines[title].1 && lines[title].2, "{locale}: bold and double height: {lines:#?}");
        assert!(lines[title].0.chars().count() <= LINE_WIDTH, "{locale}: it fits the line");
        let number = line_index(&lines, |l| l.contains("F-2026-000007"), "the invoice number");
        assert!(title < number, "{locale}: the title comes before the number");
        assert!(
            !lines.iter().any(|(l, _, _)| l.trim() == plain),
            "{locale}: the plain title is replaced, not printed twice: {lines:#?}"
        );
    }
}

/// Without the mark the paper does not move: the word is a statement about the document, so
/// only an explicit `true` switches it on — like `duplicate`.
#[test]
fn only_an_explicit_true_makes_an_invoice_corrective() {
    for flag in [None, Some(json!(false)), Some(json!("yes")), Some(json!(1))] {
        let mut data = full_invoice();
        if let Some(flag) = flag.clone() {
            data["rectifying"] = flag;
        }
        let lines = invoice_paper(&data);
        assert!(lines.iter().any(|(l, _, _)| l.trim() == "FACTURA"), "{flag:?}: {lines:#?}");
        assert!(!says(&lines, "RECTIFICATIVA"), "{flag:?}: {lines:#?}");
    }
}

/// The customer is IDENTIFIED: name, tax id and address, together, under the document data.
#[test]
fn a_full_invoice_identifies_the_customer_with_their_tax_id() {
    let lines = invoice_paper(&full_invoice());
    let name = line_index(&lines, |l| l == "Cliente: Talleres Paco SL", "the customer's name");
    let tax_id = line_index(&lines, |l| l == "NIF: B12345678", "the customer's tax id");
    let address = line_index(&lines, |l| l == "Dir: Calle Mayor 3, Madrid", "the address");
    assert_eq!((tax_id, address), (name + 1, name + 2), "the three lines go together");
    let issuer = line_index(&lines, |l| l.contains("B11111111"), "the issuer's tax id");
    assert!(issuer < name, "the issuer heads the paper, the customer comes after");
}

/// The address is printed when it comes and its absence is not a refusal: a customer's tax id
/// is what the law cannot do without, and the address is often not on file.
#[test]
fn a_full_invoice_without_a_customer_address_still_prints() {
    let mut data = full_invoice();
    data.as_object_mut().unwrap().remove("customer_address");
    let lines = invoice_paper(&data);
    assert!(says(&lines, "NIF: B12345678"));
    assert!(!says(&lines, "Dir: "), "no empty address line");
}

/// One row per rate with its BASE and its QUOTA, under a header naming the columns — the
/// breakdown a full invoice carries (RD 1619/2012 art. 6.1.g/h). The amounts are right-aligned
/// in fixed columns so the bases and the quotas read as two columns, not as prose.
#[test]
fn a_full_invoice_breaks_the_vat_down_per_rate_with_base_and_quota() {
    let lines = invoice_paper(&full_invoice());
    let header = line_index(&lines, |l| l.starts_with("Tipo") && l.contains("Base") && l.ends_with("Cuota"), "the header");
    let ten = line_index(&lines, |l| l.starts_with("IVA 10%"), "the 10% row");
    let twenty_one = line_index(&lines, |l| l.starts_with("IVA 21%"), "the 21% row");
    assert!(header < ten && ten < twenty_one, "header, then the rates in the order sent");
    assert_eq!(lines[ten].0, format!("{:<8}{:>12}{:>12}", "IVA 10%", "10.00", "1.00"));
    assert_eq!(lines[twenty_one].0, format!("{:<8}{:>12}{:>12}", "IVA 21%", "10.00", "2.10"));
    for row in [header, ten, twenty_one] {
        assert_eq!(lines[row].0.chars().count(), LINE_WIDTH, "a row fills the 80 mm line exactly");
    }
    let total = line_index(&lines, |l| l.starts_with("TOTAL"), "the total");
    assert!(twenty_one < total, "the breakdown comes before the total");
}

/// A row may name itself (a surcharge, `RE 5.2%`) and a decimal rate prints as the rate, not
/// as `5.20`. A label longer than the rate column gets a line of its own instead of pushing
/// the amounts out of their columns.
#[test]
fn a_breakdown_row_can_name_itself_and_keeps_its_columns() {
    let mut data = full_invoice();
    data["tax_breakdown"] = json!([
        { "rate": 5.2, "base": 10.0, "tax": 0.52 },
        { "rate": 1.4, "base": 10.0, "tax": 0.14, "label": "Recargo equivalencia 1.4%" }
    ]);
    let lines = invoice_paper(&data);
    assert!(says(&lines, &format!("{:<8}{:>12}{:>12}", "IVA 5.2%", "10.00", "0.52")), "{lines:#?}");
    let long = line_index(&lines, |l| l == "Recargo equivalencia 1.4%", "the long label");
    assert_eq!(lines[long + 1].0, format!("{:<8}{:>12}{:>12}", "", "10.00", "0.14"));
}

/// English paper, English labels: the invoice is not a Spanish-only document.
#[test]
fn a_full_invoice_speaks_the_language_of_the_paper() {
    let mut data = full_invoice();
    data["locale"] = json!("en");
    let lines = invoice_paper(&data);
    assert!(lines.iter().any(|(l, _, _)| l.trim() == "INVOICE"), "{lines:#?}");
    assert!(says(&lines, "Invoice: F-2026-000007"));
    assert!(says(&lines, "Customer: Talleres Paco SL"));
    assert!(says(&lines, "VAT: B12345678"));
    assert!(says(&lines, "VAT 21%"));
    assert!(!says(&lines, "FACTURA") && !says(&lines, "Cuota"), "no Spanish left over");
}

/// **An invoice missing what makes it an invoice is REFUSED, never cut as a mute ticket.** Cut
/// with the ticket's body, a customer who asked for their invoice would take home paper without
/// their tax id or the breakdown — the silent failure. The refusal names the missing field, so
/// the print host's error says what to fix.
#[test]
fn an_invoice_missing_what_makes_it_an_invoice_is_refused_naming_the_field() {
    type Breaks = Box<dyn Fn(&mut serde_json::Value)>;
    let broken: Vec<(&str, Breaks)> = vec![
        ("vat_number", Box::new(|d| { d.as_object_mut().unwrap().remove("vat_number"); })),
        ("receipt_id", Box::new(|d| d["receipt_id"] = json!(""))),
        ("customer_name", Box::new(|d| { d.as_object_mut().unwrap().remove("customer_name"); })),
        ("customer_tax_id", Box::new(|d| { d.as_object_mut().unwrap().remove("customer_tax_id"); })),
        ("customer_tax_id", Box::new(|d| d["customer_tax_id"] = json!("  "))),
        ("tax_breakdown", Box::new(|d| { d.as_object_mut().unwrap().remove("tax_breakdown"); })),
        ("tax_breakdown", Box::new(|d| d["tax_breakdown"] = json!([]))),
        ("tax_breakdown", Box::new(|d| d["tax_breakdown"] = json!({ "21.00": { "base": 10.0, "tax": 2.1 } }))),
        ("tax_breakdown", Box::new(|d| d["tax_breakdown"] = json!([{ "rate": 21, "base": 10.0 }]))),
        ("tax_breakdown", Box::new(|d| d["tax_breakdown"] = json!([{ "rate": 21, "tax": 2.1 }]))),
        ("tax_breakdown", Box::new(|d| d["tax_breakdown"] = json!([{ "base": 10.0, "tax": 2.1 }]))),
    ];
    for (field, breaks) in broken {
        let mut data = full_invoice();
        breaks(&mut data);
        match render_document(DocumentType::Invoice, &data) {
            Err(crate::PeripheralError::InvalidPayload(msg)) => assert!(
                msg.contains(field),
                "the refusal names `{field}`, got: {msg}"
            ),
            other => panic!("an invoice without a valid `{field}` is refused, got {other:?}"),
        }
    }
}

/// The ticket does not change: the same fields on a `receipt` print no title, no customer tax
/// id and no breakdown, and a receipt without them is still a receipt. Every till of the fleet
/// prints this paper, so the invoice's body must not leak into it.
#[test]
fn a_receipt_does_not_grow_the_invoice_body() {
    let bytes = render_document(DocumentType::Receipt, &full_invoice()).expect("a receipt renders");
    let lines = lines_with_modes(&bytes);
    assert!(says(&lines, "Ticket: F-2026-000007"));
    assert!(!says(&lines, "FACTURA"), "{lines:#?}");
    assert!(!says(&lines, "B12345678") && !says(&lines, "Cuota"), "{lines:#?}");
}
