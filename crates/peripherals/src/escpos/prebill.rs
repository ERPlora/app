use super::builder::{print_wrapped, wrap_to_width, Align, EscposBuilder};
use super::fields::{fmt_qty, is_truthy, modifier_lines, now_dmy_hm, str_field};
use super::locale::{Label, Locale};
use super::{LINE_WIDTH, PRODUCT_NAME};

/// **The bill taken to the table before charging.**
///
/// The waiter's most frequent piece of paper: it comes out once before every payment, so more often
/// than the fiscal ticket. It is the same list of lines as a receipt and, on purpose, **not the
/// same document**:
///
/// - **no series number** — the numbering is consumed by `complete_sale`, not before;
/// - **no VeriFactu QR** — there is no billing record to point at yet;
/// - **no payment method, amount tendered or change** — nothing has been charged;
/// - **a printed notice** that this is not an invoice.
///
/// Those four are dropped **here**, not trusted to the producer: a caller that hands over its
/// receipt document by mistake gets a bill, never a paper that passes for an invoice. Handing a
/// customer one of those is a legal problem, not a cosmetic one.
///
/// The labels come from this file's catalogue, in the document's `locale`; a producer that wants
/// its own wording for the notice sends it in `notice`.
pub(super) fn render_prebill(b: &mut EscposBuilder, data: &serde_json::Value) {
    let t = Locale::from_document(data);
    // The title is the first thing anyone reads and the cheapest way to tell this paper from a
    // ticket at a glance — the same job `COCINA` does for the kitchen order.
    b.set(Align::Center, true, true, false);
    b.text(&format!("{}\n", t.label(Label::BillTitle)));

    b.set(Align::Center, true, false, false);
    b.text(&format!("{}\n", str_field(data, "business_name", PRODUCT_NAME)));

    if is_truthy(data, "business_address") {
        b.set(Align::Center, false, false, false);
        print_wrapped(b, str_field(data, "business_address", ""));
    }

    b.text("================================\n");

    b.set(Align::Left, false, false, false);
    // What identifies a bill is the table, not a number: it is how the waiter knows which of the
    // six he is holding.
    if is_truthy(data, "customer_name") {
        b.text(&format!("{}{}\n", t.label(Label::TableOrCustomer), str_field(data, "customer_name", "")));
    }
    b.text(&format!("{}{}\n", t.label(Label::Date), now_dmy_hm()));
    b.text("--------------------------------\n");

    if let Some(items) = data.get("items").and_then(|v| v.as_array()) {
        for item in items {
            let name = str_field(item, "name", "");
            let qty = item.get("quantity").and_then(|v| v.as_f64()).unwrap_or(1.0);
            let total = item.get("total").and_then(|v| v.as_f64()).unwrap_or(0.0);
            b.set(Align::Left, false, false, false);
            b.total_line(&format!("{}x {name}", fmt_qty(item.get("quantity"), qty)), total);
            // The MENU: `sales` (>= 2.16.10) sends `components: string[]` (labels already
            // composed, supplement included) besides the one-line `notes`. One indented line per
            // component, normal weight, NO amount — the header is the line that carries the money,
            // so it is the components that step back (the opposite emphasis of the kitchen chit,
            // same indentation). `sales` also joins the same components into `notes`, so when the
            // list is there the note is skipped: printing both would say everything twice. An item
            // without the list prints its `notes`.
            let components: Vec<String> = item
                .get("components")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|c| c.as_str())
                        .filter(|c| !c.trim().is_empty())
                        .map(|c| c.to_string())
                        .collect()
                })
                .unwrap_or_default();
            // The SUPPLEMENTS of the line, structured. `components` says what the MENU is made
            // of; `modifiers` says what the customer CHANGED. They are different axes, so both
            // print, components first and the change after — the same order the kitchen chit uses.
            // Chained into `notes` they would wrap at 32 columns and the continuation would start
            // at the margin, reading as another article; one line each, indented, avoids that.
            //
            // No amount of their own: in ERPlora the delta already lives inside the line's
            // `unit_price` (`authoritative_modifiers`), so the printed `line_total` includes it and
            // a second number in that column would not add up with the rest.
            let supplements = modifier_lines(item);
            if !components.is_empty() || !supplements.is_empty() {
                b.set(Align::Left, false, false, false);
                for entry in components.iter().chain(supplements.iter()) {
                    // Wrapped by hand keeping the indent: cut at 32 columns by the printer, the
                    // continuation would start at the margin and read as another item.
                    for line in wrap_to_width(entry, LINE_WIDTH - 2) {
                        b.text(&format!("  {line}\n"));
                    }
                }
            } else if is_truthy(item, "notes") {
                b.text(&format!("  > {}\n", str_field(item, "notes", "")));
            }
        }
    }

    b.text("--------------------------------\n");

    if let Some(subtotal) = data.get("subtotal").and_then(|v| v.as_f64()) {
        b.total_line(t.label(Label::Subtotal), subtotal);
    }
    if let Some(tax_amount) = data.get("tax_amount").and_then(|v| v.as_f64()) {
        b.total_line(str_field(data, "tax_label", t.label(Label::Tax)), tax_amount);
    }
    if let Some(discount) = data.get("discount").and_then(|v| v.as_f64()) {
        if discount > 0.0 {
            b.total_line(t.label(Label::Discount), -discount);
        }
    }

    b.text("================================\n");
    b.set(Align::Left, true, true, false);
    b.total_line(t.label(Label::Total), data.get("total").and_then(|v| v.as_f64()).unwrap_or(0.0));
    b.set(Align::Left, false, false, false);
    b.text("================================\n\n");

    // The notice is what makes this paper legal, so it has a default: a producer that forgets to
    // send it must not be able to print something that passes for an invoice.
    b.set(Align::Center, false, false, false);
    let notice = match data.get("notice").and_then(|v| v.as_str()) {
        Some(text) if !text.trim().is_empty() => text,
        _ => t.label(Label::BillNotice),
    };
    for line in wrap_to_width(notice, LINE_WIDTH) {
        b.text(&format!("{line}\n"));
    }

    b.text("\n");
    b.cut();
}
