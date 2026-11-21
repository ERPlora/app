use super::*;

// ── The moment on the paper is the Hub's, not the printer's ────────────────────────────────
//
// The Hub writes on every document the moment its paper tells, already on the business's clock
// and in the paper's format: `date` («21/11/2026 15:41») on the ticket, the invoice, the bill
// and the cash report, and `time` («17:06») on the kitchen chit. A ticket reprinted an hour or a
// day later has to say the moment its invoice and its fiscal QR say, and a kitchen copy the
// moment the round was sent, which is how the cook knows how long it has been waiting. The
// device's clock only fills in when a producer sends no moment.

const CHARGED: &str = "21/11/2026 15:41";

fn date_line(doc: DocumentType, data: &serde_json::Value) -> String {
    line_of(&money_paper(doc, data), "Fecha: ").to_string()
}

fn ticket(extra: serde_json::Value) -> serde_json::Value {
    let mut doc = json!({
        "receipt_id": "T-2026-000017",
        "items": [{ "name": "Cafe", "quantity": 1, "total": 1.2 }],
        "total": 1.2,
    });
    doc.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    doc
}

#[test]
fn a_reprinted_ticket_says_when_the_sale_was_charged() {
    let doc = ticket(json!({ "date": CHARGED, "duplicate": true }));
    assert_eq!(
        date_line(DocumentType::Receipt, &doc),
        "Fecha: 21/11/2026 15:41"
    );
}

#[test]
fn a_full_invoice_says_when_the_sale_was_charged() {
    let mut doc = full_invoice();
    doc["date"] = json!(CHARGED);
    doc["duplicate"] = json!(true);
    assert_eq!(
        date_line(DocumentType::Invoice, &doc),
        "Fecha: 21/11/2026 15:41"
    );
}

#[test]
fn the_bill_says_when_it_was_asked_for() {
    let doc = ticket(json!({ "date": "21/11/2026 21:05", "customer_name": "Mesa 4" }));
    assert_eq!(
        date_line(DocumentType::Prebill, &doc),
        "Fecha: 21/11/2026 21:05"
    );
}

#[test]
fn the_cash_closing_says_when_the_session_closed() {
    let doc = json!({
        "receipt_id": "CS-7 (Z)",
        "date": "21/11/2026 23:10",
        "opening_balance": 100.0,
        "closing_balance": 101.5,
    });
    assert_eq!(
        date_line(DocumentType::CashSessionReport, &doc),
        "Fecha: 21/11/2026 23:10"
    );
}

#[test]
fn a_reprinted_kitchen_chit_says_when_its_round_was_sent() {
    let doc = json!({
        "receipt_id": "K-31",
        "round_number": 2,
        "label": "Mesa 4",
        "time": "17:06",
        "duplicate": true,
        "items": [{ "name": "Croquetas", "quantity": 2 }],
    });
    let lines = money_paper(DocumentType::KitchenOrder, &doc);
    assert_eq!(line_of(&lines, "Hora: "), "Hora: 17:06");
}

/// `dd/mm/yyyy hh:mm` or `hh:mm`, whatever the minute: what the device's clock prints.
fn is_clock(text: &str, shape: &str) -> bool {
    text.len() == shape.len()
        && text.chars().zip(shape.chars()).all(|(c, s)| match s {
            '9' => c.is_ascii_digit(),
            other => c == other,
        })
}

#[test]
fn without_a_moment_from_the_hub_the_paper_keeps_the_device_clock() {
    for missing in [json!({}), json!({ "date": "", "time": "" })] {
        for doc in [
            DocumentType::Receipt,
            DocumentType::Prebill,
            DocumentType::CashSessionReport,
        ] {
            let line = date_line(doc, &ticket(missing.clone()));
            let moment = line.strip_prefix("Fecha: ").unwrap();
            assert!(
                is_clock(moment, "99/99/9999 99:99"),
                "{doc:?} printed «{line}»"
            );
        }
        let lines = money_paper(DocumentType::KitchenOrder, &ticket(missing.clone()));
        let moment = line_of(&lines, "Hora: ")
            .strip_prefix("Hora: ")
            .unwrap()
            .to_string();
        assert!(is_clock(&moment, "99:99"), "the chit printed «{moment}»");
    }
}
