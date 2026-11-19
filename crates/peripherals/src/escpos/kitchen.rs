use super::builder::{wrap_to_width, Align, EscposBuilder};
use super::fields::{fmt_qty, is_truthy, modifier_lines, now_hm, str_field};
use super::locale::{Label, Locale};
use super::receipt::print_duplicate_mark;
use super::LINE_WIDTH;

pub(super) fn render_kitchen_order(b: &mut EscposBuilder, data: &serde_json::Value) {
    let t = Locale::from_document(data);
    b.set(Align::Center, true, true, true);
    b.text(&format!("{}\n", t.label(Label::KitchenTitle)));

    b.set(Align::Center, true, true, false);
    let order_number = data
        .get("receipt_id")
        .and_then(|v| v.as_str())
        .or_else(|| data.get("order_number").and_then(|v| v.as_str()))
        .unwrap_or("");
    b.text(&format!("#{order_number}\n"));

    // The cook reads these before the first dish; rush and allergy keep the number's loud mode.
    if let Some(rush) = rush_mark(data, t) {
        b.text(&format!("{rush}\n"));
    }
    let allergy = str_field(data, "allergy", "");
    if !allergy.is_empty() {
        b.text(&format!("{allergy}\n"));
    }
    print_duplicate_mark(b, data, t);

    b.set(Align::Center, false, false, false);
    b.text("================================\n");

    // **The room label.** The shell sends `label`; reading only `table`, a field no producer
    // sends, would print a chit that does not say which table it is for — useless paper at rush
    // hour.
    //
    // `label` prints **as it comes, with no prefix**: it is opaque on purpose — «Mesa 4»,
    // «Barra», «Recogida Ana». The kitchen does not know what a table is, nor does it need to;
    // prefixing «Mesa: » would print «Mesa: Recogida Ana».
    //
    // `table` stays behind it, with its usual prefix: the device contract grows, it is never
    // replaced. No producer sends it today, but an integration that does must not lose its table.
    // Double height AND double width: the table is the biggest thing on the paper on purpose. It is
    // the only field read from the OTHER SIDE of the pass, at speed, to match a dish with where it
    // goes — the dish is read at arm's length. Toast puts it «in large, bold font in the top-left
    // corner» and Eats365 gives it a size of its own next to the order number, the only two fields
    // of the chit that have one. Double width is dropped when it does not fit: a label the printer
    // splits in two is the opposite of a field that exists to be read at a glance.
    if is_truthy(data, "label") {
        let label = str_field(data, "label", "");
        let wide = label.chars().count() * 2 <= LINE_WIDTH;
        b.set(Align::Left, true, true, wide);
        b.text(&format!("{label}\n"));
    } else if is_truthy(data, "table") {
        b.set(Align::Left, true, true, false);
        b.text(&format!("{}{}\n", t.label(Label::Table), str_field(data, "table", "")));
    }

    b.set(Align::Left, false, false, false);
    if is_truthy(data, "waiter") {
        b.text(&format!("{}{}\n", t.label(Label::Waiter), str_field(data, "waiter", "")));
    }

    // The round tells the second course from the first at the same table. Only from round two on:
    // «Ronda 1» would be a line of noise on 99 % of the paper, which is exactly the paper nobody
    // re-reads.
    let round = data
        .get("round_number")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0);
    if round > 1.0 {
        b.text(&format!(
            "{} {}\n",
            t.label(Label::Round),
            fmt_qty(data.get("round_number"), round)
        ));
    }

    b.text(&format!("{}{}\n", t.label(Label::Time), now_hm()));
    b.text("--------------------------------\n");

    if let Some(items) = data.get("items").and_then(|v| v.as_array()) {
        // The MENU is printed as a RUN of consecutive lines with the same `combo_ref`, like
        // `groupCombos()` in the KDS. The lines arrive sorted by `line_seq`, so the run IS the
        // group.
        let mut combo: Option<&str> = None;
        for item in items {
            let this_combo = item
                .get("combo_ref")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty());

            if this_combo != combo {
                combo = this_combo;
                // **The menu header is NOT bold and at normal height.** Emphasis is for the
                // CHANGES — allergens and supplements — never for hierarchy: a bold menu steals the
                // eye from what has to be cooked. Of fifteen POS products reviewed, NONE prints the
                // combo name in double height and NONE in Font B; the hierarchy of the chit is made
                // with INDENTATION (Simphony: «indented beneath the combo meal name», and it says
                // so for chit mode; Aloha has it as a parameter, `QC indentation size`).
                //
                // It is printed only when the menu has a name: without one the indent groups the
                // lines just as well, and there is no need to invent a filler.
                if this_combo.is_some() && is_truthy(item, "combo_name") {
                    b.set(Align::Left, false, false, false);
                    b.text(&format!("{}\n", str_field(item, "combo_name", "")));
                }
            }

            // Two levels at most: at 32 columns a third would leave the text no room.
            let indent = if combo.is_some() { "  " } else { "" };
            let qty = item.get("quantity").and_then(|v| v.as_f64()).unwrap_or(1.0);
            let name = str_field(item, "name", "");

            b.set(Align::Left, true, true, false);
            b.text(&format!(
                "{indent}{}x {name}\n",
                fmt_qty(item.get("quantity"), qty)
            ));

            // **The supplements.** They go BEFORE the free note and with the same indent: the
            // customer chose the supplement from the menu and it changes the dish; the note is the
            // waiter's text. They are wrapped by hand keeping the indent — if the thermal printer
            // cut them at 32 columns, the continuation would start at the margin and read as one
            // more dish of the order.
            //
            // And IN BOLD: emphasis is the other half of the same rule — the supplement is a
            // CHANGE, and a change is what sends a dish back. Toast prints the dish in black and the
            // modifier in red, Lightspeed has `print sub-items in red`, Revel the same: three out
            // of three highlight the change. On a thermal printer without a two-colour ribbon, bold
            // is the equivalent.
            let sub = format!("{indent}   ");
            let supplements = modifier_lines(item);
            if !supplements.is_empty() {
                b.set(Align::Left, true, false, false);
                for supplement in &supplements {
                    for line in wrap_to_width(supplement, LINE_WIDTH - sub.len()) {
                        b.text(&format!("{sub}{line}\n"));
                    }
                }
            }

            // The waiter's free note is NOT a change from the menu: it is read, not shouted.
            if is_truthy(item, "notes") {
                b.set(Align::Left, false, false, false);
                b.text(&format!("{sub}>> {}\n", str_field(item, "notes", "")));
            }
        }
    }

    b.text("================================\n");
    b.text("\n");
    b.cut();
}

/// The Hub sends the mark already written in the business's language; `HIGH` is the word of the
/// older contract and still means rush.
fn rush_mark(data: &serde_json::Value, t: Locale) -> Option<&str> {
    match str_field(data, "priority", "") {
        "HIGH" => Some(t.label(Label::Rush)),
        "" | "NORMAL" => None,
        text => Some(text),
    }
}
