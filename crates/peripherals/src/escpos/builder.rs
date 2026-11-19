use super::{LINE_WIDTH, QR_MODULE_DOTS};
use icu_normalizer::{ComposingNormalizerBorrowed, DecomposingNormalizerBorrowed};
use oem_cp::{Cp437, StrExt};

/// Horizontal alignment (`ESC a n`).
#[derive(Debug, Clone, Copy)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// Builds an ESC/POS buffer: every method pushes bytes onto it. Text is encoded in cp437.
pub struct EscposBuilder {
    buf: Vec<u8>,
    /// Decimals every amount is printed with: the scale of the document's currency.
    decimals: usize,
}

impl Default for EscposBuilder {
    fn default() -> Self {
        Self {
            buf: Vec::new(),
            decimals: DEFAULT_MONEY_DECIMALS,
        }
    }
}

/// The scale a document without `decimals` is printed in: the euro's.
const DEFAULT_MONEY_DECIMALS: usize = 2;

/// The widest scale ISO 4217 defines (CLF, UYW). A document asking for more is not trusted.
const MAX_MONEY_DECIMALS: u64 = 4;

/// The currency scale a document declares in `decimals`: JPY 0, EUR 2, KWD 3.
///
/// The amounts reach the paper as major-unit floats that the producer (`sales`) already divided by
/// that scale, so the paper only needs to know how many digits to show. Anything that is not a
/// whole number in `0..=4` falls back to two decimals — what a document without the field prints —
/// rather than printing a ticket with nine.
pub(super) fn money_decimals(data: &serde_json::Value) -> usize {
    data.get("decimals")
        .and_then(|v| v.as_u64())
        .filter(|d| *d <= MAX_MONEY_DECIMALS)
        .map_or(DEFAULT_MONEY_DECIMALS, |d| d as usize)
}

impl EscposBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Prints every amount with `decimals` digits (the currency's scale).
    pub fn with_money_decimals(mut self, decimals: usize) -> Self {
        self.decimals = decimals;
        self
    }

    /// An amount as the paper shows it, in the currency's scale: `1500` in yen, `12.50` in euros,
    /// `1.234` in dinars.
    pub fn money(&self, amount: f64) -> String {
        format!("{amount:.prec$}", prec = self.decimals)
    }

    /// Print state: alignment + bold + double height/width (`ESC a`, `ESC E`, `GS !`).
    pub fn set(&mut self, align: Align, bold: bool, double_h: bool, double_w: bool) -> &mut Self {
        // ESC a n  (align: left=0, center=1, right=2)
        let a = match align {
            Align::Left => 0u8,
            Align::Center => 1u8,
            Align::Right => 2u8,
        };
        self.buf.extend_from_slice(&[0x1b, 0x61, a]);
        // ESC E n  (bold)
        self.buf
            .extend_from_slice(&[0x1b, 0x45, if bold { 1 } else { 0 }]);
        // GS ! n  (n: |0x10 double_height, |0x20 double_width)
        let mut n = 0u8;
        if double_h {
            n |= 0x10;
        }
        if double_w {
            n |= 0x20;
        }
        self.buf.extend_from_slice(&[0x1d, 0x21, n]);
        self
    }

    /// Text in cp437, with whatever it cannot represent replaced by `?`. [`to_printable`]
    /// transliterates first: without it the typographic punctuation of a translated text and the
    /// accented capitals cp437 lacks reach the paper as `?`.
    pub fn text(&mut self, txt: &str) -> &mut Self {
        for cp in to_printable(txt).to_cp_lossy::<Cp437>() {
            self.buf.push(cp.0);
        }
        self
    }

    /// Paper cut: `\n\n\n` + `GS V 1`.
    pub fn cut(&mut self) -> &mut Self {
        self.buf.extend_from_slice(b"\n\n\n");
        self.buf.extend_from_slice(&[0x1d, 0x56, 0x01]);
        self
    }

    /// **Native** barcode (`GS k`, function B):
    ///   - 12-13 digits → **EAN13** (m=67).
    ///   - anything else → **CODE128** (m=73) in code set B (`{B` prefix); anything that is not
    ///     printable ASCII → `?`.
    ///
    /// Sets the height (`GS h`), the module width (`GS w`) and the HRI below (`GS H 2`, font A
    /// `GS f 0`) before emitting the symbol. An empty code, or one over the command's limit
    /// (255 bytes), falls back to the text `[code]` so no invalid bytes are sent.
    pub fn barcode(&mut self, code: &str) -> &mut Self {
        if code.is_empty() || code.len() > 250 {
            self.text(&format!("[{code}]\n"));
            return self;
        }

        // GS h n — symbol height (dots).
        self.buf.extend_from_slice(&[0x1d, 0x68, 80]);
        // GS w n — module width.
        self.buf.extend_from_slice(&[0x1d, 0x77, 2]);
        // GS H n — HRI below the symbol (2).
        self.buf.extend_from_slice(&[0x1d, 0x48, 2]);
        // GS f n — font A for the HRI.
        self.buf.extend_from_slice(&[0x1d, 0x66, 0]);

        let all_digits = code.chars().all(|c| c.is_ascii_digit());
        if all_digits && (code.len() == 12 || code.len() == 13) {
            // EAN13, function B: GS k 67 n d1..dn (given 12 digits, the printer adds the checksum).
            self.buf.extend_from_slice(&[0x1d, 0x6b, 67, code.len() as u8]);
            self.buf.extend_from_slice(code.as_bytes());
        } else {
            // CODE128, function B: GS k 73 n {B d1..dk (code set B covers ASCII 32-127).
            let mut payload: Vec<u8> = Vec::with_capacity(code.len() + 2);
            payload.extend_from_slice(b"{B");
            payload.extend(code.bytes().map(|b| if (0x20..0x7f).contains(&b) { b } else { b'?' }));
            self.buf.extend_from_slice(&[0x1d, 0x6b, 73, payload.len() as u8]);
            self.buf.extend_from_slice(&payload);
        }
        self.buf.push(b'\n');
        self
    }

    /// **Native** QR code (`GS ( k`, model 2), in the standard Epson sequence:
    /// fn 165 (model 2) → fn 167 (module size) → fn 169 (error correction M) →
    /// fn 180 (store the data) → fn 181 (print). Empty data, or data over the command's limit
    /// (~7 KB) → no-op.
    pub fn qr(&mut self, data: &str) -> &mut Self {
        self.qr_sized(data, QR_MODULE_DOTS)
    }

    /// [`Self::qr`] with its module size in dots (fn 167): a secondary code prints smaller so the
    /// one the paper is for stays the one the eye goes to.
    pub fn qr_sized(&mut self, data: &str, module_dots: u8) -> &mut Self {
        let bytes = data.as_bytes();
        if bytes.is_empty() || bytes.len() > 7080 {
            return self;
        }

        // GS ( k 4 0 49 65 50 0 — fn 165: select model 2.
        self.buf.extend_from_slice(&[0x1d, 0x28, 0x6b, 4, 0, 49, 65, 50, 0]);
        // GS ( k 3 0 49 67 n — fn 167: module size (dots).
        self.buf.extend_from_slice(&[0x1d, 0x28, 0x6b, 3, 0, 49, 67, module_dots]);
        // GS ( k 3 0 49 69 n — fn 169: error correction level M (49).
        self.buf.extend_from_slice(&[0x1d, 0x28, 0x6b, 3, 0, 49, 69, 49]);
        // GS ( k pL pH 49 80 48 d1..dk — fn 180: store the data (len = k + 3).
        let len = bytes.len() + 3;
        self.buf.extend_from_slice(&[
            0x1d,
            0x28,
            0x6b,
            (len & 0xff) as u8,
            (len >> 8) as u8,
            49,
            80,
            48,
        ]);
        self.buf.extend_from_slice(bytes);
        // GS ( k 3 0 49 81 48 — fn 181: print the stored symbol.
        self.buf.extend_from_slice(&[0x1d, 0x28, 0x6b, 3, 0, 49, 81, 48]);
        self.buf.push(b'\n');
        self
    }

    /// Right-aligned total line: `Label        12.50`.
    pub fn total_line(&mut self, label: &str, amount: f64) -> &mut Self {
        let amount_str = self.money(amount);
        let padding = (LINE_WIDTH as isize - label.len() as isize - amount_str.len() as isize).max(1)
            as usize;
        self.text(&format!("{label}{}{amount_str}\n", " ".repeat(padding)));
        self
    }

    pub fn finish(self) -> Vec<u8> {
        self.buf
    }
}

/// **Typographic punctuation and accented letters → what a cp437 printer can actually produce.**
///
/// The encoder replaces whatever cp437 cannot represent with `?`, so the translated bill notice
/// («Cuenta — no es una factura», with the em dash a translator naturally types) would reach the
/// paper as «Cuenta ? no es una factura». Curly quotes, ellipses and non-breaking spaces do the
/// same, and they come from people writing in a text editor — not a habit anybody is going to stop
/// having, so this is the layer that copes.
///
/// cp437 has á é í ó ú ñ Ñ É Ü but not Á Í Ó Ú (nor À, Ã, Ê, õ…): a letter it lacks prints without
/// its accent, «CRUSTÁCEOS» as «CRUSTACEOS», rather than as «CRUST?CEOS». An accent that arrives as
/// a separate combining character is first joined to its letter, so it prints the same.
pub(super) fn to_printable(txt: &str) -> String {
    let mut out = String::with_capacity(txt.len());
    for c in ComposingNormalizerBorrowed::new_nfc().normalize_iter(txt.chars()) {
        match c {
            // Hyphen..horizontal bar (en/em dash included) and the minus sign.
            '\u{2010}'..='\u{2015}' | '\u{2212}' => out.push('-'),
            '\u{2018}'..='\u{201B}' | '\u{2032}' => out.push('\''),
            '\u{201C}'..='\u{201F}' | '\u{2033}' => out.push('"'),
            '\u{2026}' => out.push_str("..."),
            // Every space that is not the plain one; a printer column does not care which it was.
            '\u{00A0}' | '\u{2007}' | '\u{2009}' | '\u{202F}' => out.push(' '),
            // cp437 predates the euro. A footer saying `EUR` beats one saying `?`.
            '\u{20AC}' => out.push_str("EUR"),
            // Only letters lose their accent: «≠» is «=» plus a stroke, and must not print as «=».
            other if other.is_alphabetic() && Cp437::try_from(other).is_err() => {
                push_without_accent(&mut out, other);
            }
            other => out.push(other),
        }
    }
    out
}

/// What has no printable letter inside (a symbol, an ideogram) is left for the encoder's `?`.
fn push_without_accent(out: &mut String, c: char) {
    let base: String = DecomposingNormalizerBorrowed::new_nfd()
        .normalize_iter(std::iter::once(c))
        .filter(|part| Cp437::try_from(*part).is_ok())
        .collect();
    if base.is_empty() {
        out.push(c);
    } else {
        out.push_str(&base);
    }
}

/// Splits a text into lines of at most `width` columns, **by words**.
///
/// The printer cuts by column without looking where: without this, the bill's notice would come
/// out split in the middle of a word. A word longer than the width is left as it is — splitting it
/// would make it unreadable, and it is better for the printer to fold it.
///
/// It measures the text as it reaches the paper ([`to_printable`]): `€` prints as `EUR`, three
/// columns for one character, and counting it as one would let a line overflow by two.
pub(super) fn wrap_to_width(txt: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in to_printable(txt).split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
        } else if current.chars().count() + 1 + word.chars().count() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Prints prose the business or the producer writes (a header, a footer, an address, a note)
/// wrapped by words to the paper, keeping the line breaks it already has.
///
/// Printed as one line, the printer folds it at its own column count without looking where, and
/// the customer gets words cut in two on a 58 mm roll and on an 80 mm one alike.
pub(super) fn print_wrapped(b: &mut EscposBuilder, txt: &str) {
    for own_line in txt.lines() {
        let wrapped = wrap_to_width(own_line, LINE_WIDTH);
        if wrapped.is_empty() {
            b.text("\n");
        }
        for line in wrapped {
            b.text(&format!("{line}\n"));
        }
    }
}
