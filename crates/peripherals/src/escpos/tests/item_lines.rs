use super::*;

// ── The MENU on the customer's ticket and on the bill ──────────────────────────────────────
//
// `sales` (>= 2.16.10) sends, besides `notes`, `components: string[]` on a menu item: labels
// already composed (name + supplement «(+3,00)»), in the order they were chosen. At 32 columns
// the one-line `notes` wraps and the continuation starts at the margin — it reads as another
// item. The kitchen chit solves the same thing; here the header keeps its normal weight (it
// carries the money) and the components step back: indent, no amount, no bold.

/// One line per component, indented, WITHOUT an amount — under its item, before anything else.
#[test]
fn a_menu_item_prints_one_indented_line_per_component_on_the_ticket() {
    let text = ticket(&json!({
        "items": [{ "name": "Menu del dia", "quantity": 1, "total": 16.5,
                    "components": ["Gazpacho", "Solomillo (+3,00)"] }],
        "total": 16.5,
    }));
    let header = text.find("Menu del dia").expect("the menu line");
    let first = text.find("  Gazpacho").expect("first component, indented");
    let second = text.find("  Solomillo (+3,00)").expect("second component, indented");
    assert!(header < first && first < second, "components in order, under their item:\n{text}");
    // No amount of their own: the only money on a component is the supplement in its label.
    let gazpacho_line = text.lines().find(|l| l.contains("Gazpacho")).unwrap();
    assert!(!gazpacho_line.contains("16.5"), "a component never carries the item price:\n{text}");
}

/// With `components` present, `notes` is NOT printed: `notes` also carries the same components
/// joined by « · », and printing both would say everything twice.
#[test]
fn components_and_notes_do_not_print_the_components_twice() {
    let text = ticket(&json!({
        "items": [{ "name": "Menu del dia", "quantity": 1, "total": 16.5,
                    "components": ["Gazpacho", "Cerveza"],
                    "notes": "Gazpacho · Cerveza" }],
        "total": 16.5,
    }));
    assert_eq!(text.matches("Gazpacho").count(), 1, "the component prints ONCE:\n{text}");
    assert!(!text.contains("  > "), "the one-line note is replaced by the list:\n{text}");
}

/// An item WITHOUT `components` prints byte-identical to today: the deployed fleet's tickets
/// must not move (unknown keys are ignored, known shapes are frozen).
#[test]
fn an_item_without_components_prints_exactly_as_today() {
    let plain = json!({
        "items": [{ "name": "Cafe solo", "quantity": 1, "total": 1.8, "notes": "sin sal" }],
        "total": 1.8,
    });
    let text = ticket(&plain);
    assert!(text.contains("  > sin sal"), "the note keeps its shape:\n{text}");
    let with_empty = json!({
        "items": [{ "name": "Cafe solo", "quantity": 1, "total": 1.8, "notes": "sin sal",
                    "components": [] }],
        "total": 1.8,
    });
    let a = render_document(DocumentType::Receipt, &plain).expect("valid");
    let b2 = render_document(DocumentType::Receipt, &with_empty).expect("valid");
    assert_eq!(a, b2, "an empty list is the same as no list — byte for byte");
}

/// A long component wraps keeping its indent: the continuation must not reach the margin, or
/// it reads as another item — the exact defect of the one-line note.
#[test]
fn a_long_component_wraps_and_keeps_its_indent() {
    let text = ticket(&json!({
        "items": [{ "name": "Menu", "quantity": 1, "total": 16.5,
                    "components": ["Solomillo de ternera gallega a la brasa con pimientos (+3,00)"] }],
        "total": 16.5,
    }));
    let cont: Vec<&str> = text
        .lines()
        .skip_while(|l| !l.contains("Solomillo"))
        .skip(1)
        .take_while(|l| !l.contains("TOTAL") && !l.starts_with("--"))
        .collect();
    assert!(!cont.is_empty(), "the component wrapped somewhere:\n{text}");
    for l in cont {
        assert!(l.starts_with("  "), "every continuation keeps the indent: {l:?}\n{text}");
    }
}

/// The bill taken to the table says the same as the ticket (one composer, two papers):
/// components indented under the menu header, no note duplication.
#[test]
fn the_prebill_indents_the_menu_components_too() {
    let data = json!({
        "items": [{ "name": "Menu del dia", "quantity": 1, "total": 16.5,
                    "components": ["Gazpacho", "Solomillo (+3,00)"],
                    "notes": "Gazpacho · Solomillo (+3,00)" }],
        "total": 16.5,
        "notice": "Cuenta - no es una factura.",
    });
    let bytes = render_document(DocumentType::Prebill, &data).expect("valid bill");
    let text = strip_escpos(&bytes);
    assert!(text.contains("  Gazpacho"), "component indented on the bill:\n{text}");
    assert_eq!(text.matches("Gazpacho").count(), 1, "and only once:\n{text}");
    assert!(!text.contains("  > "), "no one-line note next to the list:\n{text}");
}

/// Ticket helper: same door as the kitchen `paper()` but for the customer's receipt.
fn ticket(data: &serde_json::Value) -> String {
    let bytes = render_document(DocumentType::Receipt, data).expect("valid ticket");
    strip_escpos(&bytes)
}

// ── The SUPPLEMENT on the customer's ticket and on the bill ─────────────────────────────────
//
// The chit prints supplements one per line. Read only from `notes`, where `sales` chains them
// with « · », two or three of them would wrap at 32 columns and the continuation would start at
// the margin, reading as another article. The menu's `components` have the same problem and
// the same fix: the structured list wins and the joined note steps aside, so nothing is said
// twice.

/// One indented line per supplement, and NO amount of its own: in ERPlora the delta is already
/// inside the line's `unit_price` (`authoritative_modifiers`), so a second number in that
/// column would not add up with the rest — a bug Odoo has.
#[test]
fn a_supplement_reaches_the_customers_ticket_on_its_own_line() {
    let text = printed(
        DocumentType::Receipt,
        &json!({
            "items": [{ "name": "Hamburguesa", "quantity": 1, "total": 10.0,
                        "modifiers": ["Extra queso", "Sin cebolla"] }],
            "total": 10.0,
        }),
    );
    let dish = text.find("Hamburguesa").expect("the dish");
    let first = text.find("  Extra queso").expect("the first supplement, indented");
    let second = text.find("  Sin cebolla").expect("the second supplement, indented");
    assert!(dish < first && first < second, "in order, under their dish:\n{text}");
    let line = text.lines().find(|l| l.contains("Extra queso")).unwrap();
    assert_eq!(line.trim(), "Extra queso", "nothing else on the line:\n{text}");
    assert!(!line.contains("10.0"), "a supplement carries no amount of its own:\n{text}");
}

/// The bill taken to the table gets exactly the same treatment — it is the paper the waiter
/// hands over, and it comes out more often than the ticket.
#[test]
fn a_supplement_reaches_the_bill_on_its_own_line() {
    let text = printed(
        DocumentType::Prebill,
        &json!({
            "items": [{ "name": "Hamburguesa", "quantity": 1, "total": 10.0,
                        "modifiers": ["Extra queso", "Sin cebolla"] }],
            "total": 10.0,
        }),
    );
    assert!(text.contains("  Extra queso"), "the supplement, indented:\n{text}");
    assert!(text.contains("  Sin cebolla"), "and the second one:\n{text}");
}

/// With `modifiers` present the joined `notes` is NOT printed: the note also carries the same
/// supplements chained with « · », and printing both would say everything twice.
#[test]
fn a_structured_supplement_replaces_the_joined_note() {
    for doc in [DocumentType::Receipt, DocumentType::Prebill] {
        let text = printed(
            doc,
            &json!({
                "items": [{ "name": "Hamburguesa", "quantity": 1, "total": 10.0,
                            "modifiers": ["Extra queso", "Sin cebolla"],
                            "notes": "Extra queso · Sin cebolla" }],
                "total": 10.0,
            }),
        );
        assert_eq!(
            text.matches("Extra queso").count(),
            1,
            "{doc:?}: the supplement prints ONCE:\n{text}"
        );
        assert!(!text.contains("  > "), "{doc:?}: the joined note steps aside:\n{text}");
    }
}

/// A MENU with a supplement carries BOTH lists: `components` says what the menu is made of and
/// `modifiers` says what was changed. They are different axes, so both print — components
/// first, the change after — and the joined note steps aside for both.
#[test]
fn a_menu_prints_its_components_and_its_supplements_both() {
    let text = printed(
        DocumentType::Receipt,
        &json!({
            "items": [{ "name": "Menu del dia", "quantity": 1, "total": 16.5,
                        "components": ["Gazpacho", "Solomillo"],
                        "modifiers": ["Sin sal"],
                        "notes": "Gazpacho · Solomillo · Sin sal" }],
            "total": 16.5,
        }),
    );
    let gazpacho = text.find("  Gazpacho").expect("the component");
    let solomillo = text.find("  Solomillo").expect("the second component");
    let sin_sal = text.find("  Sin sal").expect("the supplement");
    assert!(
        gazpacho < solomillo && solomillo < sin_sal,
        "components first, then what was changed:\n{text}"
    );
    assert!(!text.contains("  > "), "the joined note is not repeated on top:\n{text}");
}

/// A free note with NO structured list still prints as it always did — that is the whole
/// deployed fleet, and `notes` is still where a waiter's text arrives.
#[test]
fn a_free_note_without_any_list_prints_exactly_as_today() {
    for doc in [DocumentType::Receipt, DocumentType::Prebill] {
        let text = printed(
            doc,
            &json!({
                "items": [{ "name": "Cafe", "quantity": 1, "total": 1.8, "notes": "sin sal" }],
                "total": 1.8,
            }),
        );
        assert!(text.contains("  > sin sal"), "{doc:?}: the note keeps its shape:\n{text}");
    }
}

/// An empty list is the same as no list, byte for byte: a module that sends `[]` must not add
/// a blank indented line under every article.
#[test]
fn an_empty_supplement_list_leaves_the_ticket_untouched() {
    let plain = json!({
        "items": [{ "name": "Cafe", "quantity": 1, "total": 1.8, "notes": "sin sal" }],
        "total": 1.8,
    });
    let empty = json!({
        "items": [{ "name": "Cafe", "quantity": 1, "total": 1.8, "notes": "sin sal",
                    "modifiers": [] }],
        "total": 1.8,
    });
    for doc in [DocumentType::Receipt, DocumentType::Prebill] {
        assert_eq!(
            render_document(doc, &plain).expect("valid"),
            render_document(doc, &empty).expect("valid"),
            "{doc:?}: an empty list is the same as no list",
        );
    }
}

/// A long supplement wraps keeping its indent on the ticket too: cut at 32 columns by the
/// printer, the continuation would start at the margin and read as another article.
#[test]
fn a_long_supplement_on_the_ticket_keeps_its_indent_when_it_wraps() {
    let text = printed(
        DocumentType::Receipt,
        &json!({
            "items": [{ "name": "Hamburguesa", "quantity": 1, "total": 10.0,
                        "modifiers": ["Sin cebolla y sin pepinillos y con la carne muy hecha"] }],
            "total": 10.0,
        }),
    );
    let wrapped: Vec<&str> = text
        .lines()
        .skip_while(|l| !l.contains("Sin cebolla"))
        .take_while(|l| !l.starts_with("--"))
        .collect();
    assert!(wrapped.len() > 1, "it wrapped somewhere:\n{text}");
    for l in &wrapped {
        assert!(l.starts_with("  "), "every piece keeps the indent: {l:?}\n{text}");
        assert!(l.chars().count() <= LINE_WIDTH, "and fits the paper: {l:?}");
    }
}

/// The bill is the same list of lines as the ticket, so every article prints there byte for byte
/// as on the ticket: its header, its menu components, its supplements and its free note, with the
/// same printer modes. Two copies of that code end up saying different things on the two papers.
#[test]
fn the_bill_prints_every_article_byte_for_byte_as_the_ticket() {
    let items = [
        json!({ "name": "Menu del dia", "quantity": 1, "total": 16.5,
                "components": ["Gazpacho", "Solomillo (+3,00)"], "notes": "Gazpacho · Solomillo" }),
        json!({ "name": "Hamburguesa", "quantity": 2, "total": 20.0,
                "modifiers": ["Extra queso", "Sin cebolla y sin pepinillos y con la carne muy hecha"] }),
        json!({ "name": "Menu", "quantity": 1, "total": 12.0,
                "components": ["Ensalada"], "modifiers": ["Sin sal"] }),
        json!({ "name": "Cafe solo", "quantity": 1, "total": 1.8, "notes": "sin sal" }),
        json!({ "name": "Agua", "quantity": 1.5, "total": 1.5 }),
    ];
    for item in items {
        let data = json!({ "items": [item], "total": 1.0 });
        let ticket = render_document(DocumentType::Receipt, &data).expect("valid ticket");
        let bill = render_document(DocumentType::Prebill, &data).expect("valid bill");
        assert_eq!(
            article_lines(&bill),
            article_lines(&ticket),
            "the bill and the ticket print this article differently: {item}",
        );
    }
}

/// The bytes between the two dashed rules that frame the articles of a ticket or a bill.
fn article_lines(paper: &[u8]) -> &[u8] {
    let rule = "-".repeat(LINE_WIDTH) + "\n";
    let mut frames = paper
        .windows(rule.len())
        .enumerate()
        .filter(|(_, w)| *w == rule.as_bytes())
        .map(|(at, _)| at);
    let start = frames.next().expect("the rule that opens the articles") + rule.len();
    let end = frames.next().expect("the rule that closes the articles");
    &paper[start..end]
}
