use super::builder::{Align, EscposBuilder};
use super::fields::{fmt_qty, is_truthy, json_to_display, now_dmy_hm, str_field};
use super::locale::{Label, Locale};

pub(super) fn render_delivery_note(b: &mut EscposBuilder, data: &serde_json::Value) {
    let t = Locale::from_document(data);
    b.set(Align::Center, true, false, false);
    b.text(&format!("{}\n", t.label(Label::DeliveryNoteTitle)));
    b.text("================================\n");

    b.set(Align::Left, false, false, false);
    b.text(&format!("{}{}\n", t.label(Label::Number), str_field(data, "receipt_id", "")));
    b.text(&format!("{}{}\n", t.label(Label::Date), now_dmy_hm()));

    if is_truthy(data, "customer_name") {
        b.text(&format!("{}{}\n", t.label(Label::Customer), str_field(data, "customer_name", "")));
    }
    if is_truthy(data, "delivery_address") {
        b.text(&format!("{}{}\n", t.label(Label::Address), str_field(data, "delivery_address", "")));
    }

    b.text("--------------------------------\n");

    if let Some(items) = data.get("items").and_then(|v| v.as_array()) {
        for item in items {
            let qty = item.get("quantity").and_then(|v| v.as_f64()).unwrap_or(1.0);
            let name = str_field(item, "name", "");
            b.text(&format!("{}x {name}\n", fmt_qty(item.get("quantity"), qty)));
        }
    }

    b.text("================================\n");
    b.text(&format!("\n{}\n\n", t.label(Label::Signature)));
    b.cut();
}

pub(super) fn render_barcode_label(b: &mut EscposBuilder, data: &serde_json::Value) {
    b.set(Align::Center, true, false, false);
    b.text(&format!("{}\n", str_field(data, "product_name", "")));

    let barcode_value = str_field(data, "barcode", "");
    if !barcode_value.is_empty() {
        // Native `GS k` (EAN13/CODE128) with the HRI below.
        b.set(Align::Center, false, false, false);
        b.barcode(barcode_value);
    }

    if let Some(price) = data.get("price").and_then(|v| v.as_f64()) {
        b.set(Align::Center, true, true, false);
        let price = b.money(price);
        b.text(&format!("{price}\n"));
    }

    b.cut();
}

pub(super) fn render_generic(b: &mut EscposBuilder, data: &serde_json::Value) {
    let t = Locale::from_document(data);
    b.set(Align::Center, true, false, false);
    b.text(&format!("{}\n", str_field(data, "title", t.label(Label::GenericTitle))));
    b.text("================================\n");

    b.set(Align::Left, false, false, false);
    if let Some(obj) = data.as_object() {
        for (key, value) in obj {
            // `locale` steers HOW the paper is printed; it is a field of the envelope, not a line
            // of the document. Without this the generic renderer — which prints every key it does
            // not know — would print «locale: en» on the paper.
            if key == "title" || key == "receipt_id" || key == "locale" {
                continue;
            }
            b.text(&format!("{key}: {}\n", json_to_display(value)));
        }
    }

    b.text("================================\n\n");
    b.cut();
}
