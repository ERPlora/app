use super::*;

// ── The X and the Z report on paper ────────────────────────────────────────────────────────
//
// The Hub queues ONE document type for both reports (`cash_register.session.print`): the open
// session's partial report (X) and the closed session's cash closing (Z). It tells them apart
// in `receipt_id` («<number> (X)» / «<number> (Z)»), sends the float as `opening_balance`, the
// EXPECTED cash as `closing_balance`, and every figure of the report — «Expected» included
// and, on the Z only, «Counted» and «Over/short» — as `transactions`, already in the language
// of whoever prints. So the paper must not title both «CIERRE DE CAJA», relabel the expected
// cash as «Cierre» or invent a «Diferencia» (closing − opening) next to the Hub's real
// over/short.

/// The Hub's `transactions` labels in `en` and in `es` (its `cash report` catalogue).
fn hub_report_label(locale: &str, en: &'static str) -> &'static str {
    if locale == "en" {
        return en;
    }
    match en {
        "Sales" => "Ventas",
        "Cash sales" => "Ventas en efectivo",
        "Card sales" => "Ventas con tarjeta",
        "Refunds" => "Devoluciones",
        "Cash in" => "Entradas",
        "Cash out" => "Salidas",
        "Comps" => "Invitaciones",
        "Expected" => "Esperado",
        "Counted" => "Contado",
        "Over/short" => "Descuadre",
        other => panic!("no Spanish for {other:?}"),
    }
}

/// The document the Hub queues today (`test_integration_printing.py`): a 100.00 float, a 1.50
/// cash sale and 0.50 taken out; `report` is `"X"` (open) or `"Z"` (closed, 100.90 counted).
fn hub_cash_report(report: &str, locale: &str) -> serde_json::Value {
    let mut figures = vec![
        ("Sales", 1.5),
        ("Cash sales", 1.5),
        ("Card sales", 0.0),
        ("Refunds", 0.0),
        ("Cash in", 0.0),
        ("Cash out", -0.5),
        ("Comps", 0.0),
        ("Expected", 101.0),
    ];
    if report == "Z" {
        figures.extend([("Counted", 100.9), ("Over/short", -0.1)]);
    }
    let transactions: Vec<serde_json::Value> = figures
        .into_iter()
        .map(|(en, amount)| json!({ "label": hub_report_label(locale, en), "amount": amount }))
        .collect();
    json!({
        "locale": locale,
        "receipt_id": format!("S-261023-0001 ({report})"),
        "cashier": "tester",
        "opening_balance": 100.0,
        "closing_balance": 101.0,
        "transactions": transactions,
    })
}

/// The cash report's lines as they reach the roll, trimmed.
fn cash_paper(data: &serde_json::Value) -> Vec<String> {
    money_paper(DocumentType::CashSessionReport, data)
        .into_iter()
        .map(|l| l.trim().to_string())
        .collect()
}

/// How many lines of the roll start with `label` — a figure printed twice is two of them.
fn lines_starting(lines: &[String], label: &str) -> usize {
    lines.iter().filter(|l| l.starts_with(label)).count()
}

/// An open session prints the PARTIAL report (X), titled as the Hub's screen titles it — never
/// «CIERRE DE CAJA»: the shift is not closed, and a cashier handing that paper over would be
/// handing over a closing that never happened.
#[test]
fn an_open_session_prints_the_partial_x_report_not_a_cash_closing() {
    for (locale, title, closing) in [
        ("es", "INFORME PARCIAL (X)", "CIERRE DE CAJA"),
        ("en", "PARTIAL REPORT (X)", "CASH CLOSING"),
    ] {
        let lines = cash_paper(&hub_cash_report("X", locale));
        assert!(
            lines.iter().any(|l| l == title),
            "{locale}: titled {title:?}:\n{lines:#?}"
        );
        assert!(
            !lines.iter().any(|l| l.contains(closing)),
            "{locale}: an X is not a closing:\n{lines:#?}"
        );
    }
}

/// The X carries no invented «Difference» and says the expected cash ONCE, under the Hub's own
/// label: `closing_balance` is that same expected cash, not a closing balance.
#[test]
fn the_x_report_has_no_invented_difference_and_says_the_expected_cash_once() {
    for (locale, opening, expected, closing, difference) in [
        ("es", "Apertura", "Esperado", "Cierre", "Diferencia"),
        ("en", "Opening", "Expected", "Closing", "Difference"),
    ] {
        let lines = cash_paper(&hub_cash_report("X", locale));
        assert!(
            line_of(&lines, opening).ends_with(" 100.00"),
            "{locale}:\n{lines:#?}"
        );
        assert_eq!(lines_starting(&lines, expected), 1, "{locale}:\n{lines:#?}");
        assert!(
            line_of(&lines, expected).ends_with(" 101.00"),
            "{locale}:\n{lines:#?}"
        );
        assert_eq!(
            lines_starting(&lines, closing),
            0,
            "{locale}: no «{closing}»:\n{lines:#?}"
        );
        assert_eq!(
            lines_starting(&lines, difference),
            0,
            "{locale}:\n{lines:#?}"
        );
    }
}

/// A closed session prints the cash closing (Z) with the Hub's count and over/short as its
/// ONLY difference — not a second, false one (closing − opening = 1.00 here, against a real
/// over/short of −0.10).
#[test]
fn the_z_report_prints_the_hub_over_short_as_its_only_difference() {
    for (locale, title, expected, counted, over_short, closing, difference) in [
        (
            "es",
            "CIERRE DE CAJA (Z)",
            "Esperado",
            "Contado",
            "Descuadre",
            "Cierre",
            "Diferencia",
        ),
        (
            "en",
            "CASH CLOSING (Z)",
            "Expected",
            "Counted",
            "Over/short",
            "Closing",
            "Difference",
        ),
    ] {
        let lines = cash_paper(&hub_cash_report("Z", locale));
        assert!(
            lines.iter().any(|l| l == title),
            "{locale}: titled {title:?}:\n{lines:#?}"
        );
        assert_eq!(lines_starting(&lines, expected), 1, "{locale}:\n{lines:#?}");
        assert!(
            line_of(&lines, counted).ends_with(" 100.90"),
            "{locale}:\n{lines:#?}"
        );
        assert_eq!(
            lines_starting(&lines, over_short),
            1,
            "{locale}:\n{lines:#?}"
        );
        assert!(
            line_of(&lines, over_short).ends_with(" -0.10"),
            "{locale}:\n{lines:#?}"
        );
        assert_eq!(
            lines_starting(&lines, closing),
            0,
            "{locale}: no «{closing}»:\n{lines:#?}"
        );
        assert_eq!(
            lines_starting(&lines, difference),
            0,
            "{locale}:\n{lines:#?}"
        );
    }
}

/// A cash report that does not say X or Z (an older or foreign producer) gets a NEUTRAL title:
/// it may well be an open session's, so it is never titled a closing. The figures it sends
/// still reach the paper; an invented difference does not.
#[test]
fn a_cash_report_without_x_or_z_gets_a_neutral_title() {
    for receipt_id in [
        json!("S-1"),
        json!("S-1 (Y)"),
        json!("(X) S-1"),
        json!(null),
    ] {
        for (locale, title, closing_title, opening, closing, difference) in [
            (
                "es",
                "INFORME DE CAJA",
                "CIERRE DE CAJA",
                "Apertura",
                "Cierre",
                "Diferencia",
            ),
            (
                "en",
                "CASH REPORT",
                "CASH CLOSING",
                "Opening",
                "Closing",
                "Difference",
            ),
        ] {
            let lines = cash_paper(&json!({
                "locale": locale, "receipt_id": receipt_id,
                "opening_balance": 100.0, "closing_balance": 150.0,
            }));
            let case = format!("{locale}, receipt_id {receipt_id}");
            assert!(
                lines.iter().any(|l| l == title),
                "{case}: titled {title:?}:\n{lines:#?}"
            );
            assert!(
                !lines.iter().any(|l| l.contains(closing_title)),
                "{case}:\n{lines:#?}"
            );
            assert!(
                line_of(&lines, opening).ends_with(" 100.00"),
                "{case}:\n{lines:#?}"
            );
            assert!(
                line_of(&lines, closing).ends_with(" 150.00"),
                "{case}:\n{lines:#?}"
            );
            assert_eq!(lines_starting(&lines, difference), 0, "{case}:\n{lines:#?}");
        }
    }
}

/// The Hub upper-cases the letter, but a report is an X or a Z however it is written, and the
/// spaces around it do not change that.
#[test]
fn the_report_letter_is_read_whatever_its_case_and_trailing_spaces() {
    for (receipt_id, title) in [
        ("S-261023-0001 (x)", "PARTIAL REPORT (X)"),
        ("S-261023-0001 (z)  ", "CASH CLOSING (Z)"),
        ("(Z)", "CASH CLOSING (Z)"),
    ] {
        let lines = cash_paper(&json!({ "locale": "en", "receipt_id": receipt_id }));
        assert!(
            lines.iter().any(|l| l == title),
            "{receipt_id:?} → {title:?}:\n{lines:#?}"
        );
    }
}
