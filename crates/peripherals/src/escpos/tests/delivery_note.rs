use super::*;

/// The item lines of a delivery note: between the rule under the header and the closing rule.
fn note_lines(data: &serde_json::Value) -> Vec<String> {
    let bytes = render_document(DocumentType::DeliveryNote, data).expect("a valid delivery note");
    roll_lines(&bytes)
        .into_iter()
        .skip_while(|l| !l.starts_with("----"))
        .skip(1)
        .take_while(|l| !l.starts_with("===="))
        .collect()
}

#[test]
fn every_item_prints_its_quantity_and_name_in_order() {
    let lines = note_lines(&json!({
        "receipt_id": "A-7", "customer_name": "Bar Manolo", "delivery_address": "Calle Mayor 3",
        "items": [
            { "name": "Cerveza", "quantity": 24 },
            // A decomposed ñ, as some keyboards send it: it must still reach the paper as ñ.
            { "name": "Pin\u{303}a", "quantity": 3 },
            // cp437 has no Ó, so the capital loses its accent instead of printing «?».
            { "name": "JAMÓN", "quantity": 1.5 },
        ],
    }));
    assert_eq!(lines, ["24x Cerveza", "3x Piña", "1.5x JAMON"]);
}
