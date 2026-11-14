use super::*;

/// **The bill the waiter takes to the table is a document this printer knows.**
///
/// It is the most frequently printed paper of a restaurant service — it comes out before every
/// payment, so more often than the fiscal ticket — so `prebill` has to be in the vocabulary, or
/// the only door refuses it and no printer can produce it.
///
/// Named through `parse` and not the variant on purpose: this is the string the hub's queue
/// stores and the drain frame carries, so the test pins the **wire**, which is the contract.
#[test]
fn the_bill_taken_to_the_table_is_a_document_this_printer_knows() {
    assert!(
        DocumentType::parse("prebill").is_some(),
        "`prebill` is the bill taken to the table before charging; refusing it leaves the \
         waiter with nothing to carry"
    );
}

/// **The bill still carries no QR of either kind.** The pre-bill drops the fiscal one because
/// there is no billing record yet — and for exactly the same reason there is no invoice to
/// request: a claim on a sale that has not been charged would point at nothing.
#[test]
fn the_bill_carries_neither_qr() {
    let doc = json!({
        "items": [{ "name": "Cafe", "quantity": 1, "total": 1.2 }],
        "total": 1.2,
        "qr_data": "https://prewww2.aeat.es/wlpl/TIKE-CONT/ValidarQR?nif=B1",
        "claim_qr_data": "https://bar.erplora.com/p/ABCD1234ABCD1234",
        "claim_locator": "ABCD1234ABCD1234",
    });
    let bytes = render_document(DocumentType::parse("prebill").unwrap(), &doc).unwrap();
    assert!(
        !bytes.windows(3).any(|w| w == [0x1d, 0x28, 0x6b]),
        "a bill carries no QR at all"
    );
    assert!(
        !String::from_utf8_lossy(&bytes).contains("ABCD1234ABCD1234"),
        "nor the locator in text"
    );
}

/// **The bill is NOT a ticket, and the paper has to say so.**
///
/// A bill with a series number, a payment method or a VeriFactu QR is a piece of paper that
/// looks like an invoice without being one, which is a legal problem rather than a cosmetic
/// one: the numbering is consumed when charging, not before, and there is no billing record
/// yet. So the renderer must print the lines and the total — the waiter has to be able to
/// collect — and must **not** print any of the fiscal furniture, whatever the producer sends.
#[test]
fn the_bill_prints_its_lines_but_none_of_the_fiscal_furniture() {
    let doc = json!({
        "business_name": "Bar Manolo",
        "customer_name": "Mesa 4",
        "items": [
            { "name": "Cafe", "quantity": 2, "total": 2.4 },
            { "name": "Tostada", "quantity": 1, "total": 3.5 },
        ],
        "total": 5.9,
        "notice": "Cuenta - no es una factura. El tiquet fiscal se entrega al cobrar.",
        // Sent on purpose: even if a producer hands over fiscal fields, a bill must not print
        // them. Dropping them here is cheaper than trusting every caller to omit them.
        "receipt_id": "T-42",
        "payment_method": "Efectivo",
        "qr_data": "https://prevalidacion.aeat.es/verifactu",
    });
    let bytes = render_document(
        DocumentType::parse("prebill").expect("`prebill` is a document this printer knows"),
        &doc,
    )
    .expect("a well-formed bill renders");
    let text = String::from_utf8_lossy(&bytes);

    assert!(text.contains("Bar Manolo"), "the business name is on the paper");
    assert!(text.contains("Mesa 4"), "the table is on the paper: it is what identifies the bill");
    assert!(text.contains("2x Cafe"), "the lines are on the paper");
    assert!(text.contains("TOTAL"), "the total is on the paper — it is what gets collected");
    assert!(
        text.contains("no es una factura"),
        "the notice is PRINTED: paper that looks like an invoice without being one is a legal \
         problem, not a cosmetic one"
    );

    assert!(!text.contains("T-42"), "a bill carries no series number: numbering is consumed on payment");
    assert!(!text.contains("Efectivo"), "a bill carries no payment method: nothing has been charged yet");
    assert!(
        !bytes.windows(3).any(|w| w == [0x1d, 0x28, 0x6b]),
        "a bill carries no VeriFactu QR: there is no billing record to point at yet"
    );
    assert_eq!(
        &bytes[bytes.len() - 3..],
        &[0x1d, 0x56, 0x01],
        "and the paper is cut, or the bill comes out attached to the next one"
    );
}

/// **A bill with no notice still says it is not an invoice.** The producer sends the translated
/// text (`ui.prebillNotice`), but the guarantee cannot depend on every producer remembering:
/// what is at stake is handing a customer a paper that passes for an invoice.
#[test]
fn a_bill_without_a_notice_still_says_it_is_not_an_invoice() {
    let bytes = render_document(
        DocumentType::parse("prebill").expect("`prebill` is a document this printer knows"),
        &json!({ "items": [{ "name": "Cafe", "quantity": 1, "total": 1.2 }], "total": 1.2 }),
    )
    .expect("a bill with no notice still renders");
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("no es una factura"),
        "the notice has a default: a producer that forgets it must not be able to print a \
         paper that passes for an invoice"
    );
}

/// **A bill with no lines is REFUSED, not cut blank.**
///
/// This is the guard that stops the producer's mistake from becoming paper. The renderer reads
/// by key, so a document in the SCREEN's shape (`business.name`, `lines[]` — what `ok-receipt`
/// paints) finds none of the fields it wants, takes every default and hands the customer
/// «ERPlora», no lines and TOTAL 0.00. Paper that comes out WRONG is worse than paper that does
/// not come out: nobody checks a bill that printed.
///
/// It matters beyond a typo. The module that produces the bill is installed at its **published**
/// version, so an older producer that sends exactly that shape can be the one running. Failing
/// loudly puts the reason in `_print_queue.last_error` instead of in the customer's hand.
#[test]
fn a_bill_with_no_lines_is_refused_instead_of_cut_blank() {
    // The shape `<ok-receipt>` paints on screen: right document, wrong keys.
    let screen_shape = json!({
        "business": { "name": "Bar Manolo" },
        "lines": [{ "name": "Cafe", "qty": 2, "total": 2.4 }],
        "total": 2.4,
    });
    let err = render_document(
        DocumentType::parse("prebill").expect("`prebill` is a document this printer knows"),
        &screen_shape,
    )
    .expect_err("a bill with no lines the renderer can read is not a bill");
    assert!(
        matches!(err, crate::PeripheralError::InvalidPayload(_)),
        "the refusal says the document is wrong, not the printer: {err}"
    );
}
