use super::*;

/// **Two QRs, not one.** The VeriFactu one points at the AEAT's `ValidarQR` and is
/// for CHECKING; the second points at this hub and is for ASKING — «pide tu factura». They are
/// two fields on purpose: printing one over the other would either strip the fiscal proof from
/// the ticket or leave the customer with nothing to scan.
#[test]
fn a_ticket_can_carry_both_the_fiscal_qr_and_the_invoice_request_qr() {
    let doc = json!({
        "business_name": "Bar Manolo",
        "receipt_id": "T-42",
        "items": [{ "name": "Cafe", "quantity": 1, "total": 1.2 }],
        "total": 1.2,
        "qr_data": "https://prewww2.aeat.es/wlpl/TIKE-CONT/ValidarQR?nif=B1&numserie=T-42",
        "claim_note": "Pide tu factura",
        "claim_qr_data": "https://bar.erplora.com/p/ABCD1234ABCD1234",
        "claim_locator": "ABCD1234ABCD1234",
    });
    let bytes = render_document(DocumentType::parse("receipt").unwrap(), &doc)
        .expect("a well-formed ticket renders");
    let text = String::from_utf8_lossy(&bytes);

    assert_eq!(qr_symbols(&bytes), 2, "both QRs must reach the paper");
    assert!(
        text.contains("ValidarQR"),
        "the fiscal QR still points at the AEAT"
    );
    assert!(
        text.contains("/p/ABCD1234ABCD1234"),
        "and the second one at this hub"
    );
    assert!(
        text.contains("Pide tu factura"),
        "with a caption, or nobody knows what the second code is for"
    );
    assert!(
        text.contains("ABCD1234ABCD1234\n"),
        "and the locator in PLAIN TEXT below it: the camera is not always an option"
    );
}

/// **«VERI*FACTU» under the fiscal QR.** RD 1619/2012 art. 6.5.b (7.5 for the
/// simplified invoice a ticket is) wants the legend beside the QR of every invoice from a
/// system that remits all its records, and Orden HAC/1177/2024 art. 20.1.b wants it as
/// visible as the rest of the data. The producer sends it as `qr_legend`; the renderer prints
/// it right under the fiscal QR — before the second QR, so it cannot be read as its caption.
#[test]
fn the_verifactu_legend_is_printed_right_under_the_fiscal_qr() {
    let doc = json!({
        "receipt_id": "T-44",
        "items": [{ "name": "Cafe", "quantity": 1, "total": 1.2 }],
        "total": 1.2,
        "qr_data": "https://prewww2.aeat.es/wlpl/TIKE-CONT/ValidarQR?nif=B1",
        "qr_legend": "VERI*FACTU",
        "claim_note": "Pide tu factura",
        "claim_qr_data": "https://bar.erplora.com/p/ABCD1234ABCD1234",
    });
    let bytes = render_document(DocumentType::parse("receipt").unwrap(), &doc).unwrap();
    let text = String::from_utf8_lossy(&bytes);

    let fiscal = text.find("ValidarQR").expect("the fiscal QR is printed");
    let legend = text
        .find("VERI*FACTU\n")
        .expect("the legend is printed, on its own line");
    let claim = text.find("Pide tu factura").expect("the claim block is printed");
    assert!(fiscal < legend, "the legend goes under the fiscal QR");
    assert!(legend < claim, "and before the second QR, which is not what it names");

    // «bien visible» (Orden HAC/1177/2024 art. 20.1.b): the last emphasis command before the
    // legend is ESC E 1 (bold on), so it is not printed as one more 8-px caption.
    let legend_at = bytes
        .windows(b"VERI*FACTU".len())
        .position(|w| w == b"VERI*FACTU")
        .expect("legend bytes");
    let last_bold = bytes[..legend_at]
        .windows(3)
        .rposition(|w| w[0] == 0x1b && w[1] == 0x45)
        .expect("an ESC E command precedes the legend");
    assert_eq!(bytes[last_bold + 2], 1, "the legend is printed in bold");
}

/// No fiscal QR → no legend, whatever the producer sends: the legend names the QR, and alone
/// it would claim a verification the paper does not offer. The bill never carries it.
#[test]
fn the_verifactu_legend_needs_the_fiscal_qr() {
    let no_qr = json!({
        "receipt_id": "T-45",
        "items": [{ "name": "Cafe", "quantity": 1, "total": 1.2 }],
        "total": 1.2,
        "qr_legend": "VERI*FACTU",
    });
    let bytes = render_document(DocumentType::parse("receipt").unwrap(), &no_qr).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("VERI*FACTU"));

    let bill = json!({
        "items": [{ "name": "Cafe", "quantity": 1, "total": 1.2 }],
        "total": 1.2,
        "qr_data": "https://prewww2.aeat.es/wlpl/TIKE-CONT/ValidarQR?nif=B1",
        "qr_legend": "VERI*FACTU",
    });
    let bytes = render_document(DocumentType::parse("prebill").unwrap(), &bill).unwrap();
    assert!(
        !String::from_utf8_lossy(&bytes).contains("VERI*FACTU"),
        "a bill is not an invoice and cannot say it is verifiable"
    );
}

/// **«QR tributario:» above the fiscal QR, and the QR at the very top.** The AEAT's QR
/// specification (v0.5.0 section 3, which Orden HAC/1177/2024 art. 21.1 points at) wants the QR
/// «always preceded» by the text «QR tributario:», above it, and the QR «at the beginning of
/// the invoice, before the content generated by the invoicing system». The producer sends the
/// heading as `qr_heading` (a fixed legal text, not translated); the second QR stays at the foot.
#[test]
fn the_fiscal_qr_opens_the_ticket_under_its_qr_tributario_heading() {
    let doc = json!({
        "business_name": "Bar Manolo",
        "receipt_id": "T-46",
        "items": [{ "name": "Cafe", "quantity": 1, "total": 1.2 }],
        "total": 1.2,
        "qr_data": "https://prewww2.aeat.es/wlpl/TIKE-CONT/ValidarQR?nif=B1",
        "qr_heading": "QR tributario:",
        "qr_legend": "VERI*FACTU",
        "claim_note": "Pide tu factura",
        "claim_qr_data": "https://bar.erplora.com/p/ABCD1234ABCD1234",
    });
    let bytes = render_document(DocumentType::parse("receipt").unwrap(), &doc).unwrap();
    let text = String::from_utf8_lossy(&bytes);

    let heading = text
        .find("QR tributario:\n")
        .expect("the heading is printed, on its own line");
    let fiscal = text.find("ValidarQR").expect("the fiscal QR is printed");
    let legend = text.find("VERI*FACTU\n").expect("the legend is printed");
    let business = text.find("Bar Manolo").expect("the business name is printed");
    let items = text.find("Cafe").expect("the lines are printed");
    let claim = text.find("Pide tu factura").expect("the claim block is printed");
    assert!(heading < fiscal, "the heading goes ABOVE the fiscal QR");
    assert!(fiscal < legend, "the legend stays right under it");
    assert!(
        legend < business,
        "the fiscal block opens the ticket, before anything the system writes"
    );
    assert!(business < items && items < claim, "the second QR stays at the foot");

    // Same size as the rest of the data or bigger, never a smaller caption: printed in bold,
    // centred over the code.
    let heading_at = bytes
        .windows(b"QR tributario:".len())
        .position(|w| w == b"QR tributario:")
        .expect("heading bytes");
    let last_bold = bytes[..heading_at]
        .windows(3)
        .rposition(|w| w[0] == 0x1b && w[1] == 0x45)
        .expect("an ESC E command precedes the heading");
    assert_eq!(bytes[last_bold + 2], 1, "the heading is printed in bold");
    let last_align = bytes[..heading_at]
        .windows(3)
        .rposition(|w| w[0] == 0x1b && w[1] == 0x61)
        .expect("an ESC a command precedes the heading");
    assert_eq!(bytes[last_align + 2], 1, "the heading is centred over the QR");
}

/// No fiscal QR → no heading: alone it would announce a code the paper does not carry. And a
/// producer that does not send the key yet (an older `sales`) still gets its QR at the top.
#[test]
fn the_qr_tributario_heading_needs_the_fiscal_qr() {
    let no_qr = json!({
        "receipt_id": "T-47",
        "items": [{ "name": "Cafe", "quantity": 1, "total": 1.2 }],
        "total": 1.2,
        "qr_heading": "QR tributario:",
    });
    let bytes = render_document(DocumentType::parse("receipt").unwrap(), &no_qr).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("QR tributario:"));

    let bill = json!({
        "items": [{ "name": "Cafe", "quantity": 1, "total": 1.2 }],
        "total": 1.2,
        "qr_data": "https://prewww2.aeat.es/wlpl/TIKE-CONT/ValidarQR?nif=B1",
        "qr_heading": "QR tributario:",
    });
    let bytes = render_document(DocumentType::parse("prebill").unwrap(), &bill).unwrap();
    assert!(
        !String::from_utf8_lossy(&bytes).contains("QR tributario:"),
        "a bill carries no fiscal QR, so no heading either"
    );

    let old_producer = json!({
        "business_name": "Bar Manolo",
        "receipt_id": "T-48",
        "items": [{ "name": "Cafe", "quantity": 1, "total": 1.2 }],
        "total": 1.2,
        "qr_data": "https://prewww2.aeat.es/wlpl/TIKE-CONT/ValidarQR?nif=B1",
    });
    let bytes = render_document(DocumentType::parse("receipt").unwrap(), &old_producer).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.find("ValidarQR").unwrap() < text.find("Bar Manolo").unwrap(),
        "the fiscal QR opens the ticket with or without the heading key"
    );
}

/// A ticket with no claim prints only the fiscal QR. The second QR is opt-in, so a hub that does
/// not use it gets no blank line or stray caption.
#[test]
fn a_ticket_without_a_claim_is_unchanged() {
    let doc = json!({
        "receipt_id": "T-43",
        "items": [{ "name": "Cafe", "quantity": 1, "total": 1.2 }],
        "total": 1.2,
        "qr_data": "https://prewww2.aeat.es/wlpl/TIKE-CONT/ValidarQR?nif=B1",
    });
    let bytes = render_document(DocumentType::parse("receipt").unwrap(), &doc).unwrap();
    assert_eq!(qr_symbols(&bytes), 1, "only the fiscal QR");
}

fn ticket_with_promo() -> serde_json::Value {
    json!({
        "business_name": "Bar Manolo",
        "receipt_id": "T-44",
        "items": [{ "name": "Cafe", "quantity": 1, "total": 1.2 }],
        "total": 1.2,
        "qr_data": "https://prewww2.aeat.es/wlpl/TIKE-CONT/ValidarQR?nif=B1",
        "claim_note": "Pide tu factura",
        "claim_qr_data": "https://bar.erplora.com/p/ABCD1234ABCD1234",
        "claim_locator": "ABCD1234ABCD1234",
        "receipt_footer": "Vuelva pronto",
        "promo_qr": PROMO_URL,
        "promo_note": "Escanea y dejanos una resena",
    })
}

fn position(bytes: &[u8], needle: &str) -> usize {
    bytes
        .windows(needle.len())
        .position(|w| w == needle.as_bytes())
        .unwrap_or_else(|| panic!("`{needle}` is not on the paper"))
}

/// The module size (`GS ( k … 49 67 n`, fn 167) of every QR on the paper, in printing order.
fn qr_module_sizes(bytes: &[u8]) -> Vec<u8> {
    bytes
        .windows(8)
        .filter(|w| w[..7] == [0x1d, 0x28, 0x6b, 3, 0, 49, 67])
        .map(|w| w[7])
        .collect()
}

/// **The business's promotional QR reaches the thermal paper.** The screen (`<ok-receipt>`)
/// and the browser paper carry it; a roll that printed nothing would leave the business
/// believing its ticket did.
#[test]
fn a_ticket_prints_the_promotional_qr_with_its_note() {
    let bytes =
        render_document(DocumentType::parse("receipt").unwrap(), &ticket_with_promo()).unwrap();
    assert_eq!(qr_symbols(&bytes), 3, "fiscal + «pide tu factura» + promotional");
    position(&bytes, PROMO_URL);
    assert!(
        position(&bytes, "Escanea y dejanos una resena") < position(&bytes, PROMO_URL),
        "the note goes ABOVE its QR, or nobody knows what the code is for"
    );
}

/// Where it goes: it CLOSES the paper, after the fiscal QR, after «pide tu factura» and after
/// the footer and the thanks — the same order as the screen and the browser paper.
#[test]
fn the_promotional_qr_closes_the_ticket() {
    let bytes =
        render_document(DocumentType::parse("receipt").unwrap(), &ticket_with_promo()).unwrap();
    let note = position(&bytes, "Escanea y dejanos una resena");
    assert!(note > position(&bytes, "ValidarQR"), "after the fiscal QR");
    assert!(note > position(&bytes, "/p/ABCD1234ABCD1234"), "after «pide tu factura»");
    assert!(note > position(&bytes, "Vuelva pronto"), "after the footer");
    assert!(note > position(&bytes, "Gracias por su compra"), "after the thanks");
    let cut = bytes
        .windows(2)
        .rposition(|w| w == [0x1d, 0x56])
        .expect("the ticket is cut");
    assert!(position(&bytes, PROMO_URL) < cut, "and before the cut");
}

/// Smaller than the fiscal QR, like `<ok-receipt>` draws it: the fiscal proof is the code
/// the paper is for, the promotion is an extra.
#[test]
fn the_promotional_qr_is_smaller_than_the_fiscal_one() {
    let bytes =
        render_document(DocumentType::parse("receipt").unwrap(), &ticket_with_promo()).unwrap();
    let sizes = qr_module_sizes(&bytes);
    assert_eq!(sizes.len(), 3, "{sizes:?}");
    assert!(sizes[2] < sizes[0], "promotional {sizes:?} not smaller than the fiscal");
    assert!(sizes[2] > 0, "a zero module size prints nothing");
}

/// No URL, no promotion: a note alone names nothing, so the ticket prints byte for byte as if
/// the note were not there.
#[test]
fn a_promotional_note_without_its_qr_prints_nothing() {
    let mut with_note = ticket_with_promo();
    with_note.as_object_mut().unwrap().remove("promo_qr");
    let mut without = with_note.clone();
    without.as_object_mut().unwrap().remove("promo_note");
    let receipt = DocumentType::parse("receipt").unwrap();
    assert_eq!(
        render_document(receipt, &with_note).unwrap(),
        render_document(receipt, &without).unwrap()
    );
}

/// The full invoice is formal: no promotion on it, like the A4 invoice. A producer that
/// builds the invoice from its ticket (sales `saleToInvoicePrintDocument`) must not smuggle
/// the review link onto an invoice.
#[test]
fn a_full_invoice_carries_no_promotional_qr() {
    let mut doc = full_invoice();
    doc["promo_qr"] = json!(PROMO_URL);
    doc["promo_note"] = json!("Escanea y dejanos una resena");
    let bytes = render_document(DocumentType::parse("invoice").unwrap(), &doc).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(!text.contains(PROMO_URL), "no promotional QR on an invoice");
    assert!(!text.contains("Escanea"), "nor its note");
}

/// The module size of every QR on the paper is a contract, not a relation: the fiscal QR and
/// «pide tu factura» keep their 4 dots (0.5 mm at 203 dpi), and the promotional one prints at 3.
/// «Smaller than the fiscal» alone would let the fiscal one drift to 5 or the promotional one to
/// 2 (0.25 mm, which a phone no longer reads) without a test going red.
#[test]
fn every_qr_prints_at_its_contracted_module_size() {
    let receipt = DocumentType::parse("receipt").unwrap();
    let with_promo = render_document(receipt, &ticket_with_promo()).unwrap();
    assert_eq!(qr_module_sizes(&with_promo), vec![4, 4, 3], "fiscal, claim, promotional");
    let mut without = ticket_with_promo();
    without.as_object_mut().unwrap().remove("promo_qr");
    let bytes = render_document(receipt, &without).unwrap();
    assert_eq!(qr_module_sizes(&bytes), vec![4, 4], "byte for byte what it printed before");
}

#[test]
fn a_ticket_prints_accented_capitals_without_question_marks_and_keeps_the_rest() {
    let bytes = render_document(DocumentType::Receipt, &json!({
        "business_name": "Café Único",
        "items": [{ "name": "ALBÓNDIGAS", "quantity": 1, "total": 6.5 }],
        "total": 6.5,
    }))
    .unwrap();
    let text = roll_lines(&bytes).join("\n");
    assert!(text.contains("Café Unico"), "{text}");
    assert!(text.contains("ALBONDIGAS"), "{text}");
    assert!(!text.contains('?'), "{text}");
}
