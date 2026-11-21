use super::builder::{Align, EscposBuilder};
use super::fields::{is_truthy, paper_date, str_field};
use super::locale::{Label, Locale};

/// **Which report a `cash_session_report` is.**
///
/// The Hub queues one document type for both and says which in `receipt_id`: «<number> (X)» for
/// an open session, «<number> (Z)» for a closed one. Anything else — an older or foreign producer —
/// is neither, and gets the neutral title.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CashReport {
    /// The partial report of an open session (X).
    Partial,
    /// The cash closing of a closed session (Z).
    Closing,
}

impl CashReport {
    fn of(data: &serde_json::Value) -> Option<Self> {
        let id = data.get("receipt_id").and_then(|v| v.as_str())?.trim_end();
        // `get` refuses a cut inside a multi-byte character: such an ending is no «(X)» either.
        let suffix = id.get(id.len().saturating_sub(3)..)?;
        match suffix.to_ascii_uppercase().as_str() {
            "(X)" => Some(Self::Partial),
            "(Z)" => Some(Self::Closing),
            _ => None,
        }
    }
}

/// The cash-up report, with the X and the Z of the Hub.
///
/// The Hub's document: `receipt_id` (see [`CashReport`]), `cashier`, the float as
/// `opening_balance`, the EXPECTED cash as `closing_balance`, and every figure of the report as
/// `transactions` (`{label, amount}`, already in the language of whoever prints: «Expected» on
/// both, «Counted» and «Over/short» on the Z). So an X or a Z prints the float and those lines,
/// and nothing else: `closing_balance` again would say the expected cash twice, the second time
/// as «Closing». No report prints a «Difference» of its own: closing − opening is not the
/// over/short, and the Hub's over/short is already one of the lines.
pub(super) fn render_cash_report(b: &mut EscposBuilder, data: &serde_json::Value) {
    let t = Locale::from_document(data);
    let report = CashReport::of(data);
    let title = match report {
        Some(CashReport::Partial) => Label::PartialReportTitle,
        Some(CashReport::Closing) => Label::CashClosingTitle,
        None => Label::CashReportTitle,
    };
    b.set(Align::Center, true, false, false);
    b.text(&format!("{}\n", t.label(title)));
    b.text("================================\n");

    b.set(Align::Left, false, false, false);
    b.text(&format!("{}{}\n", t.label(Label::Session), str_field(data, "receipt_id", "")));
    b.text(&format!("{}{}\n", t.label(Label::Date), paper_date(data)));

    if is_truthy(data, "cashier") {
        b.text(&format!("{}{}\n", t.label(Label::Cashier), str_field(data, "cashier", "")));
    }

    b.text("--------------------------------\n");

    let opening = data.get("opening_balance").and_then(|v| v.as_f64()).unwrap_or(0.0);
    b.total_line(t.label(Label::Opening), opening);
    if report.is_none() {
        let closing = data.get("closing_balance").and_then(|v| v.as_f64()).unwrap_or(0.0);
        b.total_line(t.label(Label::Closing), closing);
    }

    b.text("--------------------------------\n");

    if let Some(transactions) = data.get("transactions").and_then(|v| v.as_array()) {
        for tx in transactions {
            let label = tx
                .get("label")
                .and_then(|v| v.as_str())
                .or_else(|| tx.get("type").and_then(|v| v.as_str()))
                .unwrap_or("");
            let amount = tx.get("amount").and_then(|v| v.as_f64()).unwrap_or(0.0);
            b.total_line(label, amount);
        }
    }

    b.text("================================\n\n");
    b.cut();
}
