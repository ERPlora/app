use super::*;
use crate::escpos::fields::now_hm;

// ── The kitchen chit prints what the row carries ────────────────────────────────────────────

/// Renders a kitchen chit and returns the paper **without control bytes**, which is what a cook
/// reads. Without this, every assertion compares against text sprinkled with `ESC a`/`GS !` and
/// ends up passing by accident.
fn paper(data: &serde_json::Value) -> String {
    let bytes = render_document(DocumentType::KitchenOrder, data).expect("a valid kitchen chit");
    strip_escpos(&bytes)
}

/// **The supplement comes out of the printer, not only on the screen.**
///
/// «Sin cebolla» is frozen into the row and the KDS paints it, but the hot kitchen IS paper: at
/// the grill nobody looks at a screen with their hands full. A supplement that does not print
/// is the dish that comes back, and that is not cosmetic.
#[test]
fn a_supplement_reaches_the_paper_under_its_own_line() {
    let text = paper(&json!({
        "receipt_id": "K-217",
        "items": [{ "name": "Entrecot", "quantity": 1, "modifiers": "Al punto, sin cebolla" }],
    }));
    assert!(text.contains("Entrecot"), "the dish is on the paper:\n{text}");
    assert!(
        text.contains("Al punto, sin cebolla"),
        "the supplement is on the paper — it is what sends the dish back:\n{text}"
    );
    // Under ITS dish, not at the end of the sheet: on an eight-line chit, a supplement loose at
    // the foot does not say which dish it belongs to.
    let plato = text.find("Entrecot").expect("the dish");
    let suplemento = text.find("Al punto").expect("the supplement");
    assert!(suplemento > plato, "the supplement goes UNDER its line:\n{text}");
}

/// **The supplement and the note live side by side, and the supplement goes first.** They are
/// two different things: the customer chose the supplement from the menu and it changes the
/// dish; the note is the waiter's free text. Printing one must not lose the other.
#[test]
fn a_line_can_carry_both_a_supplement_and_a_free_note() {
    let text = paper(&json!({
        "items": [{ "name": "Croquetas", "quantity": 2, "modifiers": "Sin gluten", "notes": "para compartir" }],
    }));
    let mods = text.find("Sin gluten").expect("the supplement is on the paper");
    let notes = text.find("para compartir").expect("the note is on the paper");
    assert!(mods < notes, "the supplement goes before the free note:\n{text}");
}

/// **The kitchen chit says which table it is for.**
///
/// The shell sends `label`; a renderer that read only `table`, a field nobody sends, would print
/// a chit without the one thing the kitchen knows about the room. At rush hour, a chit without
/// a table is paper that is good for nothing.
///
/// The label prints **as it comes**, with no prefix: it is opaque on purpose — «Mesa 4»,
/// «Barra», «Recogida Ana». The kitchen does not know what a table is, nor does it need to.
#[test]
fn the_kitchen_order_says_which_table_it_is_for() {
    let text = paper(&json!({
        "receipt_id": "K-217",
        "label": "Mesa 4",
        "round_number": 2,
        "items": [{ "name": "Croquetas", "quantity": 2 }],
    }));
    assert!(text.contains("Mesa 4"), "the room label is on the paper:\n{text}");
    assert!(!text.contains("Mesa: Mesa 4"), "as it comes, with no prefix: the label is opaque:\n{text}");
    // The round tells the second course from the first at the same table. The whole word is
    // asserted: `contains('2')` would pass on the «K-217» above — a check that is right by
    // accident proves nothing.
    assert!(text.contains("Ronda 2"), "the round is on the paper:\n{text}");
}

/// **The VOID slip of a cancelled round is a `kitchen_order`.** The shell (`print-void.ts`)
/// says the void with what an installed app that never updates already prints: the word in the
/// floor label and a negative quantity per dish. If this renderer ever dropped
/// the sign (or clamped it to 1), the slip would read as a NEW round and the dish would be
/// cooked twice — so the sign is pinned here, where the paper is made.
#[test]
fn a_void_slip_prints_the_word_and_negative_quantities() {
    // Read back with the cp437 decoder: `paper` decodes as UTF-8 and the `·` of the label
    // (cp437 0xFA) would come out as U+FFFD even though the printer gets it right.
    let bytes = render_document(DocumentType::KitchenOrder, &json!({
        "receipt_id": "K-217",
        "label": "ANULADA · Mesa 4",
        "round_number": 2,
        "items": [
            { "name": "Croquetas", "quantity": -2 },
            { "name": "Flan", "quantity": -0.5 },
        ],
    }))
    .expect("a void slip is a valid kitchen order");
    let text = roll_lines(&bytes).join("\n");
    assert!(text.contains("ANULADA · Mesa 4"), "the void word and the table:\n{text}");
    assert!(text.contains("-2x Croquetas"), "the dish with its negative quantity:\n{text}");
    assert!(text.contains("-0.5x Flan"), "a half ration keeps its sign:\n{text}");
    assert!(!text.contains("URGENTE"), "a void is never urgent:\n{text}");
}

/// **The first round is NOT announced.** It is the normal case — a chit that is not a second
/// course — and printing «Ronda 1» would add a line of noise to 99 % of the paper. The round
/// says «this is already the second send for this table», and that is only true from two on.
#[test]
fn the_first_round_is_not_announced() {
    let text = paper(&json!({
        "label": "Mesa 4", "round_number": 1,
        "items": [{ "name": "Croquetas", "quantity": 2 }],
    }));
    assert!(text.contains("Mesa 4"), "the table is:\n{text}");
    assert!(!text.contains("Ronda"), "the round is not:\n{text}");
}

/// The Hub writes the allergy lines in capitals, and cp437 has no Á: it prints as A, never `?`.
#[test]
fn an_allergy_in_capitals_prints_its_letters_not_question_marks() {
    let text = paper(&json!({
        "items": [{ "name": "ALBÓNDIGAS", "quantity": 1, "modifiers": ["*** ALERGIA: CRUSTÁCEOS ***"] }],
    }));
    assert!(text.contains("1x ALBONDIGAS"), "{text}");
    assert!(text.contains("*** ALERGIA: CRUSTACEOS ***"), "{text}");
    assert!(!text.contains('?'), "{text}");
}

/// **A long supplement is wrapped keeping its indent, not cut by the printer.** At 32 columns,
/// «Al punto, sin cebolla, sin sal, extra salsa» overflows; if the thermal printer cuts it, the
/// rest starts at the margin and reads as one more dish of the order.
#[test]
fn a_long_supplement_wraps_keeping_its_indent() {
    let text = paper(&json!({
        "items": [{
            "name": "Entrecot", "quantity": 1,
            "modifiers": "Al punto, sin cebolla, sin sal, extra salsa aparte",
        }],
    }));
    let partes: Vec<&str> = text
        .lines()
        .filter(|l| l.contains("salsa") || l.contains("Al punto"))
        .collect();
    // Without this the `for` below would never run and the test would pass on a chit that does
    // not even print the supplement — an empty loop proves nothing.
    assert!(
        partes.len() >= 2,
        "the long supplement is split over several lines, not lost:\n{text}"
    );
    for line in partes {
        assert!(line.starts_with("   "), "the continuation keeps the indent: {line:?}");
        assert!(
            line.chars().count() <= LINE_WIDTH,
            "and it fits on the paper ({LINE_WIDTH} columns): {line:?}"
        );
    }
}

/// **What `table` carries still prints.** No live producer sends it, but the device contract
/// grows, it is never replaced: an integration that uses it must not lose its table.
#[test]
fn the_legacy_table_field_still_prints() {
    let text = paper(&json!({ "table": "4", "items": [{ "name": "Croquetas", "quantity": 2 }] }));
    assert!(text.contains("Mesa: 4"), "the old shape keeps its prefix:\n{text}");
}

/// **A MENU prints as a header + INDENTED components, never as a paragraph.**
///
/// Indentation is the industry's axis of hierarchy, not size: Simphony documents the
/// components *«indented beneath the combo meal name»* **and says explicitly that it holds in
/// chit mode**, and Aloha makes it a numeric parameter (`QC indentation size`). The textbook
/// failure is the opposite: Square piles the combo into *«one long run-on paragraph, which
/// makes it kind of hard to decipher when in the kitchen»*, and its community has had it open
/// for years with «buy another printer» as the only remedy.
#[test]
fn a_menu_prints_a_header_with_its_components_indented_below() {
    let text = paper(&json!({
        "label": "Mesa 4",
        "items": [
            { "name": "Croquetas", "quantity": 2 },
            { "name": "Gazpacho", "quantity": 1, "combo_ref": "c1", "combo_name": "MENU DEL DIA" },
            { "name": "Entrecot", "quantity": 1, "combo_ref": "c1", "combo_name": "MENU DEL DIA" }
        ],
    }));
    let lines: Vec<&str> = text.lines().collect();
    assert!(
        lines.contains(&"MENU DEL DIA"),
        "the menu header is on the paper, at the far left:\n{text}"
    );
    // Every component on ITS line, indented two columns. A run-on paragraph would be Square's
    // failure; the same column as the croquetas would make them three loose dishes.
    assert!(lines.contains(&"  1x Gazpacho"), "indented component:\n{text}");
    assert!(lines.contains(&"  1x Entrecot"), "indented component:\n{text}");
    assert!(lines.contains(&"2x Croquetas"), "à la carte dishes are NOT indented:\n{text}");
}

/// 🔴 **The menu header does NOT steal the eye from the dish.**
///
/// The rule is «emphasis is for allergens and changes, NEVER for hierarchy», and on paper it
/// means taking the BOLD off the header, not shrinking it: of fifteen products reviewed,
/// **none** prints the combo name in double height and **none** in Font B. Eats365 gives six
/// fields of the chit a size of their own — order number, table, notes… — and the combo name
/// only a visibility switch: when a vendor gives you a size for six things and only «yes/no»
/// for the seventh, it is telling you the seventh is not a kitchen instruction.
///
/// It is asserted on the ESC/POS MODES, not on the text: the hierarchy of the paper lives there,
/// and a test that only read the text could not tell a discreet header from one that shouts.
#[test]
fn the_menu_header_never_steals_the_eye_from_the_dishes() {
    let bytes = render_document(
        DocumentType::KitchenOrder,
        &json!({
            "items": [
                { "name": "Gazpacho", "quantity": 1, "combo_ref": "c1", "combo_name": "MENU DEL DIA" },
                { "name": "Entrecot", "quantity": 1, "combo_ref": "c1", "combo_name": "MENU DEL DIA" }
            ],
        }),
    )
    .unwrap();
    let modes = lines_with_modes(&bytes);
    let cabecera = modes
        .iter()
        .find(|(t, _, _)| t == "MENU DEL DIA")
        .expect("the header is on the paper");
    assert_eq!(
        (cabecera.1, cabecera.2),
        (false, false),
        "the menu header is NOT bold and at normal height: it is context, not work"
    );
    // Positive control, which is what makes the assertion above mean something: the dishes —
    // what really has to be cooked — are emphasised.
    for plato in ["  1x Gazpacho", "  1x Entrecot"] {
        let l = modes.iter().find(|(t, _, _)| t == plato).expect(plato);
        assert_eq!(
            (l.1, l.2),
            (true, true),
            "the dish outranks the header: {plato}"
        );
    }
}

/// **The supplement IS emphasised** — the other half of the same rule. Emphasis is for the
/// CHANGES, and a change is exactly what a supplement is. Toast prints the dish in black and
/// the modifier in RED, Lightspeed has `print sub-items in red` and Revel the same: three out
/// of three highlight the change. On a thermal printer without a two-colour ribbon, bold is the
/// equivalent.
#[test]
fn a_supplement_is_emphasised_because_it_is_a_change_not_a_hierarchy() {
    let bytes = render_document(
        DocumentType::KitchenOrder,
        &json!({ "items": [{ "name": "Entrecot", "quantity": 1, "modifiers": "SIN CEBOLLA", "notes": "mesa con prisa" }] }),
    )
    .unwrap();
    let modes = lines_with_modes(&bytes);
    let sup = modes.iter().find(|(t, _, _)| t.contains("SIN CEBOLLA")).expect("the supplement");
    assert!(sup.1, "the supplement is bold: it is the change that sends the dish back");
    assert!(!sup.2, "but at normal height: the dish still outranks it");
    // The waiter's free note is NOT a change from the menu: it is read, not shouted.
    let nota = modes.iter().find(|(t, _, _)| t.contains("mesa con prisa")).expect("the note");
    assert!(!nota.1, "the free note does not compete with the supplement");
}

/// **The header repeats on EVERY station sheet** (Simphony's option `11 - Send to Combo Parent
/// Order Devices`; TouchBistro does it by default, with no option).
///
/// Each sheet is a document of its own with the lines of ITS role, so it is checked the way it
/// prints: two renders. A grill cook who does not read «MENÚ» does not know their entrecot is
/// paired with a gazpacho, and sends it out whenever it suits them. Toast's orphan mode — the
/// component without its parent's name — is one you opt into, not the default.
#[test]
fn the_menu_header_repeats_on_every_station_sheet() {
    let plancha = paper(&json!({
        "items": [{ "name": "Entrecot", "quantity": 1, "combo_ref": "c1", "combo_name": "MENU DEL DIA" }],
    }));
    let barra = paper(&json!({
        "items": [{ "name": "Tinto", "quantity": 1, "combo_ref": "c1", "combo_name": "MENU DEL DIA" }],
    }));
    for (estacion, hoja) in [("plancha", &plancha), ("barra", &barra)] {
        assert!(
            hoja.lines().any(|l| l == "MENU DEL DIA"),
            "the {estacion} sheet says which menu it belongs to:\n{hoja}"
        );
    }
}

/// **A nameless menu is still grouped by INDENTATION**, with no blank line where the header
/// would go. Indentation is the main mechanism, not the header: it stands on its own, so there
/// is no need to invent a filler «MENÚ».
#[test]
fn a_nameless_menu_still_groups_its_components_by_indentation() {
    let text = paper(&json!({
        "items": [
            { "name": "Gazpacho", "quantity": 1, "combo_ref": "c1", "combo_name": "" },
            { "name": "Entrecot", "quantity": 1, "combo_ref": "c1", "combo_name": "" }
        ],
    }));
    assert!(text.contains("  1x Gazpacho"), "still indented:\n{text}");
    assert!(text.contains("  1x Entrecot"), "still indented:\n{text}");
    // And no empty line sneaks in where the header would go: the first component sits RIGHT
    // UNDER the rule that opens the list. (The neighbour is checked, not «is there any empty
    // line»: the paper cut leaves three at the end and that check would always fire.)
    let lines: Vec<&str> = text.lines().collect();
    let i = lines.iter().position(|l| *l == "  1x Gazpacho").expect("the component");
    assert!(
        lines[i - 1].starts_with('-'),
        "the component follows the rule, with no blank header in between:\n{text}"
    );
}

/// **A component's supplement is indented one level further** (0 → 2 → 5). Two levels at most:
/// at 32 columns, a third leaves the text no room and the printer cuts it.
#[test]
fn a_supplement_of_a_menu_component_indents_one_level_further() {
    let text = paper(&json!({
        "items": [{
            "name": "Entrecot", "quantity": 1, "modifiers": "Al punto",
            "combo_ref": "c1", "combo_name": "MENU DEL DIA",
        }],
    }));
    assert!(text.contains("\n     Al punto\n"), "indented to 5, under its component:\n{text}");
}

/// **The table is the BIGGEST thing on the paper**: double height and double width.
///
/// It is the only field read from the other side of the pass, at speed, to match a dish with
/// where it goes — the dish is read at arm's length, the table from across the room. Toast puts
/// it *«in large, bold font in the top-left corner»* and Eats365 gives it a size of its own next
/// to the order number, the only two fields of the chit that have one.
#[test]
fn the_table_is_the_biggest_thing_on_the_paper() {
    let bytes = render_document(
        DocumentType::KitchenOrder,
        &json!({ "label": "Mesa 4", "items": [{ "name": "Croquetas", "quantity": 2 }] }),
    )
    .unwrap();
    let modes = lines_with_modes(&bytes);
    let mesa = modes.iter().find(|(t, _, _)| t == "Mesa 4").expect("the table");
    assert!(mesa.1 && mesa.2, "the table is bold and double height");
    assert!(
        double_width_of(&bytes, "Mesa 4"),
        "and double width: it is what is read from the other side of the pass"
    );
}

/// **A long label gives up double width rather than being split.** «Recogida Ana Martinez» in
/// double width takes 42 of the paper's 32 columns: the printer cuts it wherever it falls and
/// the table — the field that exists to be read at a glance — ends up in two pieces.
#[test]
fn a_long_label_gives_up_double_width_before_it_wraps() {
    let bytes = render_document(
        DocumentType::KitchenOrder,
        &json!({ "label": "Recogida Ana Martinez", "items": [{ "name": "Cafe", "quantity": 1 }] }),
    )
    .unwrap();
    assert!(
        !double_width_of(&bytes, "Recogida Ana Martinez"),
        "it fits whole, even if at single width"
    );
    let text = strip_escpos(&bytes);
    assert!(text.contains("Recogida Ana Martinez"), "and it is not lost:\n{text}");
}

/// **An à la carte chit prints EXACTLY as it always has.** It is 99 % of the chits: breaking
/// this is breaking the whole kitchen to fix a rare case.
#[test]
fn an_ordinary_order_prints_exactly_as_it_did_before() {
    let text = paper(&json!({
        "receipt_id": "K-9",
        "items": [
            { "name": "Croquetas", "quantity": 2, "notes": "sin gluten" },
            { "name": "Flan", "quantity": 1 }
        ],
    }));
    assert_eq!(
        text,
        "COCINA\n#K-9\n================================\nHora: HH:MM\n\
         --------------------------------\n2x Croquetas\n   >> sin gluten\n1x Flan\n\
         ================================\n\n\n\n\n"
            .replace("HH:MM", &now_hm()),
        "the usual paper, byte for byte"
    );
}

// ── The supplements arrive as a LIST, one line each ─────────────────────────────────────────
//
// `kitchen` composes them into a single string (`modifiers_for_display`, joined with «, »).
// An ARRAY in the same key is truthy but is not a string, so `str_field` hands back `""`: read
// that way, the supplements would vanish from the paper WITHOUT an error. Both shapes must
// print.

/// An array prints ONE indented line per supplement.
#[test]
fn each_supplement_of_the_list_gets_its_own_line() {
    let text = paper(&json!({
        "items": [{
            "name": "Hamburguesa", "quantity": 1,
            "modifiers": ["Extra queso", "Sin cebolla"],
        }],
    }));
    let dish = text.find("Hamburguesa").expect("the dish");
    let first = text.find("Extra queso").expect("the first supplement reaches the paper");
    let second = text.find("Sin cebolla").expect("the second supplement reaches the paper");
    assert!(dish < first && first < second, "in order, under their dish:\n{text}");
    for wanted in ["Extra queso", "Sin cebolla"] {
        let line = text.lines().find(|l| l.contains(wanted)).expect("its own line");
        assert_eq!(
            line.trim(),
            wanted,
            "one supplement per line, nothing else on it:\n{text}"
        );
        assert!(line.starts_with("   "), "indented under its dish: {line:?}\n{text}");
    }
}

/// Objects carry the same list: `sales` and `kitchen` hold rows, not bare strings, so
/// `[{name}]` must print like `["…"]` rather than silently print nothing.
#[test]
fn a_list_of_rows_prints_like_a_list_of_names() {
    let text = paper(&json!({
        "items": [{
            "name": "Hamburguesa", "quantity": 1,
            "modifiers": [{ "name": "Extra queso" }, { "name": "Sin cebolla" }],
        }],
    }));
    assert!(text.contains("Extra queso"), "the row's name reaches the paper:\n{text}");
    assert!(text.contains("Sin cebolla"), "and so does the second one:\n{text}");
}

/// A supplement is a CHANGE, so it keeps the chit's emphasis for changes: the list must not
/// quietly demote it to normal weight.
#[test]
fn a_listed_supplement_is_emphasised_like_a_joined_one() {
    let bytes = render_document(
        DocumentType::KitchenOrder,
        &json!({
            "items": [{ "name": "Hamburguesa", "quantity": 1, "modifiers": ["Extra queso"] }],
        }),
    )
    .expect("a valid kitchen order");
    let line = lines_with_modes(&bytes)
        .into_iter()
        .find(|(t, _, _)| t.contains("Extra queso"))
        .expect("the supplement printed");
    assert!(line.1, "the supplement prints in bold, like the joined one: {line:?}");
}

/// The string `kitchen` sends prints byte for byte as it always has: a fleet still sending it
/// must not see its chits change.
#[test]
fn the_joined_string_still_prints_exactly_as_today() {
    let text = paper(&json!({
        "items": [{
            "name": "Hamburguesa", "quantity": 1,
            "modifiers": "Extra queso, Sin cebolla",
        }],
    }));
    let line = text
        .lines()
        .find(|l| l.contains("Extra queso"))
        .expect("the joined string still prints");
    assert_eq!(
        line.trim(),
        "Extra queso, Sin cebolla",
        "it stays ONE line: splitting on the comma here would guess a boundary that belongs \
         to the module — an option name can carry a comma:\n{text}"
    );
}

/// An empty list is the same as no list — byte for byte. Otherwise a hub whose module sends
/// `[]` gets a blank indented line under every dish.
#[test]
fn an_empty_list_of_supplements_prints_nothing_at_all() {
    let without = json!({ "items": [{ "name": "Cafe", "quantity": 1 }] });
    let empty = json!({ "items": [{ "name": "Cafe", "quantity": 1, "modifiers": [] }] });
    let blanks = json!({ "items": [{ "name": "Cafe", "quantity": 1, "modifiers": ["", "  "] }] });
    let a = render_document(DocumentType::KitchenOrder, &without).expect("valid");
    for (label, doc) in [("an empty list", &empty), ("a list of blanks", &blanks)] {
        let b2 = render_document(DocumentType::KitchenOrder, doc).expect("valid");
        assert_eq!(a, b2, "{label} prints exactly like no list at all");
    }
}

/// A long supplement of the list wraps keeping its indent, like the joined one already does:
/// cut by the printer at 32 columns, the continuation would start at the margin and read as
/// another dish.
#[test]
fn a_long_listed_supplement_wraps_keeping_its_indent() {
    let text = paper(&json!({
        "items": [{
            "name": "Hamburguesa", "quantity": 1,
            "modifiers": ["Sin cebolla y sin pepinillos y con la carne muy hecha por favor"],
        }],
    }));
    let wrapped: Vec<&str> = text
        .lines()
        .skip_while(|l| !l.contains("Sin cebolla"))
        .take_while(|l| !l.starts_with("===="))
        .collect();
    assert!(wrapped.len() > 1, "it wrapped somewhere:\n{text}");
    for l in &wrapped {
        assert!(l.starts_with("   "), "every piece keeps the indent: {l:?}\n{text}");
        assert!(
            l.chars().count() <= LINE_WIDTH,
            "and fits the paper ({LINE_WIDTH} columns): {l:?}"
        );
    }
}

// ── The marks in the header: rush, allergy and copy ─────────────────────────────────────────
//
// The Hub sends `priority` and `allergy` already written in the business's language (or
// empty) and `duplicate: true` on a reprint. A cook reads them under the order number, before
// the first dish.

/// The lines between the order number and the first rule.
fn header_marks(data: &serde_json::Value) -> Vec<String> {
    paper(data)
        .lines()
        .skip_while(|l| !l.starts_with('#'))
        .skip(1)
        .take_while(|l| !l.starts_with("===="))
        .map(str::to_string)
        .collect()
}

fn chit(extra: serde_json::Value) -> serde_json::Value {
    let mut data = json!({
        "order_number": "20261119-0001", "label": "Mesa 4", "locale": "es",
        "items": [{ "name": "Hamburguesa", "quantity": 1.0, "modifiers": ["*** ALERGIA: GLUTEN ***"] }],
    });
    for (key, value) in extra.as_object().expect("an object") {
        data[key] = value.clone();
    }
    data
}

#[test]
fn the_three_marks_print_under_the_number_in_order() {
    let marks = header_marks(&chit(json!({
        "priority": "!! URGENTE !!", "allergy": "!! ALERGIA !!", "duplicate": true,
    })));
    assert_eq!(marks, ["!! URGENTE !!", "!! ALERGIA !!", "DUPLICADO"]);
}

/// The text of `priority` prints as it comes: the Hub already wrote it in the business's language.
#[test]
fn a_rush_round_prints_the_text_the_hub_sends() {
    let marks = header_marks(&chit(json!({ "priority": "!! URGENT !!", "locale": "en" })));
    assert_eq!(marks, ["!! URGENT !!"]);
}

/// `HIGH` is what producers sent before the text came translated; it still means rush, and the
/// mark is printed once, in the header.
#[test]
fn the_legacy_high_priority_still_prints_the_rush_mark_once() {
    let data = chit(json!({ "priority": "HIGH" }));
    assert_eq!(header_marks(&data), ["!! URGENTE !!"]);
    assert_eq!(paper(&data).matches("URGENTE").count(), 1, "{}", paper(&data));
    let english = chit(json!({ "priority": "HIGH", "locale": "en" }));
    assert_eq!(header_marks(&english), ["!! RUSH !!"]);
}

#[test]
fn a_round_that_is_not_urgent_prints_no_rush_mark() {
    for priority in [json!(""), json!("NORMAL"), json!(null)] {
        let data = chit(json!({ "priority": priority }));
        assert!(header_marks(&data).is_empty(), "{priority}: {:?}", header_marks(&data));
        assert!(!paper(&data).contains("NORMAL"), "{priority}:\n{}", paper(&data));
    }
}

#[test]
fn a_sheet_with_an_allergy_says_so_in_the_header() {
    assert_eq!(header_marks(&chit(json!({ "allergy": "!! ALERGIA !!" }))), ["!! ALERGIA !!"]);
}

#[test]
fn a_sheet_without_an_allergy_prints_no_allergy_mark() {
    for allergy in [json!(""), json!(null)] {
        let data = chit(json!({ "allergy": allergy }));
        assert!(header_marks(&data).is_empty(), "{allergy}: {:?}", header_marks(&data));
    }
}

#[test]
fn a_reprint_says_it_is_a_copy() {
    assert_eq!(header_marks(&chit(json!({ "duplicate": true }))), ["DUPLICADO"]);
    let english = chit(json!({ "duplicate": true, "locale": "en" }));
    assert_eq!(header_marks(&english), ["DUPLICATE"]);
}

/// Only an explicit `true` makes the paper a copy, as on the ticket.
#[test]
fn a_first_print_is_not_marked_as_a_copy() {
    for duplicate in [json!(false), json!(null), json!("true"), json!(1)] {
        let data = chit(json!({ "duplicate": duplicate }));
        assert!(header_marks(&data).is_empty(), "{duplicate}: {:?}", header_marks(&data));
    }
}

/// Rush and allergy shout like the order number; the copy mark prints like the ticket's.
#[test]
fn the_marks_print_as_loud_as_what_they_announce() {
    let bytes = render_document(
        DocumentType::KitchenOrder,
        &chit(json!({ "priority": "!! URGENTE !!", "allergy": "!! ALERGIA !!", "duplicate": true })),
    )
    .unwrap();
    let modes = lines_with_modes(&bytes);
    let mode = |text: &str| {
        let (_, bold, double) = modes.iter().find(|(t, _, _)| t == text).expect(text);
        (*bold, *double)
    };
    assert_eq!(mode("!! URGENTE !!"), (true, true));
    assert_eq!(mode("!! ALERGIA !!"), (true, true));
    assert_eq!(mode("DUPLICADO"), (true, false));
    let ticket = render_document(DocumentType::Receipt, &json!({ "duplicate": true, "items": [] })).unwrap();
    let on_ticket = lines_with_modes(&ticket).into_iter().find(|(t, _, _)| t == "DUPLICADO").unwrap();
    assert_eq!((on_ticket.1, on_ticket.2), mode("DUPLICADO"));
}

/// Empty fields print the paper of today, byte for byte.
#[test]
fn a_sheet_with_empty_marks_prints_exactly_as_without_them() {
    let without = paper(&chit(json!({})));
    let empty = paper(&chit(json!({ "priority": "", "allergy": "", "duplicate": false })));
    assert_eq!(empty, without);
}
