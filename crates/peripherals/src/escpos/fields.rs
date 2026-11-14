//! Readers of `data`.

/// `%d/%m/%Y %H:%M` in local time.
pub(super) fn now_dmy_hm() -> String {
    chrono::Local::now().format("%d/%m/%Y %H:%M").to_string()
}

/// `%Y-%m-%d %H:%M:%S` in local time (test page).
pub(super) fn now_ymd_hms() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// `%H:%M` in local time (kitchen order).
pub(super) fn now_hm() -> String {
    chrono::Local::now().format("%H:%M").to_string()
}

/// Reads a string field; missing or not a string → `default`.
pub(super) fn str_field<'a>(data: &'a serde_json::Value, key: &str, default: &'a str) -> &'a str {
    data.get(key).and_then(|v| v.as_str()).unwrap_or(default)
}

/// `true` when the field exists and is truthy the way Python's `if data.get(x):` reads it
/// (a non-empty string, a number other than 0, `true`).
pub(super) fn is_truthy(data: &serde_json::Value, key: &str) -> bool {
    match data.get(key) {
        Some(serde_json::Value::String(s)) => !s.is_empty(),
        Some(serde_json::Value::Number(n)) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::Null) | None => false,
        Some(_) => true,
    }
}

/// Renders a JSON value the way Python's `f"{value}"` does, for the generic document.
pub(super) fn json_to_display(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => {
            // Python prints True/False capitalised.
            if *b { "True".to_string() } else { "False".to_string() }
        }
        serde_json::Value::Null => "None".to_string(),
        other => other.to_string(),
    }
}

/// **The supplements of a line, as a LIST.**
///
/// `modifiers` arrives in two shapes and both have to reach the paper:
///
/// - the **string** `kitchen` composes (`modifiers_for_display`, joined with «, »), which is what
///   every module currently deployed sends;
/// - the **array** — plain strings or rows shaped `{name}` — which is what buys one line per
///   supplement instead of one line that wraps and loses its indent.
///
/// An array is *truthy* but is not a string, so reading it with `str_field` would hand back `""`
/// and the supplements would vanish from the chit **without an error** — the worst possible
/// failure for paper nobody re-reads.
///
/// The joined string stays ONE entry on purpose: splitting it on the comma here would guess a
/// boundary that belongs to the module, and an option name can perfectly well carry a comma.
pub(super) fn modifier_lines(item: &serde_json::Value) -> Vec<String> {
    match item.get("modifiers") {
        Some(serde_json::Value::Array(list)) => list
            .iter()
            .filter_map(|entry| match entry {
                serde_json::Value::String(s) => Some(s.trim()),
                other => other.get("name").and_then(|v| v.as_str()).map(str::trim),
            })
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        Some(serde_json::Value::String(s)) if !s.trim().is_empty() => vec![s.to_string()],
        _ => Vec::new(),
    }
}

/// Formats the quantity the way Python's `f"{qty}x …"` does: a JSON integer (`2`) prints as `2`,
/// not `2.0`, and a decimal (`1.5`) prints as `1.5`.
pub(super) fn fmt_qty(raw: Option<&serde_json::Value>, fallback: f64) -> String {
    match raw {
        Some(serde_json::Value::Number(n)) => {
            if let Some(i) = n.as_i64() {
                i.to_string()
            } else if let Some(u) = n.as_u64() {
                u.to_string()
            } else {
                n.to_string()
            }
        }
        Some(serde_json::Value::String(s)) => s.clone(),
        _ => {
            // No value → the default is the integer 1, as in Python.
            if fallback.fract() == 0.0 {
                format!("{}", fallback as i64)
            } else {
                fallback.to_string()
            }
        }
    }
}
