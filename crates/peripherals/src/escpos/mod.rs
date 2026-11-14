//! ESC/POS rendering: turns a `document_type` plus its `data` (the Hub's JSON) into the bytes
//! sent to the printer.

mod builder;
mod cash_report;
mod fields;
mod kitchen;
mod locale;
mod prebill;
mod receipt;
mod short_documents;

pub use builder::{Align, EscposBuilder};

use crate::Result;
use builder::money_decimals;
use cash_report::render_cash_report;
use fields::{now_ymd_hms, str_field};
use kitchen::render_kitchen_order;
use locale::{Label, Locale};
use prebill::render_prebill;
use receipt::{check_full_invoice, render_receipt, Fiscal};
use serde::{Deserialize, Serialize};
use short_documents::{render_barcode_label, render_delivery_note, render_generic};

/// Standard paper width in columns (80 mm ≈ 32 characters).
pub const LINE_WIDTH: usize = 32;

/// **The name this crate signs paper with when the document does not bring the business's own.**
///
/// A constant and not a literal per renderer because it is the name of a PRODUCT, and products get
/// renamed or retired: with one literal per renderer, the test page and the receipt beside it end
/// up signed with different names. Paper cannot be corrected after it is cut, so the two must not
/// be able to drift apart.
pub const PRODUCT_NAME: &str = "ERPlora";

/// Module size, in dots, of a QR on the paper (fiscal and «pide tu factura»).
const QR_MODULE_DOTS: u8 = 4;

/// The promotional QR at 3 dots — 75 % of the fiscal one, the nearest whole dot to the
/// 70 % `<ok-receipt>` draws on screen.
const PROMO_QR_MODULE_DOTS: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentType {
    Receipt,
    KitchenOrder,
    Invoice,
    DeliveryNote,
    BarcodeLabel,
    CashSessionReport,
    /// **The bill taken to the table before charging.** Deliberately a document of its own and
    /// not a `Receipt`: it is the same list of lines, but it must NOT carry a series
    /// number, a payment method or a VeriFactu QR — the numbering is consumed on payment and there
    /// is no billing record yet — and it must carry the printed notice that it is not an invoice.
    /// Handing a customer paper that passes for an invoice is a legal problem, not a cosmetic one.
    Prebill,
    Generic,
}

impl DocumentType {
    /// Maps the protocol string, **refusing what it does not know**. The only door.
    ///
    /// A lenient mapping (unknown → `Generic`) is the most expensive kind of failure: a `Kitchen`
    /// or a `kitchn` rendered as `Generic` — a nameless dump of key/value pairs — gives the kitchen
    /// a piece of paper that is not an order, and **nobody finds out until the plate is missing**.
    /// A ticket that fails loudly is cheaper than one that fails quietly, so the caller gets a
    /// `None` it has to deal with, and no route can degrade to `Generic` in silence.
    ///
    /// It is deliberately the same vocabulary the hub's queue accepts
    /// (`erplora_runtime::print_queue::DOCUMENT_TYPES`) — two guards, two layers, different
    /// messages, so neither can be deleted with the suite still green.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "receipt" => Self::Receipt,
            "kitchen_order" => Self::KitchenOrder,
            "invoice" => Self::Invoice,
            "delivery_note" => Self::DeliveryNote,
            "barcode_label" => Self::BarcodeLabel,
            "cash_session_report" => Self::CashSessionReport,
            "prebill" => Self::Prebill,
            "generic" => Self::Generic,
            _ => return None,
        })
    }
}

/// Main render: document → ESC/POS bytes ready for the socket.
///
/// **A `data` that is not an object is refused.** Every renderer below reads its fields
/// by key (`data.get("items")`, `data.get("total")`); handed an array, a string or a `null` it finds
/// none of them, takes every default and cuts a **blank ticket** without erroring. That is a silent
/// failure at the very end of the chain — the one place where nobody is watching — so it stops here
/// and the print host reports it instead.
pub fn render_document(doc: DocumentType, data: &serde_json::Value) -> Result<Vec<u8>> {
    if !data.is_object() {
        return Err(crate::PeripheralError::InvalidPayload(format!(
            "a document is a JSON object of fields to print, not {}",
            match data {
                serde_json::Value::Null => "null",
                serde_json::Value::Bool(_) => "a boolean",
                serde_json::Value::Number(_) => "a number",
                serde_json::Value::String(_) => "a string",
                _ => "an array",
            }
        )));
    }
    // **A bill with no lines is refused, not cut blank.** Every renderer reads by key, so a
    // document in the SCREEN's shape (`business.name`, `lines[]` — what `ok-receipt` paints) finds
    // nothing it wants and prints «ERPlora», no lines and TOTAL 0.00 without erroring. Paper that
    // comes out WRONG is worse than paper that does not come out: nobody re-checks a bill that
    // printed. It is checked for the bill and not for every document because the others have
    // legitimately item-less shapes (a label, a cash report), and a guard that fires on a correct
    // document is a guard somebody deletes.
    if doc == DocumentType::Prebill
        && !data
            .get("items")
            .and_then(|v| v.as_array())
            .is_some_and(|items| !items.is_empty())
    {
        return Err(crate::PeripheralError::InvalidPayload(
            "a bill has `items` to charge for; this document has none (is it the screen's shape, \
             with `lines`?)"
                .to_string(),
        ));
    }
    if doc == DocumentType::Invoice {
        check_full_invoice(data)?;
    }
    let mut b = EscposBuilder::new().with_money_decimals(money_decimals(data));
    match doc {
        DocumentType::Receipt => render_receipt(&mut b, data, Fiscal::Ticket),
        // The same body as the ticket (lines, totals, QR, duplicate mark) plus what
        // makes it a FULL invoice; see [`check_full_invoice`] for the fields it reads.
        DocumentType::Invoice => render_receipt(&mut b, data, Fiscal::FullInvoice),
        // NOT an arm of `render_receipt` with a flag: what the bill must not print is precisely
        // what a receipt exists to print, so sharing the body would put the fiscal furniture one
        // forgotten `if` away from the paper the waiter hands over.
        DocumentType::Prebill => render_prebill(&mut b, data),
        DocumentType::KitchenOrder => render_kitchen_order(&mut b, data),
        DocumentType::DeliveryNote => render_delivery_note(&mut b, data),
        DocumentType::BarcodeLabel => render_barcode_label(&mut b, data),
        DocumentType::CashSessionReport => render_cash_report(&mut b, data),
        DocumentType::Generic => render_generic(&mut b, data),
    }
    Ok(b.finish())
}

/// The test page.
///
/// **It is paper like the rest of the paper.** It takes the same `data` every other renderer takes
/// and reads the same two fields off it, so it comes out in the hub's language under the shop's
/// own name, like the ticket printed a second later:
///
/// - `locale` → [`Locale::from_document`], the catalogue of this file. English is the SOURCE
///   language, never the paper's.
/// - `business_name` → the header, falling back to [`PRODUCT_NAME`] exactly as `render_receipt`
///   does. A shop owner checking which printer answered recognises their own name, not ours.
///
/// The envelope is OPTIONAL at the door (`erplora_test_print` takes `Option<Value>`): an
/// `erplora-app` newer than the module that calls it prints the fallback sheet rather than
/// nothing — and the fallback is Spanish, like every other document without a `locale`, which is
/// already the right answer for the fleet this ships to.
pub fn render_test_page(printer_id: &str, data: &serde_json::Value) -> Vec<u8> {
    let t = Locale::from_document(data);
    let mut b = EscposBuilder::new();
    b.set(Align::Center, false, false, false);
    b.text("================================\n");
    b.set(Align::Center, true, true, false);
    b.text(&format!("{}\n", str_field(data, "business_name", PRODUCT_NAME)));
    b.set(Align::Center, false, false, false);
    b.text("--------------------------------\n");
    b.text(&format!("{}\n", t.label(Label::TestPageOk)));
    b.text(&format!("{}\n", now_ymd_hms()));
    b.text("--------------------------------\n");
    b.text(&format!("{}{printer_id}\n", t.label(Label::TestPagePrinter)));
    b.text("================================\n");
    b.cut();
    b.finish()
}

#[cfg(test)]
mod tests;
