use super::*;

/// **The wire vocabulary, pinned from this side.** The hub's queue keeps the same eight names
/// (`erplora_runtime::print_queue::DOCUMENT_TYPES`) and refuses anything else at the door; the
/// two lists are duplicated on purpose (the hub server has no business linking the hardware
/// crate), so each side pins its own or they drift — and a name the queue accepts becomes a
/// job the printer refuses, one layer too late to tell the producer.
#[test]
fn the_wire_vocabulary_is_exactly_these_eight() {
    let vocabulary = [
        ("receipt", DocumentType::Receipt),
        ("kitchen_order", DocumentType::KitchenOrder),
        ("invoice", DocumentType::Invoice),
        ("delivery_note", DocumentType::DeliveryNote),
        ("barcode_label", DocumentType::BarcodeLabel),
        ("cash_session_report", DocumentType::CashSessionReport),
        ("prebill", DocumentType::Prebill),
        ("generic", DocumentType::Generic),
    ];
    for (wire, expected) in vocabulary {
        assert_eq!(DocumentType::parse(wire), Some(expected), "`{wire}`");
        // The serde spelling is the same one: the queue stores this string and the frame
        // carries it, so a rename here would break the wire without touching this list.
        assert_eq!(
            serde_json::to_value(expected).unwrap(),
            json!(wire),
            "`{wire}` must serialise as itself"
        );
    }
}

/// **A document type nobody knows is refused, not printed as `Generic`.** Printed as `Generic`,
/// a typo reaches the paper as a nameless key/value dump, and the kitchen only notices when the
/// plate does not arrive. `parse` is the ONLY way in.
#[test]
fn an_unknown_document_type_is_refused_instead_of_degraded() {
    for unknown in ["Kitchen", "kitchn", "", "html", "KITCHEN_ORDER"] {
        assert_eq!(
            DocumentType::parse(unknown),
            None,
            "`{unknown}` is not a document this printer renders"
        );
    }
}

/// **`generic` is a document you ASK for, never one you fall back into.** The distinction is
/// the whole point of having no lenient door: the type exists, and a producer may legitimately
/// name it, but nothing maps an unknown string onto it.
#[test]
fn generic_is_only_reachable_by_naming_it() {
    assert_eq!(DocumentType::parse("generic"), Some(DocumentType::Generic));
    for typo in ["Generic", "generi", "gener1c", "unknown"] {
        assert_eq!(
            DocumentType::parse(typo),
            None,
            "`{typo}` must not slide into `Generic`"
        );
    }
}

/// **A `data` that is not an object is refused before it becomes blank paper.** Every renderer
/// reads by key, so an array or a string produces a ticket with nothing on it — and a cut.
#[test]
fn a_document_that_is_not_an_object_is_refused() {
    for bad in [json!(null), json!("<p>ticket</p>"), json!([1, 2]), json!(7)] {
        let err = render_document(DocumentType::Receipt, &bad)
            .expect_err("{bad} is not a document");
        assert!(
            matches!(err, crate::PeripheralError::InvalidPayload(_)),
            "the refusal says the payload is wrong, not that the printer is: {err}"
        );
    }
}

/// Sanity of the builder every renderer is written on: a QR plus a cut must produce Epson's QR
/// stamp (`GS ( k`) and the cut (`GS V 1`). Without it, a renderer can look right and still
/// hand the printer bytes it does not understand.
#[test]
fn builder_qr_and_cut_smoke() {
    let mut b = EscposBuilder::new();
    b.set(Align::Center, false, false, false).qr("https://erplora.com/v/abc").cut();
    let out = b.finish();
    assert!(out.windows(3).any(|w| w == [0x1d, 0x28, 0x6b]), "must contain GS ( k (QR)");
    assert!(out.windows(3).any(|w| w == [0x1d, 0x56, 0x01]), "must contain GS V 1 (cut)");
}

/// The guard above rejects **shape**, never content: a real ticket still renders, and what comes
/// out carries what was asked for. Without this, refusing everything would pass the test above.
#[test]
fn a_real_receipt_still_renders_its_lines_and_total() {
    let bytes = render_document(
        DocumentType::Receipt,
        &json!({
            "business_name": "Bar Manolo",
            "receipt_id": "T-42",
            "items": [{ "name": "Cafe", "quantity": 2, "total": 2.4 }],
            "total": 2.4,
        }),
    )
    .expect("a well-formed receipt renders");
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("Bar Manolo"), "the business name is on the paper");
    assert!(text.contains("T-42"), "the ticket number is on the paper");
    assert!(text.contains("2x Cafe"), "the line is on the paper");
    assert!(text.contains("TOTAL"), "the total is on the paper");
    assert_eq!(
        &bytes[bytes.len() - 3..],
        &[0x1d, 0x56, 0x01],
        "and the paper is cut, or the next ticket comes out attached to this one"
    );
}

// ── The paper is signed by a product that EXISTS ────────────────────────────────────────────

/// **The test page signs itself with a product that exists.** It is the one sheet a shop owner
/// prints on purpose, to check the printer answers, so a dead product name would go straight
/// into their hands on paper: nothing corrects a sheet once it is cut. It signs with the same
/// name the receipt next to it falls back to (`render_receipt`) — the product they installed.
#[test]
fn the_test_page_is_signed_with_a_product_that_still_exists() {
    // An EMPTY document: the sheet an `erplora-app` prints when whatever asked for it is older
    // than this binary and sends no envelope. That is the case this guard is for —
    // with a `business_name` the header is the shop's, which the next test pins.
    let paper = strip_escpos(&render_test_page("network:10.0.2.2:9100", &json!({})));
    assert!(
        paper.lines().any(|l| l.trim() == "ERPlora"),
        "the sheet is signed by the product that printed it:\n{paper}"
    );
    // Everything the sheet is FOR stays on it: a header fix that quietly dropped the outcome
    // or the printer's address would satisfy the line above and lose the diagnosis, which is
    // the only reason anybody prints this page. The wording is the catalogue's, so the
    // assertion follows the fallback language.
    assert!(
        paper.contains("Prueba de impresion correcta"),
        "the outcome is still printed:\n{paper}"
    );
    assert!(
        paper.contains("network:10.0.2.2:9100"),
        "the sheet still names the printer it came out of:\n{paper}"
    );
}

// ── The test page speaks the hub's language and carries its name ───────────────────────────

/// **The test page is translated like every other sheet.**
///
/// Every document this crate renders takes its language off the envelope
/// ([`Locale::from_document`]). Rendered from a printer id alone, with NO document behind it,
/// the test page would give a Spanish salon that pressed «Probar» a «Test Print OK» while the
/// ticket beside it came out in Spanish. English is the SOURCE language, not the paper's — a sheet
/// hardcoded in either language is what the rule forbids.
#[test]
fn the_test_page_is_printed_in_the_language_the_document_carries() {
    let english = strip_escpos(&render_test_page(
        "network:10.0.2.2:9100",
        &json!({ "locale": "en" }),
    ));
    assert!(
        english.contains("Test Print OK"),
        "an English hub gets the English outcome:\n{english}"
    );
    assert!(
        english.contains("Printer: network:10.0.2.2:9100"),
        "and the English label for the printer it came out of:\n{english}"
    );

    let spanish = strip_escpos(&render_test_page(
        "network:10.0.2.2:9100",
        &json!({ "locale": "es" }),
    ));
    assert!(
        spanish.contains("Prueba de impresion correcta"),
        "a Spanish hub gets the Spanish outcome:\n{spanish}"
    );
    assert!(
        spanish.contains("Impresora: network:10.0.2.2:9100"),
        "and the Spanish label for the printer it came out of:\n{spanish}"
    );

    // The point of the test: the two sheets are NOT the same paper. A translation that
    // resolved both keys to the same literal would satisfy every assertion above one by one.
    assert_ne!(english, spanish, "the sheet actually changes with the language");
}

/// The region is dropped like everywhere else — `es-ES` is Spanish, `en_GB` is English.
///
/// Worth its own case because the shell hands out full BCP-47 tags (`es-ES`), not bare
/// languages: a test page that only understood `es` would be English for every real hub.
#[test]
fn the_test_page_reads_a_language_tag_with_its_region() {
    let spanish = strip_escpos(&render_test_page("usb:001:002", &json!({ "locale": "es-ES" })));
    assert!(
        spanish.contains("Prueba de impresion correcta"),
        "`es-ES` is Spanish:\n{spanish}"
    );
    let english = strip_escpos(&render_test_page("usb:001:002", &json!({ "locale": "en_GB" })));
    assert!(english.contains("Test Print OK"), "`en_GB` is English:\n{english}");
}

/// **Headed by the business, like the ticket next to it.**
///
/// A sheet signed by the SOFTWARE does not tell the shop owner holding it which of their
/// printers answered, and the name they recognise is their own — the receipt heads itself that
/// way (`render_receipt`), from the very same `business_name` field.
#[test]
fn the_test_page_is_headed_by_the_business_that_printed_it() {
    let paper = strip_escpos(&render_test_page(
        "network:10.0.2.2:9100",
        &json!({ "business_name": "SALON AURORA SL", "locale": "es" }),
    ));
    assert!(
        paper.lines().any(|l| l.trim() == "SALON AURORA SL"),
        "the shop's own name heads the sheet:\n{paper}"
    );
    assert!(
        !paper.lines().any(|l| l.trim() == "ERPlora"),
        "and it replaces the product name rather than being added under it:\n{paper}"
    );
    // The diagnosis survives the new header, same as above.
    assert!(
        paper.contains("network:10.0.2.2:9100"),
        "the sheet still names the printer it came out of:\n{paper}"
    );
}

/// **No paper this crate renders names «Bridge»**, a product that no longer exists — not just
/// the test page. The guard that keeps the dead product from coming back through another
/// renderer.
///
/// It reads the RAW bytes rather than the stripped paper on purpose: a receipt carries a QR
/// (`GS ( k`), a command of variable length that [`strip_escpos`] refuses by design, so a
/// guard written on the stripped text could not cover every document — and the document it
/// could not cover is the one the customer takes home.
#[test]
fn no_paper_this_crate_renders_names_the_retired_bridge() {
    // A single document that every renderer accepts: `Prebill` is the strict one (it refuses
    // a bill with no `items`), the rest read what they know and ignore the rest.
    let data = json!({
        "business_name": "SALON AURORA SL",
        "tax_id": "12345678Z",
        // What an invoice cannot be printed without.
        "vat_number": "B11111111",
        "receipt_id": "F-1",
        "customer_name": "Cliente SL",
        "customer_tax_id": "B12345678",
        "tax_breakdown": [{ "rate": 21, "base": 9.92, "tax": 2.08 }],
        "ticket_number": "TICKET-2026-000001",
        "title": "Aviso",
        "items": [{ "name": "Corte", "quantity": 1, "total": 12.0 }],
        "total": 12.0,
    });
    let mut papers: Vec<(String, Vec<u8>)> = Vec::new();
    for doc in [
        DocumentType::Receipt,
        DocumentType::KitchenOrder,
        DocumentType::Invoice,
        DocumentType::DeliveryNote,
        DocumentType::BarcodeLabel,
        DocumentType::CashSessionReport,
        DocumentType::Prebill,
        DocumentType::Generic,
    ] {
        papers.push((
            format!("{doc:?}"),
            render_document(doc, &data).expect("a valid document"),
        ));
    }
    papers.push((
        "test page".to_string(),
        render_test_page("network:10.0.2.2:9100", &json!({})),
    ));
    for (what, bytes) in papers {
        assert!(
            !bytes.windows(b"Bridge".len()).any(|w| w == b"Bridge"),
            "{what}: the paper names the Bridge, a product that no longer exists"
        );
    }
}
