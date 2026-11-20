use super::builder::{print_wrapped, wrap_to_width, Align, EscposBuilder};
use super::fields::{fmt_qty, is_truthy, modifier_lines, now_dmy_hm, str_field};
use super::locale::{Label, Locale};
use super::{LINE_WIDTH, PRODUCT_NAME, PROMO_QR_MODULE_DOTS};
use crate::Result;

/// Which fiscal paper [`render_receipt`] cuts: the ticket (simplified invoice) or the full invoice.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Fiscal {
    Ticket,
    FullInvoice,
}

/// Width of the rate column of the VAT breakdown; the base and the quota take 12 columns each, so
/// a row fills the 32 of the roll exactly.
const BREAKDOWN_RATE_WIDTH: usize = LINE_WIDTH - 2 * BREAKDOWN_AMOUNT_WIDTH;
const BREAKDOWN_AMOUNT_WIDTH: usize = 12;

/// **What an `invoice` must carry to be printed as a FULL invoice on the roll.**
///
/// The ticket's body reads neither the customer's tax id nor a per-rate breakdown, so a customer
/// who asked for their invoice would take home a ticket, and nothing would say so. Spanish tills
/// (Ágora, Revo, Glop) print the full invoice on the 80 mm roll, and it is valid as long as it
/// carries every datum RD 1619/2012 art. 6 asks for. So the document either brings them or is
/// REFUSED here, naming the field — the print host reports the refusal instead of cutting a mute
/// ticket. The contract (field → paper):
///
/// | Field | Required | Paper |
/// |---|---|---|
/// | `vat_number` | yes | issuer's tax id, under the business name |
/// | `receipt_id` | yes | `Factura: <number>` |
/// | `customer_name` | yes | `Cliente: <name>` |
/// | `customer_tax_id` | yes | `NIF: <tax id>`, right under the name |
/// | `customer_address` | no | `Dir: <address>`, under the tax id |
/// | `tax_breakdown` | yes, non-empty array | one row per rate: `{ rate, base, tax, label? }` |
///
/// `rate` is the percentage (`21`, `5.2`), `base` and `tax` are amounts in the document's unit
/// (the same as `total`), and `label` overrides the row's name (`RE 5.2%` for a surcharge; the
/// default is `IVA <rate>%`). The rest of the fields are the ticket's.
pub(super) fn check_full_invoice(data: &serde_json::Value) -> Result<()> {
    let refuse = |field: &str, what: &str| {
        Err(crate::PeripheralError::InvalidPayload(format!(
            "a full invoice needs `{field}` ({what})"
        )))
    };
    for (field, what) in [
        ("vat_number", "the issuer's tax id"),
        ("receipt_id", "the invoice number"),
        ("customer_name", "the customer's name"),
        ("customer_tax_id", "the customer's tax id"),
    ] {
        if str_field(data, field, "").trim().is_empty() {
            return refuse(field, what);
        }
    }
    let rows = data.get("tax_breakdown").and_then(|v| v.as_array());
    let valid = rows.is_some_and(|rows| {
        !rows.is_empty()
            && rows.iter().all(|row| {
                ["rate", "base", "tax"]
                    .iter()
                    .all(|k| row.get(k).and_then(|v| v.as_f64()).is_some())
            })
    });
    if !valid {
        return refuse(
            "tax_breakdown",
            "a non-empty array of `{ rate, base, tax }`, one per VAT rate",
        );
    }
    Ok(())
}

/// A rate as a person writes it: `21`, `5.2` — not `21.00` or `5.20`.
fn fmt_rate(rate: f64) -> String {
    let s = format!("{rate:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The VAT breakdown of a full invoice: a header naming the columns and one row per rate, the
/// amounts right-aligned in fixed columns so the bases and the quotas read as two columns. A
/// label wider than its column gets a line of its own rather than pushing the amounts out.
fn render_tax_breakdown(b: &mut EscposBuilder, data: &serde_json::Value, t: Locale) {
    let Some(rows) = data.get("tax_breakdown").and_then(|v| v.as_array()) else {
        return;
    };
    let row_line = |name: &str, base: &str, tax: &str| {
        format!(
            "{name:<rate_w$}{base:>amount_w$}{tax:>amount_w$}\n",
            rate_w = BREAKDOWN_RATE_WIDTH,
            amount_w = BREAKDOWN_AMOUNT_WIDTH
        )
    };
    b.set(Align::Left, false, false, false);
    b.text(&row_line(
        t.label(Label::BreakdownRate),
        t.label(Label::BreakdownBase),
        t.label(Label::BreakdownTax),
    ));
    for row in rows {
        let num = |k: &str| row.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
        let name = match row.get("label").and_then(|v| v.as_str()).map(str::trim) {
            Some(label) if !label.is_empty() => label.to_string(),
            _ => format!("{} {}%", t.label(Label::Tax), fmt_rate(num("rate"))),
        };
        let (base, tax) = (b.money(num("base")), b.money(num("tax")));
        if name.chars().count() > BREAKDOWN_RATE_WIDTH {
            b.text(&format!("{name}\n"));
            b.text(&row_line("", &base, &tax));
        } else {
            b.text(&row_line(&name, &base, &tax));
        }
    }
    b.text("--------------------------------\n");
}

/// Only one original of an invoice may exist (RD 1619/2012 art. 14): a reprint is a duplicate and
/// has to say so. Only an explicit `true` prints it, because the word is a statement about the
/// paper, not decoration a truthy string should switch on.
pub(super) fn print_duplicate_mark(b: &mut EscposBuilder, data: &serde_json::Value, t: Locale) {
    if data.get("duplicate").and_then(|v| v.as_bool()) == Some(true) {
        b.set(Align::Center, true, false, false);
        b.text(&format!("{}\n", t.label(Label::Duplicate)));
    }
}

/// What prints under an article's header line, on the ticket and on the bill alike: its menu
/// components and its supplements, one indented line each, or else its free note.
pub(super) fn print_item_detail(b: &mut EscposBuilder, item: &serde_json::Value) {
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
        b.set(Align::Left, false, false, false);
        b.text(&format!("  > {}\n", str_field(item, "notes", "")));
    }
}

/// The receipt; with [`Fiscal::FullInvoice`] it is also the full invoice.
#[expect(clippy::too_many_lines, reason = "renders the whole receipt top to bottom in one pass")]
pub(super) fn render_receipt(b: &mut EscposBuilder, data: &serde_json::Value, kind: Fiscal) {
    let t = Locale::from_document(data);

    // The fiscal QR (VeriFactu, `qr_data`) OPENS the ticket: the AEAT's QR specification
    // (v0.5.0 section 3, per Orden HAC/1177/2024 art. 21.1) places it «at the beginning of
    // the invoice, before the content generated by the invoicing system», always preceded by the
    // text «QR tributario:» above it. The producer sends that fixed legal text as `qr_heading`.
    // Heading and legend in bold so they read at least as clearly as the rest of the data.
    if is_truthy(data, "qr_data") {
        if is_truthy(data, "qr_heading") {
            b.set(Align::Center, true, false, false);
            b.text(&format!("{}\n", str_field(data, "qr_heading", "")));
        }
        b.set(Align::Center, false, false, false);
        b.qr(str_field(data, "qr_data", ""));
        // The legal legend of the fiscal QR («VERI*FACTU», RD 1619/2012 art. 6.5.b),
        // right under it (Orden HAC/1177/2024 art. 20.1.b). Only with the QR: alone it would
        // name nothing.
        if is_truthy(data, "qr_legend") {
            b.set(Align::Center, true, false, false);
            b.text(&format!("{}\n", str_field(data, "qr_legend", "")));
        }
        b.text("\n");
    }

    b.set(Align::Center, true, false, false);
    let business_name = str_field(data, "business_name", PRODUCT_NAME);
    b.text(&format!("{business_name}\n"));

    if is_truthy(data, "business_address") {
        b.set(Align::Center, false, false, false);
        print_wrapped(b, str_field(data, "business_address", ""));
    }

    if is_truthy(data, "vat_number") {
        b.text(&format!("{}{}\n", t.label(Label::VatNumber), str_field(data, "vat_number", "")));
    }

    if is_truthy(data, "phone") {
        b.text(&format!("{}{}\n", t.label(Label::Phone), str_field(data, "phone", "")));
    }

    print_duplicate_mark(b, data, t);

    b.text("================================\n");

    // The paper says what it is, before its data: the cheapest way to tell a full
    // invoice from a ticket at a glance.
    if kind == Fiscal::FullInvoice {
        // A corrective invoice says so in its title. Only an explicit `true`, like
        // `duplicate`: the word is a statement about the document.
        let title = if data.get("rectifying").and_then(|v| v.as_bool()) == Some(true) {
            Label::RectifyingInvoiceTitle
        } else {
            Label::InvoiceTitle
        };
        b.set(Align::Center, true, true, false);
        b.text(&format!("{}\n", t.label(title)));
    }

    b.set(Align::Left, false, false, false);
    let receipt_id = str_field(data, "receipt_id", "");
    let number_label = match kind {
        Fiscal::Ticket => Label::Ticket,
        Fiscal::FullInvoice => Label::InvoiceNumber,
    };
    b.text(&format!("{}{receipt_id}\n", t.label(number_label)));
    b.text(&format!("{}{}\n", t.label(Label::Date), now_dmy_hm()));

    if is_truthy(data, "cashier") {
        b.text(&format!("{}{}\n", t.label(Label::Cashier), str_field(data, "cashier", "")));
    }

    if is_truthy(data, "customer_name") {
        b.text(&format!("{}{}\n", t.label(Label::Customer), str_field(data, "customer_name", "")));
    }

    // The customer IDENTIFIED, right under their name. Only on the full invoice: the
    // ticket's paper does not move.
    if kind == Fiscal::FullInvoice {
        b.text(&format!("{}{}\n", t.label(Label::VatNumber), str_field(data, "customer_tax_id", "")));
        if is_truthy(data, "customer_address") {
            b.text(&format!("{}{}\n", t.label(Label::Address), str_field(data, "customer_address", "")));
        }
    }

    b.text("--------------------------------\n");

    if let Some(items) = data.get("items").and_then(|v| v.as_array()) {
        for item in items {
            let name = str_field(item, "name", "");
            let qty = item.get("quantity").and_then(|v| v.as_f64()).unwrap_or(1.0);
            let total = item.get("total").and_then(|v| v.as_f64()).unwrap_or(0.0);

            b.set(Align::Left, false, false, false);
            let line = format!("{}x {name}", fmt_qty(item.get("quantity"), qty));
            let total_str = b.money(total);
            let padding = (LINE_WIDTH as isize - line.len() as isize - total_str.len() as isize)
                .max(1) as usize;
            b.text(&format!("{line}{}{total_str}\n", " ".repeat(padding)));

            print_item_detail(b, item);
        }
    }

    b.text("--------------------------------\n");

    if kind == Fiscal::FullInvoice {
        render_tax_breakdown(b, data, t);
    }

    if let Some(subtotal) = data.get("subtotal").and_then(|v| v.as_f64()) {
        b.total_line(t.label(Label::Subtotal), subtotal);
    }

    if let Some(tax_amount) = data.get("tax_amount").and_then(|v| v.as_f64()) {
        let tax_label = str_field(data, "tax_label", t.label(Label::Tax));
        b.total_line(tax_label, tax_amount);
    }

    if let Some(discount) = data.get("discount").and_then(|v| v.as_f64()) {
        if discount > 0.0 {
            b.total_line(t.label(Label::Discount), -discount);
        }
    }

    b.text("================================\n");
    b.set(Align::Left, true, true, false);
    let total = data.get("total").and_then(|v| v.as_f64()).unwrap_or(0.0);
    b.total_line(t.label(Label::Total), total);
    b.set(Align::Left, false, false, false);
    b.text("================================\n");

    if is_truthy(data, "payment_method") {
        b.set(Align::Left, false, false, false);
        b.text(&format!("{}{}\n", t.label(Label::Payment), str_field(data, "payment_method", "")));
    }

    if let Some(paid) = data.get("paid").and_then(|v| v.as_f64()) {
        b.total_line(t.label(Label::Tendered), paid);
    }

    if let Some(change) = data.get("change").and_then(|v| v.as_f64()) {
        if change > 0.0 {
            b.total_line(t.label(Label::Change), change);
        }
    }

    // **The SECOND QR: «pide tu factura»** (ask for your invoice). It does not replace the one
    // above, which is why there are two fields: the VeriFactu one points at the AEAT's site
    // (`ValidarQR`) and is for CHECKING; this one points at this hub and is for ASKING. Ágora
    // prints «CREAR FACTURA», Cuiner prints a locator + QR — and both put it at the foot, next to
    // the fiscal one, on the same paper.
    //
    // The **text locator** goes underneath and is not decorative: it is the only way in when the
    // camera does not focus, the phone has no battery or the ticket is photocopied. That is why
    // the core's Crockford alphabet avoids `I`, `L`, `O` and `U` — it is typed from thermal
    // paper, not read off a screen.
    if is_truthy(data, "claim_qr_data") {
        b.set(Align::Center, false, false, false);
        if is_truthy(data, "claim_note") {
            print_wrapped(b, str_field(data, "claim_note", ""));
        }
        b.qr(str_field(data, "claim_qr_data", ""));
        if is_truthy(data, "claim_locator") {
            b.text(&format!("{}\n", str_field(data, "claim_locator", "")));
        }
    }

    b.text("\n");
    if is_truthy(data, "receipt_header") {
        b.set(Align::Center, false, false, false);
        print_wrapped(b, str_field(data, "receipt_header", ""));
    }

    if is_truthy(data, "receipt_footer") {
        b.set(Align::Center, false, false, false);
        print_wrapped(b, str_field(data, "receipt_footer", ""));
    }

    b.set(Align::Center, false, false, false);
    b.text(&format!("\n{}\n\n", t.label(Label::Thanks)));

    // **The business's promotional QR**: reviews, social media — the link it set in the POS
    // settings. It closes the paper, like `<ok-receipt>` and the browser paper, and prints smaller
    // than the fiscal QR. Without the URL nothing prints: a note alone names nothing. Only on the
    // ticket — the full invoice is formal, like the A4 invoice.
    if kind == Fiscal::Ticket && is_truthy(data, "promo_qr") {
        if is_truthy(data, "promo_note") {
            print_wrapped(b, str_field(data, "promo_note", ""));
        }
        b.qr_sized(str_field(data, "promo_qr", ""), PROMO_QR_MODULE_DOTS);
        b.text("\n");
    }

    b.cut();
}
