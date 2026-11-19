use super::*;
use crate::escpos::builder::to_printable;

/// **Typographic punctuation reaches the paper as punctuation, not as `?`.**
///
/// The printer speaks cp437 and the encoder replaces what it cannot represent with `?`, so the
/// translated notice — «Cuenta — no es una factura» with a real em dash — would come out as
/// «Cuenta ? no es una factura». The text is written by translators in a text editor, so the
/// dashes and curly quotes are not a mistake anybody is going to stop making; the encoder is
/// the place that has to cope.
#[test]
fn typographic_punctuation_reaches_the_paper_as_punctuation() {
    let mut b = EscposBuilder::new();
    b.text("Cuenta — no es una factura: “Bar Manolo’s”…\n");
    let text = String::from_utf8_lossy(&b.finish()).to_string();
    assert!(
        !text.contains('?'),
        "nothing on the paper turned into a question mark, got: {text}"
    );
    assert!(text.contains("Cuenta - no es una factura"), "the em dash prints as a dash: {text}");
    assert!(text.contains("\"Bar Manolo's\""), "quotes print as quotes: {text}");
    assert!(text.contains("..."), "the ellipsis prints as three dots: {text}");
}

fn bytes_of(txt: &str) -> Vec<u8> {
    let mut b = EscposBuilder::new();
    b.text(txt);
    b.finish()
}

/// cp437 has no Á, Í, Ó or Ú (nor À, Â, Ã, È, Ê… or ã, õ, ý): the letter prints without its accent
/// rather than as `?`.
#[test]
fn an_accented_letter_the_printer_lacks_prints_without_its_accent() {
    for (written, printed) in [
        ("CRUSTÁCEOS", "CRUSTACEOS"),
        ("ALBÓNDIGAS", "ALBONDIGAS"),
        ("ÍNDICE", "INDICE"),
        ("ÚLTIMA", "ULTIMA"),
        ("ÀÂÃ ÈÊË ÌÎÏ ÒÔÕ ÙÛ Ý", "AAA EEE III OOO UU Y"),
        ("ã õ ý", "a o y"),
    ] {
        let expected: Vec<u8> = printed.chars().map(|c| c as u8).collect();
        assert_eq!(bytes_of(written), expected, "{written:?}");
    }
}

/// The accented letters cp437 does have keep their accent, byte for byte.
#[test]
fn the_accented_letters_cp437_has_keep_their_accent() {
    assert_eq!(
        bytes_of("áéíóúñÑÉÜçü"),
        [0xA0, 0x82, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0x90, 0x9A, 0x87, 0x81]
    );
}

/// What has no letter inside still prints as `?`: the paper shows something is missing.
#[test]
fn a_character_with_no_printable_letter_still_prints_as_a_question_mark() {
    assert_eq!(bytes_of("✓ 中"), b"? ?");
}

/// «≠» decomposes into «=» plus a stroke: printing the «=» alone would say the opposite.
#[test]
fn a_negated_sign_does_not_print_as_the_sign_it_negates() {
    assert_eq!(bytes_of("1 ≠ 2 ≤ 3"), b"1 ? 2 \xF3 3");
    assert_eq!(bytes_of("≮ ≯ ≢"), b"? ? ?");
}

/// A keyboard or a pasted text may send the accent as a separate combining character.
#[test]
fn a_letter_followed_by_a_combining_accent_prints_like_the_single_character() {
    assert_eq!(bytes_of("cafe\u{301} pin\u{303}a"), bytes_of("café piña"));
    assert_eq!(bytes_of("CRUSTA\u{301}CEOS"), b"CRUSTACEOS");
    assert_eq!(to_printable("cafe\u{301} con pin\u{303}a"), "café con piña", "one column per letter");
}

// ── The paper speaks the language the document asks for ─────────────────────────────────────
//
// A hub running in another language must not paint a translated KDS and push a Spanish chit
// out of the thermal printer. English is the source and `es` reproduces, byte for byte, the
// paper the fleet prints today: a document WITHOUT `locale` — every deployed `erplora-app` —
// must not move a single column.

/// The kitchen chit in English: the labels this printer owns are translated, the values the
/// producer sends are not touched.
#[test]
fn the_kitchen_chit_can_be_printed_in_english() {
    let text = printed(
        DocumentType::KitchenOrder,
        &json!({
            "locale": "en",
            "table": "4", "waiter": "Ana", "round_number": 2, "priority": "HIGH",
            "items": [{ "name": "Burger", "quantity": 1 }],
        }),
    );
    for wanted in ["KITCHEN", "Table: 4", "Waiter: Ana", "Round 2", "Time: ", "!! RUSH !!"] {
        assert!(text.contains(wanted), "the chit says {wanted:?}:\n{text}");
    }
    for spanish in ["COCINA", "Mesa:", "Camarero:", "Ronda", "Hora:", "URGENTE"] {
        assert!(!text.contains(spanish), "and nothing is left in Spanish ({spanish:?}):\n{text}");
    }
}

/// The customer's ticket in English — the surface the customer actually holds.
#[test]
fn the_ticket_can_be_printed_in_english() {
    let text = printed(
        DocumentType::Receipt,
        &json!({
            "locale": "en",
            "receipt_id": "T-42", "cashier": "Ana", "customer_name": "Bob",
            "vat_number": "B123", "phone": "600",
            "items": [{ "name": "Coffee", "quantity": 1, "total": 1.8 }],
            "subtotal": 1.5, "tax_amount": 0.3, "discount": 0.2,
            "total": 1.8, "payment_method": "Card", "paid": 2.0, "change": 0.2,
        }),
    );
    for wanted in [
        "VAT: B123", "Ticket: T-42", "Date: ", "Cashier: Ana", "Customer: Bob",
        "Subtotal", "VAT", "Discount", "TOTAL", "Payment: Card", "Tendered", "Change",
        "Thank you for your purchase",
    ] {
        assert!(text.contains(wanted), "the ticket says {wanted:?}:\n{text}");
    }
    for spanish in ["NIF:", "Fecha:", "Cajero:", "Cliente:", "Descuento", "Pago:", "Entregado", "Cambio", "Gracias"] {
        assert!(!text.contains(spanish), "and nothing is left in Spanish ({spanish:?}):\n{text}");
    }
}

/// The bill in English, DEFAULT NOTICE INCLUDED: the notice is what keeps this paper from
/// passing for an invoice, so a producer that forgets to send it still gets it in the
/// customer's language rather than in Spanish.
#[test]
fn the_bill_and_its_default_notice_can_be_printed_in_english() {
    let text = printed(
        DocumentType::Prebill,
        &json!({
            "locale": "en",
            "customer_name": "Table 4",
            "items": [{ "name": "Coffee", "quantity": 1, "total": 1.8 }],
            "total": 1.8,
        }),
    );
    assert!(text.contains("BILL"), "the title:\n{text}");
    assert!(text.contains("Table/Customer: Table 4"), "the table:\n{text}");
    assert!(text.contains("not an invoice"), "the notice is in English too:\n{text}");
    for spanish in ["CUENTA", "Mesa/Cliente:", "no es una factura"] {
        assert!(!text.contains(spanish), "nothing left in Spanish ({spanish:?}):\n{text}");
    }
}

/// The delivery note and the cash-up report are labels of this file too.
#[test]
fn the_delivery_note_and_the_cash_report_can_be_printed_in_english() {
    let note = printed(
        DocumentType::DeliveryNote,
        &json!({
            "locale": "en", "receipt_id": "A-1",
            "customer_name": "Bob", "delivery_address": "Main St 1",
            "items": [{ "name": "Coffee", "quantity": 1 }],
        }),
    );
    for wanted in ["DELIVERY NOTE", "No: A-1", "Date: ", "Customer: Bob", "Addr: Main St 1", "Signature"] {
        assert!(note.contains(wanted), "the delivery note says {wanted:?}:\n{note}");
    }
    let report = printed(
        DocumentType::CashSessionReport,
        &json!({ "locale": "en", "receipt_id": "S-1", "cashier": "Ana",
                 "opening_balance": 100.0, "closing_balance": 150.0 }),
    );
    // No «Difference»: closing − opening is not the over/short.
    for wanted in ["CASH REPORT", "Session: S-1", "Cashier: Ana", "Opening", "Closing"] {
        assert!(report.contains(wanted), "the cash report says {wanted:?}:\n{report}");
    }
}

/// 🔴 The compatibility guarantee: a document WITHOUT `locale` prints byte for byte the Spanish
/// paper the fleet already prints, and `locale: "es"` prints the very same bytes. Every deployed
/// `erplora-app` sends no locale, so if this moves, every kitchen in the fleet notices.
#[test]
fn a_document_without_a_locale_prints_exactly_the_spanish_paper_of_today() {
    let cases: Vec<(DocumentType, serde_json::Value, Vec<&str>)> = vec![
        (
            DocumentType::KitchenOrder,
            json!({ "table": "4", "waiter": "Ana", "round_number": 2, "priority": "HIGH",
                    "items": [{ "name": "Cafe", "quantity": 1 }] }),
            vec!["COCINA", "Mesa: 4", "Camarero: Ana", "Ronda 2", "Hora: ", "!! URGENTE !!"],
        ),
        (
            DocumentType::Receipt,
            json!({ "receipt_id": "T-42", "cashier": "Ana", "customer_name": "Bob",
                    "vat_number": "B1", "phone": "600", "discount": 0.2, "paid": 2.0,
                    "change": 0.2, "payment_method": "Tarjeta", "subtotal": 1.5,
                    "tax_amount": 0.3, "total": 1.8,
                    "items": [{ "name": "Cafe", "quantity": 1, "total": 1.8 }] }),
            vec!["NIF: B1", "Ticket: T-42", "Fecha: ", "Cajero: Ana", "Cliente: Bob",
                 "IVA", "Descuento", "TOTAL", "Pago: Tarjeta", "Entregado", "Cambio",
                 "Gracias por su compra"],
        ),
        (
            DocumentType::Prebill,
            json!({ "customer_name": "Mesa 4", "total": 1.8,
                    "items": [{ "name": "Cafe", "quantity": 1, "total": 1.8 }] }),
            // Asserted in pieces: the notice is wrapped to 32 columns, so the whole
            // sentence never lives on one line.
            vec!["CUENTA", "Mesa/Cliente: Mesa 4", "no es una factura", "tiquet fiscal"],
        ),
        (
            DocumentType::DeliveryNote,
            json!({ "receipt_id": "A-1", "customer_name": "Bob", "delivery_address": "Calle 1",
                    "items": [{ "name": "Cafe", "quantity": 1 }] }),
            vec!["ALBARAN", "N: A-1", "Cliente: Bob", "Dir: Calle 1", "Firma: _______________"],
        ),
        (
            DocumentType::CashSessionReport,
            json!({ "receipt_id": "S-1", "cashier": "Ana",
                    "opening_balance": 100.0, "closing_balance": 150.0 }),
            // A report that does not say X or Z is titled neutrally and carries no invented
            // «Diferencia».
            vec!["INFORME DE CAJA", "Sesion: S-1", "Cajero: Ana", "Apertura", "Cierre"],
        ),
    ];
    for (doc, data, spanish) in cases {
        let text = printed(doc, &data);
        for wanted in &spanish {
            assert!(text.contains(wanted), "{doc:?} still says {wanted:?}:\n{text}");
        }
        // …and asking for `es` explicitly is the same paper, byte for byte.
        let mut with_es = data.clone();
        with_es["locale"] = json!("es");
        assert_eq!(
            render_document(doc, &data).expect("valid"),
            render_document(doc, &with_es).expect("valid"),
            "{doc:?}: `locale: es` is the default paper, byte for byte",
        );
    }
}

/// A language this printer does not carry falls back to the paper of today instead of
/// printing an empty label or refusing the job: paper that does not come out loses a service.
#[test]
fn an_unknown_language_falls_back_to_the_paper_of_today() {
    let base = json!({ "items": [{ "name": "Cafe", "quantity": 1 }], "waiter": "Ana" });
    let today = render_document(DocumentType::KitchenOrder, &base).expect("valid");
    for odd in [json!("fr"), json!("zz-ZZ"), json!(42), json!(null)] {
        let mut data = base.clone();
        data["locale"] = odd.clone();
        assert_eq!(
            render_document(DocumentType::KitchenOrder, &data).expect("valid"),
            today,
            "`locale: {odd}` prints the default paper",
        );
    }
    // A region is not a language: `es-ES`/`en_GB` are the language before the separator.
    let mut english = base.clone();
    english["locale"] = json!("en-GB");
    assert!(
        strip_escpos(&render_document(DocumentType::KitchenOrder, &english).expect("valid"))
            .contains("KITCHEN"),
        "the region is dropped and the language honoured",
    );
}

/// `locale` is a field of the ENVELOPE, not a line of the document: the generic renderer
/// prints every unknown key, so without this it would print «locale: en» on the paper.
#[test]
fn the_locale_is_never_printed_as_a_line_of_the_document() {
    let text = printed(
        DocumentType::Generic,
        &json!({ "title": "Aviso", "locale": "en", "cosa": "valor" }),
    );
    assert!(text.contains("cosa: valor"), "the real fields still print:\n{text}");
    assert!(!text.contains("locale"), "the envelope field is not a line:\n{text}");
}

// ── The paper prints money in the currency's own scale ────────────────────────────────────

fn yen_ticket() -> serde_json::Value {
    json!({
        "business_name": "Sushi Tanaka",
        "decimals": 0,
        "items": [{ "name": "Nigiri", "quantity": 2, "total": 1500.0 }],
        "subtotal": 1364.0,
        "tax_amount": 136.0,
        "discount": 100.0,
        "total": 1400.0,
        "payment_method": "Efectivo",
        "paid": 2000.0,
        "change": 600.0,
    })
}

/// **A yen shop's ticket has no cents**: `sales` sends the amounts already divided by the
/// currency's scale and says which scale it is, so the paper must not add «.00» to every one of
/// them while the screen and the HTML ticket show «1500».
#[test]
fn a_yen_ticket_prints_every_amount_without_decimals() {
    let lines = money_paper(DocumentType::Receipt, &yen_ticket());
    assert!(!lines.iter().any(|l| l.contains(".00")), "{lines:#?}");
    assert!(line_of(&lines, "2x Nigiri").ends_with(" 1500"), "{lines:#?}");
    assert!(line_of(&lines, "TOTAL").ends_with(" 1400"), "{lines:#?}");
    assert!(line_of(&lines, "Subtotal").ends_with(" 1364"), "{lines:#?}");
    assert!(line_of(&lines, "Descuento").ends_with(" -100"), "{lines:#?}");
    assert!(line_of(&lines, "Entregado").ends_with(" 2000"), "{lines:#?}");
    assert!(line_of(&lines, "Cambio").ends_with(" 600"), "{lines:#?}");
}

/// The bill the waiter takes to the table, same scale.
#[test]
fn a_yen_bill_prints_every_amount_without_decimals() {
    let lines = money_paper(DocumentType::Prebill, &yen_ticket());
    assert!(!lines.iter().any(|l| l.contains(".00")), "{lines:#?}");
    assert!(line_of(&lines, "2x Nigiri").ends_with(" 1500"), "{lines:#?}");
    assert!(line_of(&lines, "TOTAL").ends_with(" 1400"), "{lines:#?}");
}

/// **A dinar has three decimals, and the third one is money**: printed as `1.23`, `1.234` KWD
/// would lose a fil on every amount — the VAT breakdown included.
#[test]
fn a_dinar_invoice_keeps_its_third_decimal_on_every_amount() {
    let mut data = full_invoice();
    data["decimals"] = json!(3);
    data["items"] = json!([{ "name": "Menu", "quantity": 1, "total": 1.234 }]);
    data["tax_breakdown"] = json!([{ "rate": 10, "base": 1.122, "tax": 0.112 }]);
    data["subtotal"] = json!(1.122);
    data["tax_amount"] = json!(0.112);
    data["total"] = json!(1.234);
    let lines = money_paper(DocumentType::Invoice, &data);
    assert!(line_of(&lines, "1x Menu").ends_with(" 1.234"), "{lines:#?}");
    assert!(line_of(&lines, "TOTAL").ends_with(" 1.234"), "{lines:#?}");
    let row = line_of(&lines, "IVA 10%");
    assert!(row.contains(" 1.122") && row.ends_with(" 0.112"), "{lines:#?}");
}

/// The cash report and the shelf label are money too: a yen till closes in yen.
#[test]
fn a_yen_cash_report_and_shelf_label_print_without_decimals() {
    let report = money_paper(
        DocumentType::CashSessionReport,
        &json!({
            "decimals": 0,
            "opening_balance": 10000.0,
            "closing_balance": 25000.0,
            "transactions": [{ "label": "Venta", "amount": 15000.0 }],
        }),
    );
    assert!(!report.iter().any(|l| l.contains(".00")), "{report:#?}");
    assert!(line_of(&report, "Venta").ends_with(" 15000"), "{report:#?}");
    let label = money_paper(
        DocumentType::BarcodeLabel,
        &json!({ "decimals": 0, "product_name": "Te verde", "price": 480.0 }),
    );
    assert!(label.iter().any(|l| l.trim() == "480"), "{label:#?}");
}

/// **A document without `decimals` prints two decimals**, as a euro ticket always has: a module
/// that does not send the field must not change a byte of it.
/// A scale that is not a whole number between 0 and 4 (the widest ISO 4217 scale) is not
/// trusted either: two decimals, never a ticket with nine.
#[test]
fn a_document_without_a_valid_scale_prints_two_decimals() {
    let mut data = yen_ticket();
    data.as_object_mut().unwrap().remove("decimals");
    let without = render_document(DocumentType::Receipt, &data).expect("valid");
    let lines = money_paper(DocumentType::Receipt, &data);
    assert!(line_of(&lines, "TOTAL").ends_with(" 1400.00"), "{lines:#?}");
    for bad in [json!(5), json!(-1), json!("0"), json!(2.5), json!(null)] {
        data["decimals"] = bad.clone();
        let bytes = render_document(DocumentType::Receipt, &data).expect("valid");
        assert_eq!(bytes, without, "decimals = {bad} must print as if absent");
    }
    data["decimals"] = json!(2);
    assert_eq!(
        render_document(DocumentType::Receipt, &data).expect("valid"),
        without,
        "an explicit 2 is today's paper"
    );
}

// ── The business's own text wraps by words on the roll ────────────────────────────────────
//
// The header, the footer and the notes of the ticket are prose the business (or the producer)
// writes, of any length. Printed as one line, the printer folds them at its column count
// without looking where — «…de 2,00 E» / «UR, ya aplicado…» on a 58 mm roll, «ya apl» /
// «icado…» on an 80 mm one at 42 columns. Wrapped by words at the paper's 32 columns, no roll
// the hub drives ever has to fold them.

/// Columns a roll prints per line: 58 mm (32) and 80 mm (42 or 48, depending on its font).
const ROLL_COLUMNS: [usize; 3] = [32, 42, 48];

/// What a printer with `columns` columns does to a line longer than that: it folds it there.
fn folded(line: &str, columns: usize) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    chars.chunks(columns).map(|c| c.iter().collect()).collect()
}

/// Every line of the roll fits the paper, and on every roll width each word of `text` reaches
/// the paper whole, each of its own lines starting a printed line of its own.
fn assert_wrapped_by_words(lines: &[String], field: &str, text: &str) {
    for l in lines {
        assert!(
            l.chars().count() <= LINE_WIDTH,
            "{field}: every line fits the paper ({LINE_WIDTH} columns): {l:?}\n{lines:#?}"
        );
    }
    for columns in ROLL_COLUMNS {
        let pieces: Vec<String> = lines.iter().flat_map(|l| folded(l, columns)).collect();
        for word in to_printable(text).split_whitespace() {
            assert!(
                pieces.iter().any(|p| p.split_whitespace().any(|w| w == word)),
                "{field}: «{word}» is cut on a {columns}-column roll:\n{pieces:#?}"
            );
        }
    }
    for own_line in to_printable(text).lines() {
        assert!(
            lines.iter().any(|l| !l.trim().is_empty() && own_line.starts_with(l.trim())),
            "{field}: «{own_line}» starts a printed line of its own:\n{lines:#?}"
        );
    }
}

/// The business's text on the ticket, long enough not to fit one line, with accents, `€`
/// (printed `EUR`, three columns for one character) and the line break Ventas puts between
/// the discount note and the business's footer.
const LONG_TEXTS: [(&str, &str); 5] = [
    ("receipt_header", "Vale de 5 € para tu próxima cita en el salón"),
    (
        "receipt_footer",
        "Descuento en el ticket de 2,00 €, ya aplicado en los importes de arriba.\n\
         Gracias por su visita a Peluquería Begoña, ¡hasta pronto!",
    ),
    ("business_address", "Calle de la Constitución 125, bajo izquierda, 29001 Málaga"),
    ("claim_note", "Pide tu factura completa escaneando este código con el móvil"),
    ("promo_note", "Escanea y déjanos una reseña en Google: nos ayudas muchísimo"),
];

fn ticket_with(field: &str, text: &str) -> serde_json::Value {
    let mut data = json!({
        "business_name": "Bar Manolo",
        "receipt_id": "T-45",
        "items": [{ "name": "Cafe", "quantity": 1, "total": 1.2 }],
        "total": 1.2,
        "qr_data": "https://prewww2.aeat.es/wlpl/TIKE-CONT/ValidarQR?nif=B1",
        "claim_qr_data": "https://bar.erplora.com/p/ABCD1234ABCD1234",
        "promo_qr": PROMO_URL,
    });
    data[field] = json!(text);
    data
}

/// **The business's text reaches the ticket cut between words**, never inside one, on a 58 mm
/// roll and on an 80 mm one.
#[test]
fn the_business_text_on_the_ticket_wraps_by_words_on_every_roll() {
    for (field, text) in LONG_TEXTS {
        let bytes = render_document(DocumentType::Receipt, &ticket_with(field, text))
            .expect("a valid ticket");
        assert_wrapped_by_words(&roll_lines(&bytes), field, text);
    }
}

/// The full invoice prints the same header, footer and address, so it wraps them the same way.
#[test]
fn the_business_text_on_the_full_invoice_wraps_by_words_too() {
    for (field, text) in LONG_TEXTS.iter().filter(|(f, _)| !f.ends_with("_note")) {
        let mut data = full_invoice();
        data[*field] = json!(text);
        let bytes = render_document(DocumentType::Invoice, &data).expect("a valid invoice");
        assert_wrapped_by_words(&roll_lines(&bytes), field, text);
    }
}

/// The bill taken to the table prints the business's address too, wrapped the same way.
#[test]
fn the_address_on_the_bill_wraps_by_words_too() {
    let (field, text) = LONG_TEXTS[2];
    let data = json!({
        "items": [{ "name": "Cafe", "quantity": 1, "total": 1.2 }],
        "total": 1.2,
        field: text,
    });
    let bytes = render_document(DocumentType::Prebill, &data).expect("a valid bill");
    assert_wrapped_by_words(&roll_lines(&bytes), field, text);
}

/// A blank line the business typed between two paragraphs of its footer stays on the paper.
#[test]
fn a_blank_line_in_the_footer_stays_on_the_paper() {
    let bytes = render_document(
        DocumentType::Receipt,
        &ticket_with("receipt_footer", "Gracias por su visita\n\nVuelva pronto"),
    )
    .expect("a valid ticket");
    let lines = roll_lines(&bytes);
    let at = lines.iter().position(|l| l == "Gracias por su visita").expect("the footer prints");
    assert_eq!(lines[at + 1..at + 3], ["", "Vuelva pronto"], "{lines:#?}");
}

/// A text that already fits prints as written: one line.
#[test]
fn a_short_footer_prints_exactly_as_today() {
    let data = ticket_with("receipt_footer", "Vuelva pronto");
    let bytes = render_document(DocumentType::Receipt, &data).expect("a valid ticket");
    let lines = roll_lines(&bytes);
    assert_eq!(
        lines.iter().filter(|l| l.contains("Vuelva")).collect::<Vec<_>>(),
        vec!["Vuelva pronto"],
        "{lines:#?}"
    );
}
