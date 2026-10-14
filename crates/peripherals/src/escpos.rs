//! Render ESC/POS: convierte un `document_type` + `data` (JSON del Hub) en bytes para enviar
//! por el socket TCP de la impresora.
//!
//! Porta:
//!   - los comandos crudos de `_RawNetworkPrinter` (`discovery.py`): align/bold/size/cut.
//!   - los renderizadores de documento de `printer.py` (`_print_receipt`, `_print_kitchen_order`,
//!     `_print_invoice`, `_print_delivery_note`, `_print_barcode_label`, `_print_cash_report`,
//!     `_print_generic`) y `test_print`.

use crate::Result;
use oem_cp::{Cp437, StrExt};
use serde::{Deserialize, Serialize};

/// Ancho de papel estándar en columnas (80mm ≈ 32 chars). Espejo del `padding = 32 - …` de Python.
pub const LINE_WIDTH: usize = 32;

/// **The name this crate signs paper with when the document does not bring the business's own.**
///
/// A constant and not a literal per renderer because it is the name of a PRODUCT, and products get
/// retired: the test page spent a month signed «ERPlora Bridge» after ADR-0196 retired the daemon
/// and hub#340 deleted it from the tree (hub#1735), while the receipt beside it already fell back
/// to the right one. Paper cannot be corrected after it is cut, so the two must not be able to
/// drift apart again.
pub const PRODUCT_NAME: &str = "ERPlora";

/// Module size, in dots, of a QR on the paper (fiscal and «pide tu factura»).
const QR_MODULE_DOTS: u8 = 4;

/// The promotional QR (hub#2009) at 3 dots — 75 % of the fiscal one, the nearest whole dot to the
/// 70 % `<ok-receipt>` draws on screen.
const PROMO_QR_MODULE_DOTS: u8 = 3;

/// Tipos de documento soportados (espejo del `if document_type == …` de `PrinterManager`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentType {
    Receipt,
    KitchenOrder,
    Invoice,
    DeliveryNote,
    BarcodeLabel,
    CashSessionReport,
    /// **The bill taken to the table before charging** (ADR-0141, hub#748). Deliberately a document
    /// of its own and not a `Receipt`: it is the same list of lines, but it must NOT carry a series
    /// number, a payment method or a VeriFactu QR — the numbering is consumed on payment and there
    /// is no billing record yet — and it must carry the printed notice that it is not an invoice.
    /// Handing a customer paper that passes for an invoice is a legal problem, not a cosmetic one.
    Prebill,
    Generic,
}

impl DocumentType {
    /// Maps the protocol string, **refusing what it does not know** (hub#501). The only door.
    ///
    /// It used to have a lenient twin (`from_wire`, unknown → `Generic`, mirroring Python's
    /// `else`), and that leniency is the most expensive kind of failure: a `Kitchen` or a `kitchn`
    /// rendered as `Generic` — a nameless dump of key/value pairs — so the kitchen got a piece of
    /// paper that was not an order and **nobody found out until the plate was missing**. A ticket
    /// that fails loudly is cheaper than one that fails quietly, so the caller gets a `None` it has
    /// to deal with. The twin's only caller was the standalone Bridge, deleted by hub#340; hub#578
    /// removed the door with it, so no route can degrade to `Generic` in silence any more.
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

/// Alineación horizontal (ESC a n).
#[derive(Debug, Clone, Copy)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// Constructor de un buffer ESC/POS. Cada método empuja bytes al buffer interno.
/// Porta `_RawNetworkPrinter.{set,text,cut,barcode}` (codificación cp437).
pub struct EscposBuilder {
    buf: Vec<u8>,
    /// Decimals every amount is printed with: the scale of the document's currency (hub#2129).
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

/// The scale a document without `decimals` is printed in: the euro's, which is what every paper
/// printed before hub#2129.
const DEFAULT_MONEY_DECIMALS: usize = 2;

/// The widest scale ISO 4217 defines (CLF, UYW). A document asking for more is not trusted.
const MAX_MONEY_DECIMALS: u64 = 4;

/// The currency scale a document declares in `decimals` (hub#2129): JPY 0, EUR 2, KWD 3.
///
/// The amounts reach the paper as major-unit floats that the producer (`sales`) already divided by
/// that scale, so the paper only needs to know how many digits to show. Anything that is not a
/// whole number in `0..=4` falls back to two decimals — the paper of every document that predates
/// the field — rather than printing a ticket with nine.
fn money_decimals(data: &serde_json::Value) -> usize {
    data.get("decimals")
        .and_then(|v| v.as_u64())
        .filter(|d| *d <= MAX_MONEY_DECIMALS)
        .map_or(DEFAULT_MONEY_DECIMALS, |d| d as usize)
}

impl EscposBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Prints every amount with `decimals` digits (the currency's scale, hub#2129).
    pub fn with_money_decimals(mut self, decimals: usize) -> Self {
        self.decimals = decimals;
        self
    }

    /// An amount as the paper shows it, in the currency's scale: `1500` in yen, `12.50` in euros,
    /// `1.234` in dinars.
    pub fn money(&self, amount: f64) -> String {
        format!("{amount:.prec$}", prec = self.decimals)
    }

    /// Estado de impresión: alineación + negrita + doble alto/ancho (`ESC a`, `ESC E`, `GS !`).
    /// Espejo exacto de `_RawNetworkPrinter.set`.
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

    /// Texto (cp437, reemplazando lo no representable por '?'). Porta `_RawNetworkPrinter.text`
    /// (`txt.encode('cp437', errors='replace')`), con la transliteración de [`to_printable`]
    /// delante: sin ella la puntuación tipográfica de un texto traducido llega al papel como `?`.
    pub fn text(&mut self, txt: &str) -> &mut Self {
        for cp in to_printable(txt).to_cp_lossy::<Cp437>() {
            self.buf.push(cp.0);
        }
        self
    }

    /// Corte de papel (`GS V`). Porta `_RawNetworkPrinter.cut`: `\n\n\n` + GS V 1.
    pub fn cut(&mut self) -> &mut Self {
        self.buf.extend_from_slice(b"\n\n\n");
        self.buf.extend_from_slice(&[0x1d, 0x56, 0x01]);
        self
    }

    /// Código de barras **nativo** (`GS k`, función B). Sustituye al antiguo fallback de texto
    /// `[code]` del Bridge Python (bridge#1):
    ///   - 12-13 dígitos → **EAN13** (m=67).
    ///   - resto → **CODE128** (m=73) en code set B (prefijo `{B`); lo no-ASCII-imprimible → `?`.
    ///
    /// Configura altura (`GS h`), ancho de módulo (`GS w`) y HRI debajo (`GS H 2`, fuente A
    /// `GS f 0`) antes de emitir el símbolo. Si el código está vacío o excede el límite del
    /// comando (255 bytes) cae al fallback de texto `[code]` para no enviar bytes inválidos.
    pub fn barcode(&mut self, code: &str) -> &mut Self {
        if code.is_empty() || code.len() > 250 {
            self.text(&format!("[{code}]\n"));
            return self;
        }

        // GS h n — altura del símbolo (puntos).
        self.buf.extend_from_slice(&[0x1d, 0x68, 80]);
        // GS w n — ancho de módulo.
        self.buf.extend_from_slice(&[0x1d, 0x77, 2]);
        // GS H n — HRI debajo del símbolo (2).
        self.buf.extend_from_slice(&[0x1d, 0x48, 2]);
        // GS f n — fuente A para el HRI.
        self.buf.extend_from_slice(&[0x1d, 0x66, 0]);

        let all_digits = code.chars().all(|c| c.is_ascii_digit());
        if all_digits && (code.len() == 12 || code.len() == 13) {
            // EAN13, función B: GS k 67 n d1..dn (con 12 dígitos la impresora calcula el checksum).
            self.buf.extend_from_slice(&[0x1d, 0x6b, 67, code.len() as u8]);
            self.buf.extend_from_slice(code.as_bytes());
        } else {
            // CODE128, función B: GS k 73 n {B d1..dk (code set B cubre ASCII 32-127).
            let mut payload: Vec<u8> = Vec::with_capacity(code.len() + 2);
            payload.extend_from_slice(b"{B");
            payload.extend(code.bytes().map(|b| if (0x20..0x7f).contains(&b) { b } else { b'?' }));
            self.buf.extend_from_slice(&[0x1d, 0x6b, 73, payload.len() as u8]);
            self.buf.extend_from_slice(&payload);
        }
        self.buf.push(b'\n');
        self
    }

    /// Código QR **nativo** (`GS ( k`, modelo 2) — bridge#1. Secuencia estándar Epson:
    /// fn 165 (modelo 2) → fn 167 (tamaño de módulo) → fn 169 (corrección de errores M) →
    /// fn 180 (almacenar datos) → fn 181 (imprimir). Datos vacíos o > límite del comando
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

        // GS ( k 4 0 49 65 50 0 — fn 165: seleccionar modelo 2.
        self.buf.extend_from_slice(&[0x1d, 0x28, 0x6b, 4, 0, 49, 65, 50, 0]);
        // GS ( k 3 0 49 67 n — fn 167: tamaño de módulo (puntos).
        self.buf.extend_from_slice(&[0x1d, 0x28, 0x6b, 3, 0, 49, 67, module_dots]);
        // GS ( k 3 0 49 69 n — fn 169: nivel de corrección M (49).
        self.buf.extend_from_slice(&[0x1d, 0x28, 0x6b, 3, 0, 49, 69, 49]);
        // GS ( k pL pH 49 80 48 d1..dk — fn 180: almacenar los datos (len = k + 3).
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
        // GS ( k 3 0 49 81 48 — fn 181: imprimir el símbolo almacenado.
        self.buf.extend_from_slice(&[0x1d, 0x28, 0x6b, 3, 0, 49, 81, 48]);
        self.buf.push(b'\n');
        self
    }

    /// Línea de total alineada a la derecha: `Etiqueta        12.50`. Porta `_print_total_line`.
    pub fn total_line(&mut self, label: &str, amount: f64) -> &mut Self {
        let amount_str = self.money(amount);
        let padding = (LINE_WIDTH as isize - label.len() as isize - amount_str.len() as isize).max(1)
            as usize;
        self.text(&format!("{label}{}{amount_str}\n", " ".repeat(padding)));
        self
    }

    /// Devuelve los bytes acumulados.
    pub fn finish(self) -> Vec<u8> {
        self.buf
    }
}

/// Main render: document → ESC/POS bytes ready for the socket.
/// Dispatches on `DocumentType` like `PrinterManager.print_document`.
///
/// **A `data` that is not an object is refused** (hub#501). Every renderer below reads its fields
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
        // hub#2005 — the same body as the ticket (lines, totals, QR, duplicate mark) plus what
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

/// Página de prueba. Porta `PrinterManager.test_print`.
///
/// **Signed with [`PRODUCT_NAME`], never with the product that printed it a month ago** (hub#1735):
/// the header used to read «ERPlora Bridge», the standalone daemon ADR-0196 retired and hub#340
/// deleted from the tree.
///
/// **And it is paper like the rest of the paper** (hub#1803). This sheet used to be rendered from
/// a printer id alone, so it was the only document in this file with no envelope: its two lines
/// were English literals and its header was the product's name, while the ticket printed a second
/// later came out in the hub's language under the shop's own name. It now takes the same `data`
/// every other renderer takes and reads the same two fields off it:
///
/// - `locale` → [`Locale::from_document`], the catalogue of this file (hub#1159). English is the
///   SOURCE language, never the paper's (ADR-0055/0199).
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

// ─── Helpers de lectura de `data` (espejo de los `.get(...)` de Python) ──────

/// `%d/%m/%Y %H:%M` en hora local (como `datetime.now().strftime` de Python).
fn now_dmy_hm() -> String {
    chrono::Local::now().format("%d/%m/%Y %H:%M").to_string()
}

/// `%Y-%m-%d %H:%M:%S` en hora local (test page).
fn now_ymd_hms() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// `%H:%M` en hora local (kitchen order).
fn now_hm() -> String {
    chrono::Local::now().format("%H:%M").to_string()
}

/// Lee un campo string; ausente/no-string → `default`.
fn str_field<'a>(data: &'a serde_json::Value, key: &str, default: &'a str) -> &'a str {
    data.get(key).and_then(|v| v.as_str()).unwrap_or(default)
}

/// `true` si el campo existe y es "truthy" como el `if data.get(x):` de Python
/// (string no vacío, número != 0, bool true).
fn is_truthy(data: &serde_json::Value, key: &str) -> bool {
    match data.get(key) {
        Some(serde_json::Value::String(s)) => !s.is_empty(),
        Some(serde_json::Value::Number(n)) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::Null) | None => false,
        Some(_) => true,
    }
}

/// Renderiza un valor JSON tal como lo haría `f"{value}"` de Python para el documento genérico.
fn json_to_display(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => {
            // Python imprime True/False con mayúscula inicial.
            if *b { "True".to_string() } else { "False".to_string() }
        }
        serde_json::Value::Null => "None".to_string(),
        other => other.to_string(),
    }
}

// ─── The paper's language (hub#1159 · ADR-0055/0199) ─────────────────────────

/// Which key of the catalogue below; the values live in [`Locale::label`].
///
/// An enum rather than free strings so the compiler, not a reviewer, is what guarantees every
/// label has both columns filled in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Label {
    // Ticket / invoice.
    VatNumber,
    Phone,
    Ticket,
    Date,
    Cashier,
    Customer,
    Subtotal,
    Tax,
    Discount,
    Total,
    Payment,
    Tendered,
    Change,
    Thanks,
    Duplicate,
    // The full invoice (hub#2005).
    InvoiceTitle,
    RectifyingInvoiceTitle,
    InvoiceNumber,
    BreakdownRate,
    BreakdownBase,
    BreakdownTax,
    // The bill taken to the table.
    BillTitle,
    TableOrCustomer,
    BillNotice,
    // The kitchen chit.
    KitchenTitle,
    Table,
    Waiter,
    Round,
    Time,
    Rush,
    // The delivery note.
    DeliveryNoteTitle,
    Number,
    Address,
    Signature,
    // The cash-up report.
    CashReportTitle,
    Session,
    Opening,
    Closing,
    Difference,
    // Anything else.
    GenericTitle,
    // The test sheet (hub#1803).
    TestPageOk,
    TestPagePrinter,
}

/// **The language the paper is printed in** (hub#1159).
///
/// Every label of this file used to be wired in Spanish: the crate was ported from `printer.py`
/// and never had a translator. A hub running in another language therefore painted a translated
/// KDS on screen and pushed a *Spanish* chit out of the thermal printer — and the chit and the
/// ticket are precisely the surface the CUSTOMER and the KITCHEN read, not an admin screen.
///
/// **The catalogue lives here, and the language travels in the document** (`locale`). The
/// alternative considered in the issue — each producer sending its ~30 labels already translated
/// inside `data` — spreads one catalogue over N module repos, turns adding a label into an N-repo
/// change, and buys a half-Spanish ticket from every module that forgets one. Odoo does it this
/// way too: the report gets a `lang` and the template's own catalogue resolves the strings.
///
/// 🔴 **English is the source and `es` is a translation** (ADR-0055/0199), but the FALLBACK is
/// `es`: a document without `locale` — which is every `erplora-app` deployed today — must print
/// the paper of today, byte for byte. The device contract grows; it never moves under a fleet
/// that cannot be updated as fast as a module can.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Locale {
    En,
    Es,
}

impl Locale {
    /// Reads `locale` off the document.
    ///
    /// A language this printer does not carry falls back to the paper of today rather than
    /// refusing the job or printing an empty label: paper that does not come out loses a service,
    /// and a chit in the wrong language still gets the burger cooked. The region is dropped —
    /// `es-ES` and `en_GB` are the language before the separator.
    fn from_document(data: &serde_json::Value) -> Self {
        match data
            .get("locale")
            .and_then(|v| v.as_str())
            .map(|s| {
                s.split(['-', '_'])
                    .next()
                    .unwrap_or_default()
                    .to_ascii_lowercase()
            })
            .as_deref()
        {
            Some("en") => Locale::En,
            _ => Locale::Es,
        }
    }

    /// The catalogue, one `(en, es)` pair per key.
    ///
    /// The `es` column is, literal for literal, what this file printed before hub#1159 — the
    /// characterisation test holds it there, because moving one of these strings moves the paper
    /// in every kitchen of the fleet at once.
    fn label(self, key: Label) -> &'static str {
        let (en, es) = match key {
            Label::VatNumber => ("VAT: ", "NIF: "),
            Label::Phone => ("Tel: ", "Tel: "),
            Label::Ticket => ("Ticket: ", "Ticket: "),
            Label::Date => ("Date: ", "Fecha: "),
            Label::Cashier => ("Cashier: ", "Cajero: "),
            Label::Customer => ("Customer: ", "Cliente: "),
            Label::Subtotal => ("Subtotal", "Subtotal"),
            Label::Tax => ("VAT", "IVA"),
            Label::Discount => ("Discount", "Descuento"),
            Label::Total => ("TOTAL", "TOTAL"),
            Label::Payment => ("Payment: ", "Pago: "),
            Label::Tendered => ("Tendered", "Entregado"),
            Label::Change => ("Change", "Cambio"),
            Label::Thanks => ("Thank you for your purchase", "Gracias por su compra"),
            // RD 1619/2012 art. 14.4: every copy after the original says «duplicado».
            Label::Duplicate => ("DUPLICATE", "DUPLICADO"),
            Label::InvoiceTitle => ("INVOICE", "FACTURA"),
            // hub#2381 — what the A4, Holded, Odoo and Business Central title an R1–R4.
            Label::RectifyingInvoiceTitle => ("CORRECTIVE INVOICE", "FACTURA RECTIFICATIVA"),
            Label::InvoiceNumber => ("Invoice: ", "Factura: "),
            // The three columns of the VAT breakdown; «Cuota» is what a Spanish invoice calls the
            // tax amount of a rate.
            Label::BreakdownRate => ("Rate", "Tipo"),
            Label::BreakdownBase => ("Base", "Base"),
            Label::BreakdownTax => ("Tax", "Cuota"),
            Label::BillTitle => ("BILL", "CUENTA"),
            Label::TableOrCustomer => ("Table/Customer: ", "Mesa/Cliente: "),
            // The notice is what keeps this paper from passing for an invoice, so it has a
            // default (a producer that forgets it must not print something that looks fiscal) —
            // and the default has to be in the customer's language too.
            Label::BillNotice => (
                "Bill - not an invoice. The fiscal receipt is handed over on payment.",
                "Cuenta - no es una factura. El tiquet fiscal se entrega al cobrar.",
            ),
            Label::KitchenTitle => ("KITCHEN", "COCINA"),
            Label::Table => ("Table: ", "Mesa: "),
            Label::Waiter => ("Waiter: ", "Camarero: "),
            Label::Round => ("Round", "Ronda"),
            Label::Time => ("Time: ", "Hora: "),
            // «RUSH» is what an English-speaking kitchen has printed on its chits for decades;
            // a literal «URGENT» is a translation of the word, not of the paper.
            Label::Rush => ("!! RUSH !!", "!! URGENTE !!"),
            Label::DeliveryNoteTitle => ("DELIVERY NOTE", "ALBARAN"),
            Label::Number => ("No: ", "N: "),
            Label::Address => ("Addr: ", "Dir: "),
            Label::Signature => ("Signature: ___________", "Firma: _______________"),
            Label::CashReportTitle => ("CASH REPORT", "CIERRE DE CAJA"),
            Label::Session => ("Session: ", "Sesion: "),
            Label::Opening => ("Opening", "Apertura"),
            Label::Closing => ("Closing", "Cierre"),
            Label::Difference => ("Difference", "Diferencia"),
            Label::GenericTitle => ("Document", "Documento"),
            // The two lines of the test sheet (hub#1803). Unaccented like every other `es` entry
            // above: the column is ASCII from end to end, which is also what lets the tests read
            // the paper back (`strip_escpos` decodes the cp437 bytes as UTF-8).
            Label::TestPageOk => ("Test Print OK", "Prueba de impresion correcta"),
            Label::TestPagePrinter => ("Printer: ", "Impresora: "),
        };
        match self {
            Locale::En => en,
            Locale::Es => es,
        }
    }
}

/// **The supplements of a line, as a LIST** (hub#1138).
///
/// `modifiers` arrives in two shapes and both have to reach the paper:
///
/// - the **string** `kitchen` composes today (`modifiers_for_display`, joined with «, »), which is
///   what every module currently deployed sends;
/// - the **array** this issue asks for — plain strings or rows shaped `{name}` — which is what
///   buys one line per supplement instead of one line that wraps and loses its indent.
///
/// Before this, an array was *truthy* but was not a string, so `str_field` handed back `""` and
/// the supplements vanished from the chit **without an error** — the silent drop sales#78 warns
/// about, and the worst possible failure for paper nobody re-reads.
///
/// The joined string stays ONE entry on purpose: splitting it on the comma here would guess a
/// boundary that belongs to the module, and an option name can perfectly well carry a comma.
fn modifier_lines(item: &serde_json::Value) -> Vec<String> {
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

// ─── Renderizadores de documento (porta `printer.py`) ────────────────────────

/// Which fiscal paper [`render_receipt`] cuts: the ticket (simplified invoice) or the full invoice.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fiscal {
    Ticket,
    FullInvoice,
}

/// Width of the rate column of the VAT breakdown; the base and the quota take 12 columns each, so
/// a row fills the 32 of the roll exactly.
const BREAKDOWN_RATE_WIDTH: usize = LINE_WIDTH - 2 * BREAKDOWN_AMOUNT_WIDTH;
const BREAKDOWN_AMOUNT_WIDTH: usize = 12;

/// **What a `invoice` must carry to be printed as a FULL invoice on the roll** (hub#2005).
///
/// `invoice` used to fall into the ticket's body, which reads neither the customer's tax id nor a
/// per-rate breakdown: a customer who asked for their invoice took home a ticket, and nothing said
/// so. Spanish tills (Ágora, Revo, Glop) print the full invoice on the 80 mm roll, and it is valid
/// as long as it carries every datum RD 1619/2012 art. 6 asks for. So the document either brings
/// them or is REFUSED here, naming the field — the print host reports the refusal instead of
/// cutting a mute ticket. The contract (field → paper):
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
fn check_full_invoice(data: &serde_json::Value) -> Result<()> {
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

/// Porta `_print_receipt`; with [`Fiscal::FullInvoice`] it is also the full invoice (hub#2005).
fn render_receipt(b: &mut EscposBuilder, data: &serde_json::Value, kind: Fiscal) {
    let t = Locale::from_document(data);

    // The fiscal QR (VeriFactu, `qr_data`) OPENS the ticket (sales#339): the AEAT's QR
    // specification (v0.5.0 §3, per Orden HAC/1177/2024 art. 21.1) places it «at the beginning of
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
        // sales#327 — the legal legend of the fiscal QR («VERI*FACTU», RD 1619/2012 art. 6.5.b),
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

    // hub#1931 — only one original of an invoice may exist (RD 1619/2012 art. 14): a reprint is a
    // duplicate and has to say so. Only an explicit `true` prints it, because the word is a legal
    // statement about the paper, not decoration a truthy string should switch on.
    if data.get("duplicate").and_then(|v| v.as_bool()) == Some(true) {
        b.set(Align::Center, true, false, false);
        b.text(&format!("{}\n", t.label(Label::Duplicate)));
    }

    b.text("================================\n");

    // hub#2005 — the paper says what it is, before its data: the cheapest way to tell a full
    // invoice from a ticket at a glance.
    if kind == Fiscal::FullInvoice {
        // hub#2381 — a corrective invoice says so in its title. Only an explicit `true`, like
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

    // hub#2005 — the customer IDENTIFIED, right under their name. Only on the full invoice: the
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

            // hub#1165 / ADR-0396 — the MENU: `sales` (>= 2.16.10) sends `components: string[]`
            // (labels already composed, supplement included) besides the one-line `notes`. One
            // indented line per component, normal weight, NO amount — the header is the line that
            // carries the money, so it is the components that step back (the opposite emphasis of
            // the kitchen chit, ADR-0394, same indentation). While `sales` still joins the same
            // components into `notes`, the list is preferred and the note is skipped: printing
            // both would say everything twice. An item without the list prints exactly as today.
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
            // hub#1138 — the SUPPLEMENTS of the line, structured. `components` says what the MENU
            // is made of; `modifiers` says what the customer CHANGED. They are different axes, so
            // both print, components first and the change after — the same order the kitchen chit
            // uses. Chained into `notes` they wrapped at 32 columns and the continuation started at
            // the margin, reading as another article; one line each, indented, fixes exactly that.
            //
            // No amount of their own: in ERPlora the delta already lives inside the line's
            // `unit_price` (`authoritative_modifiers`), so the printed `line_total` includes it and
            // a second number in that column would not add up with the rest (sales#148).
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

    // **El SEGUNDO QR: «pide tu factura»** (hub#963). No sustituye al de arriba y por eso son dos
    // campos: el de VeriFactu apunta a la Sede de la AEAT (`ValidarQR`) y sirve para COTEJAR; este
    // apunta a este hub y sirve para PEDIR. Ágora imprime «CREAR FACTURA», Cuiner imprime
    // localizador + QR — y los dos lo ponen al pie, junto al fiscal, sobre el mismo papel.
    //
    // El **localizador en texto** va debajo y no es decorativo: es la única vía cuando la cámara no
    // enfoca, el móvil no tiene batería o el tique se fotocopia. Por eso el alfabeto Crockford del
    // core evita `I`, `L`, `O` y `U` — se teclea de un papel térmico, no se lee de una pantalla.
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

    // **The business's promotional QR** (hub#2009): reviews, social media — the link it set in
    // the POS settings. It closes the paper, like `<ok-receipt>` and the browser paper (sales#345),
    // and prints smaller than the fiscal QR. Without the URL nothing prints: a note alone names
    // nothing. Only on the ticket — the full invoice is formal, like the A4.
    if kind == Fiscal::Ticket && is_truthy(data, "promo_qr") {
        if is_truthy(data, "promo_note") {
            print_wrapped(b, str_field(data, "promo_note", ""));
        }
        b.qr_sized(str_field(data, "promo_qr", ""), PROMO_QR_MODULE_DOTS);
        b.text("\n");
    }

    b.cut();
}

/// **The bill taken to the table before charging** (ADR-0141, hub#748).
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
/// The Spanish literals are the ones this whole file prints (`COCINA`, `TOTAL`, `Fecha`): the paper
/// is composed at the device, which has no translator, so what the producer wants said in the
/// customer's language it sends — that is what `notice` is for.
fn render_prebill(b: &mut EscposBuilder, data: &serde_json::Value) {
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
            // hub#1165 / ADR-0396 — the MENU: `sales` (>= 2.16.10) sends `components: string[]`
            // (labels already composed, supplement included) besides the one-line `notes`. One
            // indented line per component, normal weight, NO amount — the header is the line that
            // carries the money, so it is the components that step back (the opposite emphasis of
            // the kitchen chit, ADR-0394, same indentation). While `sales` still joins the same
            // components into `notes`, the list is preferred and the note is skipped: printing
            // both would say everything twice. An item without the list prints exactly as today.
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
            // hub#1138 — the SUPPLEMENTS of the line, structured. `components` says what the MENU
            // is made of; `modifiers` says what the customer CHANGED. They are different axes, so
            // both print, components first and the change after — the same order the kitchen chit
            // uses. Chained into `notes` they wrapped at 32 columns and the continuation started at
            // the margin, reading as another article; one line each, indented, fixes exactly that.
            //
            // No amount of their own: in ERPlora the delta already lives inside the line's
            // `unit_price` (`authoritative_modifiers`), so the printed `line_total` includes it and
            // a second number in that column would not add up with the rest (sales#148).
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

/// Porta `_print_kitchen_order`.
fn render_kitchen_order(b: &mut EscposBuilder, data: &serde_json::Value) {
    let t = Locale::from_document(data);
    b.set(Align::Center, true, true, true);
    b.text(&format!("{}\n", t.label(Label::KitchenTitle)));

    b.set(Align::Center, true, true, false);
    // order_number = data.get('receipt_id', data.get('order_number', ''))
    let order_number = data
        .get("receipt_id")
        .and_then(|v| v.as_str())
        .or_else(|| data.get("order_number").and_then(|v| v.as_str()))
        .unwrap_or("");
    b.text(&format!("#{order_number}\n"));

    b.set(Align::Center, false, false, false);
    b.text("================================\n");

    // **La etiqueta de sala** (hub#1156 · ADR-0141/0144). El shell manda `label` desde siempre y
    // aquí se leía `table`, un campo que no manda nadie: la comanda salía sin decir de qué mesa
    // era, que en hora punta es papel inservible.
    //
    // `label` se imprime **tal cual, sin prefijo**: es opaca a propósito — «Mesa 4», «Barra»,
    // «Recogida Ana». Cocina no sabe qué es una mesa, ni tiene por qué; anteponerle «Mesa: »
    // produciría «Mesa: Recogida Ana».
    //
    // `table` sigue vivo detrás, con su prefijo de siempre: el contrato del dispositivo se amplía,
    // nunca se sustituye. Hoy no hay ningún productor que lo mande —de ahí venía el fallo— pero
    // arreglar el camino nuevo no puede dejar sin mesa a una integración que use el viejo.
    // Doble alto Y doble ancho: la mesa es lo más grande del papel a propósito. Es el único campo
    // que se lee desde el OTRO LADO del pase, a velocidad, para casar un plato con su destino — el
    // plato se lee a distancia de brazo. Toast lo pone «in large, bold font in the top-left corner»
    // y Eats365 le da tamaño propio junto al número de pedido, los dos únicos campos del chit que
    // lo tienen. Renuncia al doble ancho si no cabe: una etiqueta partida en dos trozos por la
    // impresora es justo lo contrario de un campo que existe para leerse de un vistazo.
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

    // La ronda distingue el segundo pase del primero en la misma mesa. Sólo a partir de la dos:
    // «Ronda 1» sería una línea de ruido en el 99 % del papel, que es justo el que nadie relee.
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
        // El MENÚ se pinta como una TIRADA de líneas consecutivas con el mismo `combo_ref`
        // (hub#1156 · kitchen#57 · ADR-0381), igual que `groupCombos()` en el KDS. Las líneas
        // llegan ordenadas por `line_seq`, así que la tirada ES el grupo.
        let mut combo: Option<&str> = None;
        for item in items {
            let this_combo = item
                .get("combo_ref")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty());

            if this_combo != combo {
                combo = this_combo;
                // **La cabecera del menú va SIN negrita y a altura normal.** El realce es para los
                // CAMBIOS —alérgenos y suplementos—, nunca para la jerarquía: un menú en negrita le
                // roba el ojo a lo que hay que cocinar. De quince productos revisados en kitchen#57,
                // CERO imprimen el nombre del combo en doble altura y CERO en Font B; la jerarquía
                // del chit se hace con SANGRADO (Simphony: «indented beneath the combo meal name»,
                // y lo dice para modo chit; Aloha lo tiene como parámetro, `QC indentation size`).
                //
                // Se emite sólo si el menú tiene nombre: sin él el sangrado agrupa igual y no hay
                // que inventarse un relleno — que además sería una cadena nueva cableada en un
                // renderizador que hoy no traduce nada.
                if this_combo.is_some() && is_truthy(item, "combo_name") {
                    b.set(Align::Left, false, false, false);
                    b.text(&format!("{}\n", str_field(item, "combo_name", "")));
                }
            }

            // Dos niveles como mucho: a 32 columnas un tercero deja el texto sin sitio.
            let indent = if combo.is_some() { "  " } else { "" };
            let qty = item.get("quantity").and_then(|v| v.as_f64()).unwrap_or(1.0);
            let name = str_field(item, "name", "");

            b.set(Align::Left, true, true, false);
            b.text(&format!(
                "{indent}{}x {name}\n",
                fmt_qty(item.get("quantity"), qty)
            ));

            // **Los suplementos** (hub#1156 · pm#93). Van ANTES de la nota libre y con el mismo
            // sangrado: el suplemento lo eligió el cliente en la carta y cambia el plato, la nota
            // es texto del camarero. Se parten a mano conservando el sangrado — si lo cortara la
            // térmica a 32 columnas, la continuación arrancaría pegada al margen y se leería como
            // un plato más de la comanda.
            //
            // Y EN NEGRITA: el realce es la otra mitad de la misma regla — el suplemento es un
            // CAMBIO, y un cambio es lo que devuelve el plato. Toast imprime el plato en negro y el
            // modificador en rojo, Lightspeed tiene `print sub-items in red`, Revel igual: tres de
            // tres realzan el cambio. En una térmica sin cinta bicolor, el equivalente es la negrita.
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

            // La nota libre del camarero NO es un cambio de la carta: se lee, no se grita.
            if is_truthy(item, "notes") {
                b.set(Align::Left, false, false, false);
                b.text(&format!("{sub}>> {}\n", str_field(item, "notes", "")));
            }
        }
    }

    b.text("================================\n");

    let priority = str_field(data, "priority", "NORMAL");
    if priority == "HIGH" {
        b.set(Align::Center, true, true, false);
        b.text(&format!("{}\n", t.label(Label::Rush)));
    }

    b.text("\n");
    b.cut();
}

/// Porta `_print_delivery_note`.
fn render_delivery_note(b: &mut EscposBuilder, data: &serde_json::Value) {
    let t = Locale::from_document(data);
    b.set(Align::Center, true, false, false);
    b.text(&format!("{}\n", t.label(Label::DeliveryNoteTitle)));
    b.text("================================\n");

    b.set(Align::Left, false, false, false);
    b.text(&format!("{}{}\n", t.label(Label::Number), str_field(data, "receipt_id", "")));
    b.text(&format!("{}{}\n", t.label(Label::Date), now_dmy_hm()));

    if is_truthy(data, "customer_name") {
        b.text(&format!("{}{}\n", t.label(Label::Customer), str_field(data, "customer_name", "")));
    }
    if is_truthy(data, "delivery_address") {
        b.text(&format!("{}{}\n", t.label(Label::Address), str_field(data, "delivery_address", "")));
    }

    b.text("--------------------------------\n");

    if let Some(items) = data.get("items").and_then(|v| v.as_array()) {
        for item in items {
            let qty = item.get("quantity").and_then(|v| v.as_f64()).unwrap_or(1.0);
            let name = str_field(item, "name", "");
            b.text(&format!("{}x {name}\n", fmt_qty(item.get("quantity"), qty)));
        }
    }

    b.text("================================\n");
    b.text(&format!("\n{}\n\n", t.label(Label::Signature)));
    b.cut();
}

/// Porta `_print_barcode_label`.
fn render_barcode_label(b: &mut EscposBuilder, data: &serde_json::Value) {
    b.set(Align::Center, true, false, false);
    b.text(&format!("{}\n", str_field(data, "product_name", "")));

    let barcode_value = str_field(data, "barcode", "");
    if !barcode_value.is_empty() {
        // GS k nativo (EAN13/CODE128) con HRI debajo; ya no es el fallback de texto [code].
        b.set(Align::Center, false, false, false);
        b.barcode(barcode_value);
    }

    if let Some(price) = data.get("price").and_then(|v| v.as_f64()) {
        b.set(Align::Center, true, true, false);
        let price = b.money(price);
        b.text(&format!("{price}\n"));
    }

    b.cut();
}

/// Porta `_print_cash_report`.
fn render_cash_report(b: &mut EscposBuilder, data: &serde_json::Value) {
    let t = Locale::from_document(data);
    b.set(Align::Center, true, false, false);
    b.text(&format!("{}\n", t.label(Label::CashReportTitle)));
    b.text("================================\n");

    b.set(Align::Left, false, false, false);
    b.text(&format!("{}{}\n", t.label(Label::Session), str_field(data, "receipt_id", "")));
    b.text(&format!("{}{}\n", t.label(Label::Date), now_dmy_hm()));

    if is_truthy(data, "cashier") {
        b.text(&format!("{}{}\n", t.label(Label::Cashier), str_field(data, "cashier", "")));
    }

    b.text("--------------------------------\n");

    let opening = data.get("opening_balance").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let closing = data.get("closing_balance").and_then(|v| v.as_f64()).unwrap_or(0.0);
    b.total_line(t.label(Label::Opening), opening);
    b.total_line(t.label(Label::Closing), closing);

    let diff = closing - opening;
    b.total_line(t.label(Label::Difference), diff);

    b.text("--------------------------------\n");

    if let Some(transactions) = data.get("transactions").and_then(|v| v.as_array()) {
        for tx in transactions {
            // label = tx.get('label', tx.get('type', ''))
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

/// Porta `_print_generic`.
fn render_generic(b: &mut EscposBuilder, data: &serde_json::Value) {
    let t = Locale::from_document(data);
    b.set(Align::Center, true, false, false);
    b.text(&format!("{}\n", str_field(data, "title", t.label(Label::GenericTitle))));
    b.text("================================\n");

    b.set(Align::Left, false, false, false);
    if let Some(obj) = data.as_object() {
        for (key, value) in obj {
            // `locale` steers HOW the paper is printed (hub#1159); it is a field of the
            // envelope, not a line of the document. Without this the generic renderer — which
            // prints every key it does not know — would print «locale: en» on the paper.
            if key == "title" || key == "receipt_id" || key == "locale" {
                continue;
            }
            b.text(&format!("{key}: {}\n", json_to_display(value)));
        }
    }

    b.text("================================\n\n");
    b.cut();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── La comanda de cocina imprime lo que la fila trae (hub#1156) ─────────────────────────────

    /// Rinde una comanda y devuelve el papel **sin bytes de control**, que es lo que lee un
    /// cocinero. Sin esto, cada aserción compara contra un texto salpicado de `ESC a`/`GS !` y
    /// acaba pasando por casualidad.
    fn paper(data: &serde_json::Value) -> String {
        let bytes = render_document(DocumentType::KitchenOrder, data).expect("comanda válida");
        strip_escpos(&bytes)
    }

    /// Quita los mandos ESC/POS y deja el texto que un cocinero lee.
    ///
    /// Filtrar «bytes de control» a secas NO vale y es una trampa que ya mordió: en `ESC a 0` sólo
    /// el `ESC` es de control — la `a` y el `0` son ASCII imprimible, así que la comanda se leía
    /// como `aE!0COCINA` y cualquier `contains` pasaba por casualidad sobre basura.
    ///
    /// Los mandos de la comanda son todos de tres bytes (`ESC a n`, `ESC E n`, `ESC M n`,
    /// `GS ! n`, `GS V n`): no hay QR ni código de barras aquí, que son los de longitud variable.
    /// Si algún día entra uno, este helper lo delata en vez de tragárselo.
    fn strip_escpos(bytes: &[u8]) -> String {
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                0x1b | 0x1d => {
                    assert!(
                        i + 2 < bytes.len(),
                        "mando ESC/POS truncado en {i}: el renderizador emitió bytes a medias"
                    );
                    assert!(
                        !matches!((bytes[i], bytes[i + 1]), (0x1d, 0x28) | (0x1d, 0x6b)),
                        "mando de longitud variable (QR/barcode) en una comanda: este helper \
                         sólo sabe de mandos de 3 bytes y lo estaría cortando mal"
                    );
                    i += 3;
                }
                b => {
                    out.push(b);
                    i += 1;
                }
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    /// Cómo sale IMPRESA cada línea, no sólo qué dice: `(texto, negrita, doble_alto)`.
    ///
    /// La jerarquía del papel se decide en los modos ESC/POS, no en el texto, así que un test que
    /// sólo mire el texto no puede distinguir una cabecera discreta de una cabecera que le roba el
    /// ojo al plato — que es justo lo que hub#1156 tiene que garantizar.
    fn lines_with_modes(bytes: &[u8]) -> Vec<(String, bool, bool)> {
        let (mut bold, mut double_h) = (false, false);
        let (mut out, mut cur) = (Vec::new(), Vec::new());
        // El modo vigente cuando ARRANCA la línea es el que la imprime.
        let (mut line_bold, mut line_double) = (false, false);
        let mut fresh = true;
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                0x1b if bytes.get(i + 1) == Some(&0x45) => {
                    bold = bytes[i + 2] == 1;
                    i += 3;
                }
                0x1d if bytes.get(i + 1) == Some(&0x21) => {
                    double_h = bytes[i + 2] & 0x10 != 0;
                    i += 3;
                }
                0x1b | 0x1d => i += 3,
                b'\n' => {
                    out.push((
                        String::from_utf8_lossy(&cur).into_owned(),
                        line_bold,
                        line_double,
                    ));
                    cur.clear();
                    fresh = true;
                    i += 1;
                }
                b => {
                    if fresh {
                        line_bold = bold;
                        line_double = double_h;
                        fresh = false;
                    }
                    cur.push(b);
                    i += 1;
                }
            }
        }
        out
    }

    /// Si la línea `txt` se imprimió con doble ancho (`GS ! n`, bit 0x20).
    fn double_width_of(bytes: &[u8], txt: &str) -> bool {
        let (mut wide, mut i) = (false, 0usize);
        while i < bytes.len() {
            match bytes[i] {
                0x1d if bytes.get(i + 1) == Some(&0x21) => {
                    wide = bytes[i + 2] & 0x20 != 0;
                    i += 3;
                }
                0x1b | 0x1d => i += 3,
                _ => {
                    if bytes[i..].starts_with(txt.as_bytes()) {
                        return wide;
                    }
                    i += 1;
                }
            }
        }
        panic!("«{txt}» no está en el papel");
    }

    /// Los dos lectores de arriba se prueban a sí mismos, por lo mismo que `strip_escpos`: si
    /// devolvieran siempre `false`, las aserciones de jerarquía saldrían verdes sobre cualquier cosa.
    #[test]
    fn the_mode_reader_tells_a_loud_line_from_a_quiet_one() {
        let mut b = EscposBuilder::new();
        b.set(Align::Left, true, true, true).text("PLATO\n");
        b.set(Align::Left, false, false, false).text("nota\n");
        let bytes = b.finish();
        assert_eq!(
            lines_with_modes(&bytes),
            vec![
                ("PLATO".to_string(), true, true),
                ("nota".to_string(), false, false),
            ]
        );
        assert!(double_width_of(&bytes, "PLATO"));
        assert!(!double_width_of(&bytes, "nota"));
    }

    /// El helper de arriba es el que sostiene todas las aserciones de la comanda, así que se prueba
    /// a sí mismo: si dejara residuo, un `contains("Mesa 4")` seguiría pasando sobre `aE!0Mesa 4`
    /// y las pruebas de este bloque valdrían para nada.
    #[test]
    fn the_paper_helper_leaves_no_command_residue() {
        let mut b = EscposBuilder::new();
        b.set(Align::Center, true, true, true).text("COCINA\n");
        b.set(Align::Left, false, false, false).text("2x Croquetas\n");
        assert_eq!(strip_escpos(&b.finish()), "COCINA\n2x Croquetas\n");
    }

    /// **El suplemento sale por la impresora, no sólo por la pantalla** (hub#1156 · pm#93).
    ///
    /// «Sin cebolla» se congela en la fila y el KDS lo pinta, pero la cocina caliente ES papel: en
    /// la plancha nadie mira una pantalla con las manos ocupadas. Un suplemento que no se imprime
    /// es el plato que vuelve, y eso no es cosmética.
    #[test]
    fn a_supplement_reaches_the_paper_under_its_own_line() {
        let text = paper(&json!({
            "receipt_id": "K-217",
            "items": [{ "name": "Entrecot", "quantity": 1, "modifiers": "Al punto, sin cebolla" }],
        }));
        assert!(text.contains("Entrecot"), "el plato está en el papel:\n{text}");
        assert!(
            text.contains("Al punto, sin cebolla"),
            "el suplemento está en el papel — es lo que hace que el plato vuelva:\n{text}"
        );
        // Debajo de SU plato, no al final de la hoja: en una comanda de ocho líneas, un suplemento
        // suelto al pie no dice a qué plato pertenece.
        let plato = text.find("Entrecot").expect("el plato");
        let suplemento = text.find("Al punto").expect("el suplemento");
        assert!(suplemento > plato, "el suplemento va DEBAJO de su línea:\n{text}");
    }

    // ── hub#1165 · the MENU on the customer's ticket and on the bill (ADR-0396) ────────────────
    //
    // `sales` (>= 2.16.10) sends, besides `notes`, `components: string[]` on a menu item: labels
    // already composed (name + supplement «(+3,00)»), in the order they were chosen. At 32 columns
    // the one-line `notes` wraps and the continuation starts at the margin — it reads as another
    // item. Same complaint hub#1156 fixed for the kitchen chit; here the header keeps its normal
    // weight (it carries the money) and the components step back: indent, no amount, no bold.

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

    /// With `components` present, `notes` is NOT printed: until `sales` separates them, `notes`
    /// still carries the same components joined by « · » and printing both would say everything
    /// twice (the issue's second acceptance criterion).
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
    /// must not move (sales#78 — unknown keys are ignored, known shapes are frozen).
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

    /// The bill taken to the table says the same as the ticket (ADR-0396 §5: one composer, two
    /// papers): components indented under the menu header, no note duplication.
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

    /// **El suplemento y la nota conviven, y el suplemento va primero.** Son dos cosas distintas:
    /// el suplemento lo eligió el cliente en la carta y cambia el plato; la nota es texto libre del
    /// camarero. Perder una al pintar la otra fue el fallo original.
    #[test]
    fn a_line_can_carry_both_a_supplement_and_a_free_note() {
        let text = paper(&json!({
            "items": [{ "name": "Croquetas", "quantity": 2, "modifiers": "Sin gluten", "notes": "para compartir" }],
        }));
        let mods = text.find("Sin gluten").expect("el suplemento está en el papel");
        let notes = text.find("para compartir").expect("la nota está en el papel");
        assert!(mods < notes, "el suplemento va antes que la nota libre:\n{text}");
    }

    /// **La comanda dice de qué mesa es** (hub#1156 · ADR-0141/0144).
    ///
    /// El shell manda `label` desde siempre y el renderizador leía `table`, un campo que no manda
    /// nadie: la comanda salía sin la única cosa que cocina sabe de la sala. En hora punta, una
    /// comanda sin mesa es papel que no sirve para nada.
    ///
    /// La etiqueta se imprime **tal cual**, sin prefijo: es opaca a propósito — «Mesa 4», «Barra»,
    /// «Recogida Ana». Cocina no sabe qué es una mesa, ni tiene por qué.
    #[test]
    fn the_kitchen_order_says_which_table_it_is_for() {
        let text = paper(&json!({
            "receipt_id": "K-217",
            "label": "Mesa 4",
            "round_number": 2,
            "items": [{ "name": "Croquetas", "quantity": 2 }],
        }));
        assert!(text.contains("Mesa 4"), "la etiqueta de sala está en el papel:\n{text}");
        assert!(!text.contains("Mesa: Mesa 4"), "tal cual, sin prefijo: la etiqueta es opaca:\n{text}");
        // La ronda distingue el segundo pase del primero en la misma mesa. Viajaba y nadie la leía.
        // Se afirma la palabra entera: `contains('2')` habría pasado por el «K-217» de arriba —
        // un control que acierta por casualidad no prueba nada.
        assert!(text.contains("Ronda 2"), "la ronda está en el papel:\n{text}");
    }

    /// **The VOID slip of a cancelled round is a `kitchen_order` (kitchen#168).** The shell
    /// (`print-void.ts`) says the void with what an installed app that never updates already prints:
    /// the word in the floor label and a negative quantity per dish. If this renderer ever dropped
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

    /// **La primera ronda NO se anuncia.** Es el caso normal —una comanda que no es un segundo
    /// pase— y ponerle «Ronda 1» le añadiría una línea de ruido al 99 % del papel. La ronda dice
    /// «esto ya es el segundo envío de esta mesa», y eso sólo es cierto a partir de la dos.
    #[test]
    fn the_first_round_is_not_announced() {
        let text = paper(&json!({
            "label": "Mesa 4", "round_number": 1,
            "items": [{ "name": "Croquetas", "quantity": 2 }],
        }));
        assert!(text.contains("Mesa 4"), "la mesa sí:\n{text}");
        assert!(!text.contains("Ronda"), "la ronda no:\n{text}");
    }

    /// **Un suplemento largo se parte con su sangrado, no lo parte la impresora.** A 32 columnas,
    /// «Al punto, sin cebolla, sin sal, extra salsa» desborda; si lo corta la térmica, el resto
    /// arranca pegado al margen y se lee como un plato más de la comanda.
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
        // Sin esto el `for` de abajo no se ejecutaría nunca y el test saldría verde sobre una
        // comanda que ni siquiera imprime el suplemento — un bucle vacío no prueba nada.
        assert!(
            partes.len() >= 2,
            "el suplemento largo se parte en varias líneas, no se pierde:\n{text}"
        );
        for line in partes {
            assert!(line.starts_with("   "), "la continuación conserva el sangrado: {line:?}");
            assert!(
                line.chars().count() <= LINE_WIDTH,
                "y cabe en el papel ({LINE_WIDTH} columnas): {line:?}"
            );
        }
    }

    /// **Lo que ya mandaba `table` sigue imprimiéndose.** No hay ningún productor vivo que lo mande
    /// —de ahí el fallo— pero el contrato del dispositivo se amplía, nunca se sustituye: una
    /// integración que lo use no puede quedarse sin mesa por arreglar el camino nuevo.
    #[test]
    fn the_legacy_table_field_still_prints() {
        let text = paper(&json!({ "table": "4", "items": [{ "name": "Croquetas", "quantity": 2 }] }));
        assert!(text.contains("Mesa: 4"), "la forma vieja conserva su prefijo:\n{text}");
    }

    /// **Un MENÚ sale como cabecera + componentes SANGRADOS, nunca como párrafo** (hub#1156 ·
    /// kitchen#57 · ADR-0381).
    ///
    /// El sangrado es el eje de jerarquía del sector, no el tamaño: Simphony documenta los
    /// componentes *«indented beneath the combo meal name»* **y dice explícitamente que vale en
    /// modo chit**, y Aloha lo eleva a parámetro numérico (`QC indentation size`). El fallo
    /// estrella es el contrario: Square amontona el combo en *«one long run-on paragraph, which
    /// makes it kind of hard to decipher when in the kitchen»*, y su comunidad lo tiene abierto
    /// desde hace años con «comprar otra impresora» como único remedio.
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
            "la cabecera del menú está en el papel, a la izquierda del todo:\n{text}"
        );
        // Cada componente en SU línea y sangrado dos columnas. Un párrafo corrido sería el fallo de
        // Square; la misma columna que las croquetas los haría tres platos sueltos.
        assert!(lines.contains(&"  1x Gazpacho"), "componente sangrado:\n{text}");
        assert!(lines.contains(&"  1x Entrecot"), "componente sangrado:\n{text}");
        assert!(lines.contains(&"2x Croquetas"), "lo de la carta NO se sangra:\n{text}");
    }

    /// 🔴 **La cabecera del menú NO le roba el ojo al plato** (kitchen#57).
    ///
    /// La regla es «el realce es para alérgenos y cambios, NUNCA para jerarquía», y en papel se
    /// traduce quitándole la NEGRITA a la cabecera, no encogiéndola: de quince productos revisados,
    /// **cero** imprimen el nombre del combo en doble altura y **cero** en Font B. Eats365 da
    /// tamaño propio a seis campos del chit —nº de pedido, mesa, notas…— y al nombre del combo sólo
    /// un interruptor de visibilidad: cuando un fabricante te da tamaño para seis cosas y para la
    /// séptima sólo «sí/no», te está diciendo que esa séptima no es una instrucción de cocina.
    ///
    /// Se afirma sobre los MODOS ESC/POS, no sobre el texto: la jerarquía del papel vive ahí, y un
    /// test que sólo leyera el texto no distinguiría una cabecera discreta de una que grita.
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
            .expect("la cabecera está en el papel");
        assert_eq!(
            (cabecera.1, cabecera.2),
            (false, false),
            "la cabecera del menú va SIN negrita y a altura normal: es contexto, no trabajo"
        );
        // Control positivo, que es lo que hace que la aserción de arriba signifique algo: los
        // platos —lo que de verdad hay que cocinar— sí van realzados.
        for plato in ["  1x Gazpacho", "  1x Entrecot"] {
            let l = modes.iter().find(|(t, _, _)| t == plato).expect(plato);
            assert_eq!(
                (l.1, l.2),
                (true, true),
                "el plato manda sobre la cabecera: {plato}"
            );
        }
    }

    /// **El suplemento SÍ va realzado** — la otra mitad de la misma regla. El realce es para los
    /// CAMBIOS, y un cambio es exactamente lo que es un suplemento. Toast imprime el plato en negro
    /// y el modificador en ROJO, Lightspeed tiene `print sub-items in red` y Revel igual: tres de
    /// tres realzan el cambio. En una térmica sin cinta bicolor, el equivalente es la negrita.
    #[test]
    fn a_supplement_is_emphasised_because_it_is_a_change_not_a_hierarchy() {
        let bytes = render_document(
            DocumentType::KitchenOrder,
            &json!({ "items": [{ "name": "Entrecot", "quantity": 1, "modifiers": "SIN CEBOLLA", "notes": "mesa con prisa" }] }),
        )
        .unwrap();
        let modes = lines_with_modes(&bytes);
        let sup = modes.iter().find(|(t, _, _)| t.contains("SIN CEBOLLA")).expect("el suplemento");
        assert!(sup.1, "el suplemento va en negrita: es el cambio que devuelve el plato");
        assert!(!sup.2, "pero a altura normal: el plato sigue mandando");
        // La nota libre del camarero NO es un cambio de la carta: se lee, no se grita.
        let nota = modes.iter().find(|(t, _, _)| t.contains("mesa con prisa")).expect("la nota");
        assert!(!nota.1, "la nota libre no compite con el suplemento");
    }

    /// **La cabecera se repite en CADA hoja de estación** (opción `11 - Send to Combo Parent Order
    /// Devices` de Simphony; TouchBistro lo hace por defecto y sin opción).
    ///
    /// Cada hoja es un documento propio con las líneas de SU rol, así que se comprueba como se
    /// imprime: dos renderizados. Un cocinero de la plancha que no lee «MENÚ» no sabe que su
    /// entrecot va acoplado a un gazpacho, y lo saca cuando le viene bien. El modo huérfano de
    /// Toast —el componente sin el nombre del padre— es al que se entra a propósito, no el defecto.
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
                "la hoja de {estacion} dice de qué menú es:\n{hoja}"
            );
        }
    }

    /// **Un menú sin nombre sigue agrupándose por el SANGRADO**, sin una línea en blanco donde
    /// iría la cabecera. El sangrado es el mecanismo principal, no la cabecera: se sostiene solo.
    /// Y así no hay que inventarse un «MENÚ» de relleno, que sería una cadena nueva cableada en un
    /// renderizador que hoy no traduce nada.
    #[test]
    fn a_nameless_menu_still_groups_its_components_by_indentation() {
        let text = paper(&json!({
            "items": [
                { "name": "Gazpacho", "quantity": 1, "combo_ref": "c1", "combo_name": "" },
                { "name": "Entrecot", "quantity": 1, "combo_ref": "c1", "combo_name": "" }
            ],
        }));
        assert!(text.contains("  1x Gazpacho"), "sigue sangrado:\n{text}");
        assert!(text.contains("  1x Entrecot"), "sigue sangrado:\n{text}");
        // Y no se cuela una línea vacía donde iría la cabecera: el primer componente va PEGADO a la
        // regla que abre la lista. (Se mira el vecino, no «hay alguna línea vacía»: el corte de
        // papel deja tres al final y esa comprobación saltaría siempre.)
        let lines: Vec<&str> = text.lines().collect();
        let i = lines.iter().position(|l| *l == "  1x Gazpacho").expect("el componente");
        assert!(
            lines[i - 1].starts_with('-'),
            "el componente sigue a la regla, sin cabecera en blanco por medio:\n{text}"
        );
    }

    /// **El suplemento de un componente se sangra un nivel más** (0 → 2 → 5). Dos niveles como
    /// mucho: a 32 columnas, un tercero deja el texto sin sitio y lo parte la impresora.
    #[test]
    fn a_supplement_of_a_menu_component_indents_one_level_further() {
        let text = paper(&json!({
            "items": [{
                "name": "Entrecot", "quantity": 1, "modifiers": "Al punto",
                "combo_ref": "c1", "combo_name": "MENU DEL DIA",
            }],
        }));
        assert!(text.contains("\n     Al punto\n"), "sangrado a 5, bajo su componente:\n{text}");
    }

    /// **La mesa es lo MÁS GRANDE del papel**: doble alto y doble ancho.
    ///
    /// Es el único campo que se lee desde el otro lado del pase, a velocidad, para casar un plato
    /// con su destino — el plato se lee a distancia de brazo, la mesa a distancia de sala. Toast lo
    /// pone *«in large, bold font in the top-left corner»* y Eats365 le da tamaño propio junto al
    /// número de pedido, los dos únicos campos del chit que lo tienen.
    #[test]
    fn the_table_is_the_biggest_thing_on_the_paper() {
        let bytes = render_document(
            DocumentType::KitchenOrder,
            &json!({ "label": "Mesa 4", "items": [{ "name": "Croquetas", "quantity": 2 }] }),
        )
        .unwrap();
        let modes = lines_with_modes(&bytes);
        let mesa = modes.iter().find(|(t, _, _)| t == "Mesa 4").expect("la mesa");
        assert!(mesa.1 && mesa.2, "la mesa va en negrita y doble alto");
        assert!(
            double_width_of(&bytes, "Mesa 4"),
            "y en doble ancho: es lo que se lee desde el otro lado del pase"
        );
    }

    /// **Una etiqueta larga renuncia al doble ancho antes que partirse.** «Recogida Ana Martinez» a
    /// doble ancho ocupa 42 de las 32 columnas del papel: la impresora la corta por donde le toca y
    /// la mesa —el campo que existe para leerse de un vistazo— acaba en dos trozos.
    #[test]
    fn a_long_label_gives_up_double_width_before_it_wraps() {
        let bytes = render_document(
            DocumentType::KitchenOrder,
            &json!({ "label": "Recogida Ana Martinez", "items": [{ "name": "Cafe", "quantity": 1 }] }),
        )
        .unwrap();
        assert!(
            !double_width_of(&bytes, "Recogida Ana Martinez"),
            "cabe entera aunque sea a un solo ancho"
        );
        let text = strip_escpos(&bytes);
        assert!(text.contains("Recogida Ana Martinez"), "y no se pierde:\n{text}");
    }

    /// **Una comanda a la carta sale EXACTAMENTE igual que antes de hub#1156.** Es el 99 % de las
    /// comandas: romper esto es romper la cocina entera para arreglar un caso raro.
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
            "el papel de siempre, byte a byte"
        );
    }

    /// **The wire vocabulary, pinned from this side.** The hub's queue keeps the same eight names
    /// (`erplora_runtime::print_queue::DOCUMENT_TYPES`) and refuses anything else at the door; the
    /// two lists are duplicated on purpose (the hub server has no business linking the hardware
    /// crate), so each side pins its own or they drift — and a name the queue accepts becomes a
    /// job the printer refuses, one layer too late to tell the producer.
    #[test]
    fn the_wire_vocabulary_is_exactly_these_eight() {
        let vocabulary = [
            ("receipt", DocumentType::Receipt),
            ("kitchen_order", DocumentType::KitchenOrder),
            ("invoice", DocumentType::Invoice),
            ("delivery_note", DocumentType::DeliveryNote),
            ("barcode_label", DocumentType::BarcodeLabel),
            ("cash_session_report", DocumentType::CashSessionReport),
            ("prebill", DocumentType::Prebill),
            ("generic", DocumentType::Generic),
        ];
        for (wire, expected) in vocabulary {
            assert_eq!(DocumentType::parse(wire), Some(expected), "`{wire}`");
            // The serde spelling is the same one: the queue stores this string and the frame
            // carries it, so a rename here would break the wire without touching this list.
            assert_eq!(
                serde_json::to_value(expected).unwrap(),
                json!(wire),
                "`{wire}` must serialise as itself"
            );
        }
    }

    /// **A document type nobody knows is refused, not printed as `Generic`.** A typo used to reach
    /// the paper as a nameless key/value dump, and the kitchen only noticed when the plate did not
    /// arrive. `parse` is now the ONLY way in — the lenient `from_wire` that degraded to `Generic`
    /// went with its last caller, the standalone Bridge (hub#340 / hub#578).
    #[test]
    fn an_unknown_document_type_is_refused_instead_of_degraded() {
        for unknown in ["Kitchen", "kitchn", "", "html", "KITCHEN_ORDER"] {
            assert_eq!(
                DocumentType::parse(unknown),
                None,
                "`{unknown}` is not a document this printer renders"
            );
        }
    }

    /// **`generic` is a document you ASK for, never one you fall back into.** The distinction is
    /// the whole point of removing the lenient door: the type still exists, and a producer may
    /// legitimately name it, but nothing maps an unknown string onto it any more.
    #[test]
    fn generic_is_only_reachable_by_naming_it() {
        assert_eq!(DocumentType::parse("generic"), Some(DocumentType::Generic));
        for typo in ["Generic", "generi", "gener1c", "unknown"] {
            assert_eq!(
                DocumentType::parse(typo),
                None,
                "`{typo}` must not slide into `Generic`"
            );
        }
    }

    /// **A `data` that is not an object is refused before it becomes blank paper.** Every renderer
    /// reads by key, so an array or a string produces a ticket with nothing on it — and a cut.
    #[test]
    fn a_document_that_is_not_an_object_is_refused() {
        for bad in [json!(null), json!("<p>ticket</p>"), json!([1, 2]), json!(7)] {
            let err = render_document(DocumentType::Receipt, &bad)
                .expect_err("{bad} is not a document");
            assert!(
                matches!(err, crate::PeripheralError::InvalidPayload(_)),
                "the refusal says the payload is wrong, not that the printer is: {err}"
            );
        }
    }

    /// Sanity of the builder every renderer is written on: a QR plus a cut must produce Epson's QR
    /// stamp (`GS ( k`) and the cut (`GS V 1`). Without it, a renderer can look right and still
    /// hand the printer bytes it does not understand.
    #[test]
    fn builder_qr_and_cut_smoke() {
        let mut b = EscposBuilder::new();
        b.set(Align::Center, false, false, false).qr("https://erplora.com/v/abc").cut();
        let out = b.finish();
        assert!(out.windows(3).any(|w| w == [0x1d, 0x28, 0x6b]), "debe contener GS ( k (QR)");
        assert!(out.windows(3).any(|w| w == [0x1d, 0x56, 0x01]), "debe contener GS V 1 (corte)");
    }

    /// The guard above rejects **shape**, never content: a real ticket still renders, and what comes
    /// out carries what was asked for. Without this, refusing everything would pass the test above.
    #[test]
    fn a_real_receipt_still_renders_its_lines_and_total() {
        let bytes = render_document(
            DocumentType::Receipt,
            &json!({
                "business_name": "Bar Manolo",
                "receipt_id": "T-42",
                "items": [{ "name": "Cafe", "quantity": 2, "total": 2.4 }],
                "total": 2.4,
            }),
        )
        .expect("a well-formed receipt renders");
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("Bar Manolo"), "the business name is on the paper");
        assert!(text.contains("T-42"), "the ticket number is on the paper");
        assert!(text.contains("2x Cafe"), "the line is on the paper");
        assert!(text.contains("TOTAL"), "the total is on the paper");
        assert_eq!(
            &bytes[bytes.len() - 3..],
            &[0x1d, 0x56, 0x01],
            "and the paper is cut, or the next ticket comes out attached to this one"
        );
    }

    // ── A reprint says «duplicado» (hub#1931, RD 1619/2012 art. 14.4) ───────────────────────────

    fn fiscal_paper(doc: DocumentType, extra: serde_json::Value) -> Vec<(String, bool, bool)> {
        let mut data = json!({
            "business_name": "Bar Manolo",
            "receipt_id": "T-42",
            "items": [{ "name": "Cafe", "quantity": 2, "total": 2.4 }],
            "total": 2.4,
        });
        // A full invoice is refused without its parties and its breakdown (hub#2005), so the
        // invoice gets them here: what these tests look at is the mark, not the refusal.
        if doc == DocumentType::Invoice {
            for (k, v) in full_invoice().as_object().expect("an object") {
                if data.get(k).is_none() {
                    data[k] = v.clone();
                }
            }
        }
        for (k, v) in extra.as_object().expect("extra fields are an object") {
            data[k] = v.clone();
        }
        let bytes = render_document(doc, &data).expect("a well-formed fiscal document renders");
        lines_with_modes(&bytes)
    }

    fn says(lines: &[(String, bool, bool)], word: &str) -> bool {
        lines.iter().any(|(text, _, _)| text.contains(word))
    }

    /// Only one original of an invoice may exist, and every other copy has to say «duplicado»
    /// (art. 14.4). A simplified invoice (the ticket) is an invoice too, so both documents carry it,
    /// in bold above the ticket data — where whoever gets the paper reads it first.
    #[test]
    fn a_duplicate_ticket_or_invoice_says_so_above_its_data() {
        for doc in [DocumentType::Receipt, DocumentType::Invoice] {
            let lines = fiscal_paper(doc, json!({ "duplicate": true }));
            let mark = lines
                .iter()
                .position(|(text, _, _)| text.trim() == "DUPLICADO")
                .unwrap_or_else(|| panic!("{doc:?}: the duplicate carries the mark, got {lines:?}"));
            assert!(lines[mark].1, "{doc:?}: the mark is printed in bold");
            let ticket = lines
                .iter()
                .position(|(text, _, _)| text.contains("T-42"))
                .expect("the document number is on the paper");
            assert!(mark < ticket, "{doc:?}: the mark comes before the ticket data");
        }
    }

    /// The original — the first print, the automatic one at checkout — carries no mark, and
    /// neither does a document that says `duplicate: false` or something that is not a boolean:
    /// the word is a legal statement, so only an explicit `true` prints it.
    #[test]
    fn the_original_carries_no_duplicate_mark() {
        for extra in [json!({}), json!({ "duplicate": false }), json!({ "duplicate": "yes" })] {
            let lines = fiscal_paper(DocumentType::Receipt, extra.clone());
            assert!(!says(&lines, "DUPLICADO"), "{extra}: no mark on the original, got {lines:?}");
        }
    }

    /// The mark speaks the language of the paper, like every other label (hub#1159).
    #[test]
    fn the_duplicate_mark_speaks_the_language_of_the_paper() {
        let lines = fiscal_paper(DocumentType::Receipt, json!({ "duplicate": true, "locale": "en" }));
        assert!(says(&lines, "DUPLICATE"), "an English paper says DUPLICATE, got {lines:?}");
        assert!(!says(&lines, "DUPLICADO"), "and not the Spanish word");
    }

    // ── A full invoice on the thermal roll (hub#2005) ──────────────────────────────────────────

    /// What makes an invoice a FULL invoice (RD 1619/2012 art. 6): both parties identified and
    /// the VAT broken down per rate. Two rates on purpose: a single-rate sale is the case where a
    /// breakdown and a lone `tax_amount` look the same on paper.
    fn full_invoice() -> serde_json::Value {
        json!({
            "business_name": "Bar Manolo",
            "vat_number": "B11111111",
            "receipt_id": "F-2026-000007",
            "customer_name": "Talleres Paco SL",
            "customer_tax_id": "B12345678",
            "customer_address": "Calle Mayor 3, Madrid",
            "items": [
                { "name": "Menu", "quantity": 1, "total": 11.0 },
                { "name": "Vino", "quantity": 1, "total": 12.1 }
            ],
            "tax_breakdown": [
                { "rate": 10, "base": 10.0, "tax": 1.0 },
                { "rate": 21, "base": 10.0, "tax": 2.1 }
            ],
            "subtotal": 20.0,
            "tax_amount": 3.1,
            "total": 23.1,
        })
    }

    fn invoice_paper(data: &serde_json::Value) -> Vec<(String, bool, bool)> {
        let bytes = render_document(DocumentType::Invoice, data).expect("a full invoice renders");
        lines_with_modes(&bytes)
    }

    fn line_index(lines: &[(String, bool, bool)], pred: impl Fn(&str) -> bool, what: &str) -> usize {
        lines
            .iter()
            .position(|(text, _, _)| pred(text))
            .unwrap_or_else(|| panic!("{what} is not on the paper: {lines:#?}"))
    }

    /// The paper says what it is: «FACTURA», bold and double height, above the document data —
    /// the first thing whoever gets it reads, and what tells it from a ticket at a glance.
    #[test]
    fn a_full_invoice_is_titled_factura_above_its_data() {
        let lines = invoice_paper(&full_invoice());
        let title = line_index(&lines, |l| l.trim() == "FACTURA", "the title");
        assert!(lines[title].1 && lines[title].2, "the title is bold and double height: {lines:#?}");
        let number = line_index(&lines, |l| l.contains("F-2026-000007"), "the invoice number");
        assert!(title < number, "the title comes before the number");
        assert!(
            lines[number].0.starts_with("Factura: "),
            "the number is the invoice's, not a ticket's: {:?}",
            lines[number].0
        );
        assert!(!says(&lines, "Ticket: "), "an invoice does not call itself a ticket");
    }

    /// **A corrective invoice says so in its title** (hub#2381). An R1–R4 prints with a negative
    /// total and, titled like any other invoice, reads at a glance as one more invoice; the A4 and
    /// the market (Holded, Odoo, Business Central) title it «Factura rectificativa». Same place,
    /// same weight as the plain title, and it fits the 80 mm line.
    #[test]
    fn a_corrective_invoice_is_titled_factura_rectificativa() {
        for (locale, expected, plain) in [
            ("es", "FACTURA RECTIFICATIVA", "FACTURA"),
            ("en", "CORRECTIVE INVOICE", "INVOICE"),
        ] {
            let mut data = full_invoice();
            data["rectifying"] = json!(true);
            data["locale"] = json!(locale);
            let lines = invoice_paper(&data);
            let title = line_index(&lines, |l| l.trim() == expected, "the corrective title");
            assert!(lines[title].1 && lines[title].2, "{locale}: bold and double height: {lines:#?}");
            assert!(lines[title].0.chars().count() <= LINE_WIDTH, "{locale}: it fits the line");
            let number = line_index(&lines, |l| l.contains("F-2026-000007"), "the invoice number");
            assert!(title < number, "{locale}: the title comes before the number");
            assert!(
                !lines.iter().any(|(l, _, _)| l.trim() == plain),
                "{locale}: the plain title is replaced, not printed twice: {lines:#?}"
            );
        }
    }

    /// Without the mark the paper does not move: the word is a statement about the document, so
    /// only an explicit `true` switches it on — like `duplicate` (hub#1931).
    #[test]
    fn only_an_explicit_true_makes_an_invoice_corrective() {
        for flag in [None, Some(json!(false)), Some(json!("yes")), Some(json!(1))] {
            let mut data = full_invoice();
            if let Some(flag) = flag.clone() {
                data["rectifying"] = flag;
            }
            let lines = invoice_paper(&data);
            assert!(lines.iter().any(|(l, _, _)| l.trim() == "FACTURA"), "{flag:?}: {lines:#?}");
            assert!(!says(&lines, "RECTIFICATIVA"), "{flag:?}: {lines:#?}");
        }
    }

    /// The customer is IDENTIFIED: name, tax id and address, together, under the document data.
    #[test]
    fn a_full_invoice_identifies_the_customer_with_their_tax_id() {
        let lines = invoice_paper(&full_invoice());
        let name = line_index(&lines, |l| l == "Cliente: Talleres Paco SL", "the customer's name");
        let tax_id = line_index(&lines, |l| l == "NIF: B12345678", "the customer's tax id");
        let address = line_index(&lines, |l| l == "Dir: Calle Mayor 3, Madrid", "the address");
        assert_eq!((tax_id, address), (name + 1, name + 2), "the three lines go together");
        let issuer = line_index(&lines, |l| l.contains("B11111111"), "the issuer's tax id");
        assert!(issuer < name, "the issuer heads the paper, the customer comes after");
    }

    /// The address is printed when it comes and its absence is not a refusal: a customer's tax id
    /// is what the law cannot do without, and the address is often not on file.
    #[test]
    fn a_full_invoice_without_a_customer_address_still_prints() {
        let mut data = full_invoice();
        data.as_object_mut().unwrap().remove("customer_address");
        let lines = invoice_paper(&data);
        assert!(says(&lines, "NIF: B12345678"));
        assert!(!says(&lines, "Dir: "), "no empty address line");
    }

    /// One row per rate with its BASE and its QUOTA, under a header naming the columns — the
    /// breakdown a full invoice carries (RD 1619/2012 art. 6.1.g/h). The amounts are right-aligned
    /// in fixed columns so the bases and the quotas read as two columns, not as prose.
    #[test]
    fn a_full_invoice_breaks_the_vat_down_per_rate_with_base_and_quota() {
        let lines = invoice_paper(&full_invoice());
        let header = line_index(&lines, |l| l.starts_with("Tipo") && l.contains("Base") && l.ends_with("Cuota"), "the header");
        let ten = line_index(&lines, |l| l.starts_with("IVA 10%"), "the 10% row");
        let twenty_one = line_index(&lines, |l| l.starts_with("IVA 21%"), "the 21% row");
        assert!(header < ten && ten < twenty_one, "header, then the rates in the order sent");
        assert_eq!(lines[ten].0, format!("{:<8}{:>12}{:>12}", "IVA 10%", "10.00", "1.00"));
        assert_eq!(lines[twenty_one].0, format!("{:<8}{:>12}{:>12}", "IVA 21%", "10.00", "2.10"));
        for row in [header, ten, twenty_one] {
            assert_eq!(lines[row].0.chars().count(), LINE_WIDTH, "a row fills the 80 mm line exactly");
        }
        let total = line_index(&lines, |l| l.starts_with("TOTAL"), "the total");
        assert!(twenty_one < total, "the breakdown comes before the total");
    }

    /// A row may name itself (a surcharge, `RE 5.2%`) and a decimal rate prints as the rate, not
    /// as `5.20`. A label longer than the rate column gets a line of its own instead of pushing
    /// the amounts out of their columns.
    #[test]
    fn a_breakdown_row_can_name_itself_and_keeps_its_columns() {
        let mut data = full_invoice();
        data["tax_breakdown"] = json!([
            { "rate": 5.2, "base": 10.0, "tax": 0.52 },
            { "rate": 1.4, "base": 10.0, "tax": 0.14, "label": "Recargo equivalencia 1.4%" }
        ]);
        let lines = invoice_paper(&data);
        assert!(says(&lines, &format!("{:<8}{:>12}{:>12}", "IVA 5.2%", "10.00", "0.52")), "{lines:#?}");
        let long = line_index(&lines, |l| l == "Recargo equivalencia 1.4%", "the long label");
        assert_eq!(lines[long + 1].0, format!("{:<8}{:>12}{:>12}", "", "10.00", "0.14"));
    }

    /// English paper, English labels (hub#1159): the invoice is not a Spanish-only document.
    #[test]
    fn a_full_invoice_speaks_the_language_of_the_paper() {
        let mut data = full_invoice();
        data["locale"] = json!("en");
        let lines = invoice_paper(&data);
        assert!(lines.iter().any(|(l, _, _)| l.trim() == "INVOICE"), "{lines:#?}");
        assert!(says(&lines, "Invoice: F-2026-000007"));
        assert!(says(&lines, "Customer: Talleres Paco SL"));
        assert!(says(&lines, "VAT: B12345678"));
        assert!(says(&lines, "VAT 21%"));
        assert!(!says(&lines, "FACTURA") && !says(&lines, "Cuota"), "no Spanish left over");
    }

    /// **An invoice missing what makes it an invoice is REFUSED, never cut as a mute ticket**
    /// (hub#2005). Before, `invoice` fell into the ticket's body and a customer who asked for their
    /// invoice took home paper without their tax id or the breakdown — the silent failure. The
    /// refusal names the missing field, so the print host's error says what to fix.
    #[test]
    fn an_invoice_missing_what_makes_it_an_invoice_is_refused_naming_the_field() {
        type Breaks = Box<dyn Fn(&mut serde_json::Value)>;
        let broken: Vec<(&str, Breaks)> = vec![
            ("vat_number", Box::new(|d| { d.as_object_mut().unwrap().remove("vat_number"); })),
            ("receipt_id", Box::new(|d| d["receipt_id"] = json!(""))),
            ("customer_name", Box::new(|d| { d.as_object_mut().unwrap().remove("customer_name"); })),
            ("customer_tax_id", Box::new(|d| { d.as_object_mut().unwrap().remove("customer_tax_id"); })),
            ("customer_tax_id", Box::new(|d| d["customer_tax_id"] = json!("  "))),
            ("tax_breakdown", Box::new(|d| { d.as_object_mut().unwrap().remove("tax_breakdown"); })),
            ("tax_breakdown", Box::new(|d| d["tax_breakdown"] = json!([]))),
            ("tax_breakdown", Box::new(|d| d["tax_breakdown"] = json!({ "21.00": { "base": 10.0, "tax": 2.1 } }))),
            ("tax_breakdown", Box::new(|d| d["tax_breakdown"] = json!([{ "rate": 21, "base": 10.0 }]))),
            ("tax_breakdown", Box::new(|d| d["tax_breakdown"] = json!([{ "rate": 21, "tax": 2.1 }]))),
            ("tax_breakdown", Box::new(|d| d["tax_breakdown"] = json!([{ "base": 10.0, "tax": 2.1 }]))),
        ];
        for (field, breaks) in broken {
            let mut data = full_invoice();
            breaks(&mut data);
            match render_document(DocumentType::Invoice, &data) {
                Err(crate::PeripheralError::InvalidPayload(msg)) => assert!(
                    msg.contains(field),
                    "the refusal names `{field}`, got: {msg}"
                ),
                other => panic!("an invoice without a valid `{field}` is refused, got {other:?}"),
            }
        }
    }

    /// The ticket does not change: the same fields on a `receipt` print no title, no customer tax
    /// id and no breakdown, and a receipt without them is still a receipt. Every till of the fleet
    /// prints this paper, so the invoice's body must not leak into it.
    #[test]
    fn a_receipt_does_not_grow_the_invoice_body() {
        let bytes = render_document(DocumentType::Receipt, &full_invoice()).expect("a receipt renders");
        let lines = lines_with_modes(&bytes);
        assert!(says(&lines, "Ticket: F-2026-000007"));
        assert!(!says(&lines, "FACTURA"), "{lines:#?}");
        assert!(!says(&lines, "B12345678") && !says(&lines, "Cuota"), "{lines:#?}");
    }

    /// **The bill the waiter takes to the table is a document this printer knows** (hub#748).
    ///
    /// It is the most frequently printed paper of a restaurant service — it comes out before every
    /// payment, so more often than the fiscal ticket — and until now `prebill` was not in the
    /// vocabulary at all, so the only door refused it and no printer could produce it.
    ///
    /// Named through `parse` and not the variant on purpose: this is the string the hub's queue
    /// stores and the drain frame carries, so the test pins the **wire**, which is the contract.
    #[test]
    fn the_bill_taken_to_the_table_is_a_document_this_printer_knows() {
        assert!(
            DocumentType::parse("prebill").is_some(),
            "`prebill` is the bill taken to the table before charging; refusing it leaves the \
             waiter with nothing to carry"
        );
    }

    /// How many QR symbols reached the paper. One symbol is FIVE `GS ( k` commands (set model,
    /// module size, error correction, store data, print), so counting the raw prefix would answer
    /// five when the honest answer is one.
    fn qr_symbols(bytes: &[u8]) -> usize {
        bytes
            .windows(3)
            .filter(|w| *w == [0x1d, 0x28, 0x6b])
            .count()
            / 5
    }

    /// **Two QRs, not one** (hub#963). The VeriFactu one points at the AEAT's `ValidarQR` and is
    /// for CHECKING; the second points at this hub and is for ASKING — «pide tu factura». They are
    /// two fields on purpose: printing one over the other would either strip the fiscal proof from
    /// the ticket or leave the customer with nothing to scan.
    #[test]
    fn a_ticket_can_carry_both_the_fiscal_qr_and_the_invoice_request_qr() {
        let doc = json!({
            "business_name": "Bar Manolo",
            "receipt_id": "T-42",
            "items": [{ "name": "Cafe", "quantity": 1, "total": 1.2 }],
            "total": 1.2,
            "qr_data": "https://prewww2.aeat.es/wlpl/TIKE-CONT/ValidarQR?nif=B1&numserie=T-42",
            "claim_note": "Pide tu factura",
            "claim_qr_data": "https://bar.erplora.com/p/ABCD1234ABCD1234",
            "claim_locator": "ABCD1234ABCD1234",
        });
        let bytes = render_document(DocumentType::parse("receipt").unwrap(), &doc)
            .expect("a well-formed ticket renders");
        let text = String::from_utf8_lossy(&bytes);

        assert_eq!(qr_symbols(&bytes), 2, "both QRs must reach the paper");
        assert!(
            text.contains("ValidarQR"),
            "the fiscal QR still points at the AEAT"
        );
        assert!(
            text.contains("/p/ABCD1234ABCD1234"),
            "and the second one at this hub"
        );
        assert!(
            text.contains("Pide tu factura"),
            "with a caption, or nobody knows what the second code is for"
        );
        assert!(
            text.contains("ABCD1234ABCD1234\n"),
            "and the locator in PLAIN TEXT below it: the camera is not always an option"
        );
    }

    /// **«VERI*FACTU» under the fiscal QR** (sales#327). RD 1619/2012 art. 6.5.b (7.5 for the
    /// simplified invoice a ticket is) wants the legend beside the QR of every invoice from a
    /// system that remits all its records, and Orden HAC/1177/2024 art. 20.1.b wants it as
    /// visible as the rest of the data. The producer sends it as `qr_legend`; the renderer prints
    /// it right under the fiscal QR — before the second QR, so it cannot be read as its caption.
    #[test]
    fn the_verifactu_legend_is_printed_right_under_the_fiscal_qr() {
        let doc = json!({
            "receipt_id": "T-44",
            "items": [{ "name": "Cafe", "quantity": 1, "total": 1.2 }],
            "total": 1.2,
            "qr_data": "https://prewww2.aeat.es/wlpl/TIKE-CONT/ValidarQR?nif=B1",
            "qr_legend": "VERI*FACTU",
            "claim_note": "Pide tu factura",
            "claim_qr_data": "https://bar.erplora.com/p/ABCD1234ABCD1234",
        });
        let bytes = render_document(DocumentType::parse("receipt").unwrap(), &doc).unwrap();
        let text = String::from_utf8_lossy(&bytes);

        let fiscal = text.find("ValidarQR").expect("the fiscal QR is printed");
        let legend = text
            .find("VERI*FACTU\n")
            .expect("the legend is printed, on its own line");
        let claim = text.find("Pide tu factura").expect("the claim block is printed");
        assert!(fiscal < legend, "the legend goes under the fiscal QR");
        assert!(legend < claim, "and before the second QR, which is not what it names");

        // «bien visible» (Orden HAC/1177/2024 art. 20.1.b): the last emphasis command before the
        // legend is ESC E 1 (bold on), so it is not printed as one more 8-px caption.
        let legend_at = bytes
            .windows(b"VERI*FACTU".len())
            .position(|w| w == b"VERI*FACTU")
            .expect("legend bytes");
        let last_bold = bytes[..legend_at]
            .windows(3)
            .rposition(|w| w[0] == 0x1b && w[1] == 0x45)
            .expect("an ESC E command precedes the legend");
        assert_eq!(bytes[last_bold + 2], 1, "the legend is printed in bold");
    }

    /// No fiscal QR → no legend, whatever the producer sends: the legend names the QR, and alone
    /// it would claim a verification the paper does not offer. The bill never carries it.
    #[test]
    fn the_verifactu_legend_needs_the_fiscal_qr() {
        let no_qr = json!({
            "receipt_id": "T-45",
            "items": [{ "name": "Cafe", "quantity": 1, "total": 1.2 }],
            "total": 1.2,
            "qr_legend": "VERI*FACTU",
        });
        let bytes = render_document(DocumentType::parse("receipt").unwrap(), &no_qr).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("VERI*FACTU"));

        let bill = json!({
            "items": [{ "name": "Cafe", "quantity": 1, "total": 1.2 }],
            "total": 1.2,
            "qr_data": "https://prewww2.aeat.es/wlpl/TIKE-CONT/ValidarQR?nif=B1",
            "qr_legend": "VERI*FACTU",
        });
        let bytes = render_document(DocumentType::parse("prebill").unwrap(), &bill).unwrap();
        assert!(
            !String::from_utf8_lossy(&bytes).contains("VERI*FACTU"),
            "a bill is not an invoice and cannot say it is verifiable"
        );
    }

    /// **«QR tributario:» above the fiscal QR, and the QR at the very top** (sales#339). The AEAT's
    /// QR specification (v0.5.0 §3, which Orden HAC/1177/2024 art. 21.1 points at) wants the QR
    /// «always preceded» by the text «QR tributario:», above it, and the QR «at the beginning of
    /// the invoice, before the content generated by the invoicing system». The producer sends the
    /// heading as `qr_heading` (a fixed legal text, not translated); the second QR stays at the foot.
    #[test]
    fn the_fiscal_qr_opens_the_ticket_under_its_qr_tributario_heading() {
        let doc = json!({
            "business_name": "Bar Manolo",
            "receipt_id": "T-46",
            "items": [{ "name": "Cafe", "quantity": 1, "total": 1.2 }],
            "total": 1.2,
            "qr_data": "https://prewww2.aeat.es/wlpl/TIKE-CONT/ValidarQR?nif=B1",
            "qr_heading": "QR tributario:",
            "qr_legend": "VERI*FACTU",
            "claim_note": "Pide tu factura",
            "claim_qr_data": "https://bar.erplora.com/p/ABCD1234ABCD1234",
        });
        let bytes = render_document(DocumentType::parse("receipt").unwrap(), &doc).unwrap();
        let text = String::from_utf8_lossy(&bytes);

        let heading = text
            .find("QR tributario:\n")
            .expect("the heading is printed, on its own line");
        let fiscal = text.find("ValidarQR").expect("the fiscal QR is printed");
        let legend = text.find("VERI*FACTU\n").expect("the legend is printed");
        let business = text.find("Bar Manolo").expect("the business name is printed");
        let items = text.find("Cafe").expect("the lines are printed");
        let claim = text.find("Pide tu factura").expect("the claim block is printed");
        assert!(heading < fiscal, "the heading goes ABOVE the fiscal QR");
        assert!(fiscal < legend, "the legend stays right under it");
        assert!(
            legend < business,
            "the fiscal block opens the ticket, before anything the system writes"
        );
        assert!(business < items && items < claim, "the second QR stays at the foot");

        // Same size as the rest of the data or bigger, never a smaller caption: printed in bold,
        // centred over the code.
        let heading_at = bytes
            .windows(b"QR tributario:".len())
            .position(|w| w == b"QR tributario:")
            .expect("heading bytes");
        let last_bold = bytes[..heading_at]
            .windows(3)
            .rposition(|w| w[0] == 0x1b && w[1] == 0x45)
            .expect("an ESC E command precedes the heading");
        assert_eq!(bytes[last_bold + 2], 1, "the heading is printed in bold");
        let last_align = bytes[..heading_at]
            .windows(3)
            .rposition(|w| w[0] == 0x1b && w[1] == 0x61)
            .expect("an ESC a command precedes the heading");
        assert_eq!(bytes[last_align + 2], 1, "the heading is centred over the QR");
    }

    /// No fiscal QR → no heading: alone it would announce a code the paper does not carry. And a
    /// producer that does not send the key yet (an older `sales`) still gets its QR at the top.
    #[test]
    fn the_qr_tributario_heading_needs_the_fiscal_qr() {
        let no_qr = json!({
            "receipt_id": "T-47",
            "items": [{ "name": "Cafe", "quantity": 1, "total": 1.2 }],
            "total": 1.2,
            "qr_heading": "QR tributario:",
        });
        let bytes = render_document(DocumentType::parse("receipt").unwrap(), &no_qr).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("QR tributario:"));

        let bill = json!({
            "items": [{ "name": "Cafe", "quantity": 1, "total": 1.2 }],
            "total": 1.2,
            "qr_data": "https://prewww2.aeat.es/wlpl/TIKE-CONT/ValidarQR?nif=B1",
            "qr_heading": "QR tributario:",
        });
        let bytes = render_document(DocumentType::parse("prebill").unwrap(), &bill).unwrap();
        assert!(
            !String::from_utf8_lossy(&bytes).contains("QR tributario:"),
            "a bill carries no fiscal QR, so no heading either"
        );

        let old_producer = json!({
            "business_name": "Bar Manolo",
            "receipt_id": "T-48",
            "items": [{ "name": "Cafe", "quantity": 1, "total": 1.2 }],
            "total": 1.2,
            "qr_data": "https://prewww2.aeat.es/wlpl/TIKE-CONT/ValidarQR?nif=B1",
        });
        let bytes = render_document(DocumentType::parse("receipt").unwrap(), &old_producer).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            text.find("ValidarQR").unwrap() < text.find("Bar Manolo").unwrap(),
            "the fiscal QR opens the ticket with or without the heading key"
        );
    }

    /// A ticket with no claim prints exactly what it printed before. The second QR is opt-in, so
    /// no hub that has not adopted this grows a blank line or a stray caption.
    #[test]
    fn a_ticket_without_a_claim_is_unchanged() {
        let doc = json!({
            "receipt_id": "T-43",
            "items": [{ "name": "Cafe", "quantity": 1, "total": 1.2 }],
            "total": 1.2,
            "qr_data": "https://prewww2.aeat.es/wlpl/TIKE-CONT/ValidarQR?nif=B1",
        });
        let bytes = render_document(DocumentType::parse("receipt").unwrap(), &doc).unwrap();
        assert_eq!(qr_symbols(&bytes), 1, "only the fiscal QR");
    }

    /// **The bill still carries no QR of either kind.** The pre-bill drops the fiscal one because
    /// there is no billing record yet — and for exactly the same reason there is no invoice to
    /// request: a claim on a sale that has not been charged would point at nothing.
    #[test]
    fn the_bill_carries_neither_qr() {
        let doc = json!({
            "items": [{ "name": "Cafe", "quantity": 1, "total": 1.2 }],
            "total": 1.2,
            "qr_data": "https://prewww2.aeat.es/wlpl/TIKE-CONT/ValidarQR?nif=B1",
            "claim_qr_data": "https://bar.erplora.com/p/ABCD1234ABCD1234",
            "claim_locator": "ABCD1234ABCD1234",
        });
        let bytes = render_document(DocumentType::parse("prebill").unwrap(), &doc).unwrap();
        assert!(
            !bytes.windows(3).any(|w| w == [0x1d, 0x28, 0x6b]),
            "a bill carries no QR at all"
        );
        assert!(
            !String::from_utf8_lossy(&bytes).contains("ABCD1234ABCD1234"),
            "nor the locator in text"
        );
    }

    /// **The bill is NOT a ticket, and the paper has to say so** (ADR-0141).
    ///
    /// A bill with a series number, a payment method or a VeriFactu QR is a piece of paper that
    /// looks like an invoice without being one, which is a legal problem rather than a cosmetic
    /// one: the numbering is consumed when charging, not before, and there is no billing record
    /// yet. So the renderer must print the lines and the total — the waiter has to be able to
    /// collect — and must **not** print any of the fiscal furniture, whatever the producer sends.
    #[test]
    fn the_bill_prints_its_lines_but_none_of_the_fiscal_furniture() {
        let doc = json!({
            "business_name": "Bar Manolo",
            "customer_name": "Mesa 4",
            "items": [
                { "name": "Cafe", "quantity": 2, "total": 2.4 },
                { "name": "Tostada", "quantity": 1, "total": 3.5 },
            ],
            "total": 5.9,
            "notice": "Cuenta - no es una factura. El tiquet fiscal se entrega al cobrar.",
            // Sent on purpose: even if a producer hands over fiscal fields, a bill must not print
            // them. Dropping them here is cheaper than trusting every caller to omit them.
            "receipt_id": "T-42",
            "payment_method": "Efectivo",
            "qr_data": "https://prevalidacion.aeat.es/verifactu",
        });
        let bytes = render_document(
            DocumentType::parse("prebill").expect("`prebill` is a document this printer knows"),
            &doc,
        )
        .expect("a well-formed bill renders");
        let text = String::from_utf8_lossy(&bytes);

        assert!(text.contains("Bar Manolo"), "the business name is on the paper");
        assert!(text.contains("Mesa 4"), "the table is on the paper: it is what identifies the bill");
        assert!(text.contains("2x Cafe"), "the lines are on the paper");
        assert!(text.contains("TOTAL"), "the total is on the paper — it is what gets collected");
        assert!(
            text.contains("no es una factura"),
            "the notice is PRINTED: paper that looks like an invoice without being one is a legal \
             problem, not a cosmetic one"
        );

        assert!(!text.contains("T-42"), "a bill carries no series number: numbering is consumed on payment");
        assert!(!text.contains("Efectivo"), "a bill carries no payment method: nothing has been charged yet");
        assert!(
            !bytes.windows(3).any(|w| w == [0x1d, 0x28, 0x6b]),
            "a bill carries no VeriFactu QR: there is no billing record to point at yet"
        );
        assert_eq!(
            &bytes[bytes.len() - 3..],
            &[0x1d, 0x56, 0x01],
            "and the paper is cut, or the bill comes out attached to the next one"
        );
    }

    /// **A bill with no notice still says it is not an invoice.** The producer sends the translated
    /// text (`ui.prebillNotice`), but the guarantee cannot depend on every producer remembering:
    /// what is at stake is handing a customer a paper that passes for an invoice.
    #[test]
    fn a_bill_without_a_notice_still_says_it_is_not_an_invoice() {
        let bytes = render_document(
            DocumentType::parse("prebill").expect("`prebill` is a document this printer knows"),
            &json!({ "items": [{ "name": "Cafe", "quantity": 1, "total": 1.2 }], "total": 1.2 }),
        )
        .expect("a bill with no notice still renders");
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            text.contains("no es una factura"),
            "the notice has a default: a producer that forgets it must not be able to print a \
             paper that passes for an invoice"
        );
    }

    /// **A bill with no lines is REFUSED, not cut blank.**
    ///
    /// This is the guard that stops the producer's mistake from becoming paper. The renderer reads
    /// by key, so a document in the SCREEN's shape (`business.name`, `lines[]` — what `ok-receipt`
    /// paints) finds none of the fields it wants, takes every default and hands the customer
    /// «ERPlora», no lines and TOTAL 0.00. Paper that comes out WRONG is worse than paper that does
    /// not come out: nobody checks a bill that printed.
    ///
    /// It matters beyond a typo. The module that produces the bill is installed at its **published**
    /// version, so between the hub taking this change and the module being republished, the old
    /// producer is what runs — and it sends exactly that shape (ERPlora/sales#78). Failing loudly
    /// puts the reason in `_print_queue.last_error` instead of in the customer's hand.
    #[test]
    fn a_bill_with_no_lines_is_refused_instead_of_cut_blank() {
        // The shape `<ok-receipt>` paints on screen: right document, wrong keys.
        let screen_shape = json!({
            "business": { "name": "Bar Manolo" },
            "lines": [{ "name": "Cafe", "qty": 2, "total": 2.4 }],
            "total": 2.4,
        });
        let err = render_document(
            DocumentType::parse("prebill").expect("`prebill` is a document this printer knows"),
            &screen_shape,
        )
        .expect_err("a bill with no lines the renderer can read is not a bill");
        assert!(
            matches!(err, crate::PeripheralError::InvalidPayload(_)),
            "the refusal says the document is wrong, not the printer: {err}"
        );
    }

    /// **Typographic punctuation reaches the paper as punctuation, not as `?`.**
    ///
    /// The printer speaks cp437 and the encoder replaces what it cannot represent with `?`, so the
    /// translated notice — «Cuenta — no es una factura» with a real em dash — came out as
    /// «Cuenta ? no es una factura». The text is written by translators in a text editor, so the
    /// dashes and curly quotes are not a mistake anybody is going to stop making; the encoder is
    /// the place that has to cope.
    #[test]
    fn typographic_punctuation_reaches_the_paper_as_punctuation() {
        let mut b = EscposBuilder::new();
        b.text("Cuenta — no es una factura: “Bar Manolo’s”…\n");
        let text = String::from_utf8_lossy(&b.finish()).to_string();
        assert!(
            !text.contains('?'),
            "nothing on the paper turned into a question mark, got: {text}"
        );
        assert!(text.contains("Cuenta - no es una factura"), "the em dash prints as a dash: {text}");
        assert!(text.contains("\"Bar Manolo's\""), "quotes print as quotes: {text}");
        assert!(text.contains("..."), "the ellipsis prints as three dots: {text}");
    }

    // ── hub#1138 · the supplements arrive as a LIST, one line each ──────────────────────────────
    //
    // `kitchen` composes them today into a single string (`modifiers_for_display`, joined with
    // «, ») because the renderer only ever read a string. An ARRAY in the same key is truthy but
    // is not a string, so `str_field` hands back `""`: the supplements vanish from the paper
    // WITHOUT an error — exactly the silent drop sales#78 warns about. Both shapes must print.

    /// An array prints ONE indented line per supplement — the point of the issue.
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

    /// A supplement is a CHANGE, so it keeps the emphasis hub#1156 gave it (ADR-0394): the list
    /// must not quietly demote it to normal weight.
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

    /// The string `kitchen` sends TODAY prints byte for byte as it does today: the fleet that has
    /// not moved yet must not see its chits change.
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

    // ── hub#1159 · the paper speaks the language the document asks for ──────────────────────────
    //
    // Every label in this file was wired in Spanish, so a hub running in another language painted
    // a translated KDS and pushed a Spanish chit out of the thermal printer. English is the source
    // (ADR-0055/0199) and `es` reproduces, byte for byte, the paper the fleet prints today: a
    // document WITHOUT `locale` — every deployed `erplora-app` — must not move a single column.

    /// Renders any document and returns the paper without control bytes.
    fn printed(doc: DocumentType, data: &serde_json::Value) -> String {
        strip_escpos(&render_document(doc, data).expect("a valid document"))
    }

    /// The kitchen chit in English: the labels this printer owns are translated, the values the
    /// producer sends are not touched.
    #[test]
    fn the_kitchen_chit_can_be_printed_in_english() {
        let text = printed(
            DocumentType::KitchenOrder,
            &json!({
                "locale": "en",
                "table": "4", "waiter": "Ana", "round_number": 2, "priority": "HIGH",
                "items": [{ "name": "Burger", "quantity": 1 }],
            }),
        );
        for wanted in ["KITCHEN", "Table: 4", "Waiter: Ana", "Round 2", "Time: ", "!! RUSH !!"] {
            assert!(text.contains(wanted), "the chit says {wanted:?}:\n{text}");
        }
        for spanish in ["COCINA", "Mesa:", "Camarero:", "Ronda", "Hora:", "URGENTE"] {
            assert!(!text.contains(spanish), "and nothing is left in Spanish ({spanish:?}):\n{text}");
        }
    }

    /// The customer's ticket in English — the surface the customer actually holds.
    #[test]
    fn the_ticket_can_be_printed_in_english() {
        let text = printed(
            DocumentType::Receipt,
            &json!({
                "locale": "en",
                "receipt_id": "T-42", "cashier": "Ana", "customer_name": "Bob",
                "vat_number": "B123", "phone": "600",
                "items": [{ "name": "Coffee", "quantity": 1, "total": 1.8 }],
                "subtotal": 1.5, "tax_amount": 0.3, "discount": 0.2,
                "total": 1.8, "payment_method": "Card", "paid": 2.0, "change": 0.2,
            }),
        );
        for wanted in [
            "VAT: B123", "Ticket: T-42", "Date: ", "Cashier: Ana", "Customer: Bob",
            "Subtotal", "VAT", "Discount", "TOTAL", "Payment: Card", "Tendered", "Change",
            "Thank you for your purchase",
        ] {
            assert!(text.contains(wanted), "the ticket says {wanted:?}:\n{text}");
        }
        for spanish in ["NIF:", "Fecha:", "Cajero:", "Cliente:", "Descuento", "Pago:", "Entregado", "Cambio", "Gracias"] {
            assert!(!text.contains(spanish), "and nothing is left in Spanish ({spanish:?}):\n{text}");
        }
    }

    /// The bill in English, DEFAULT NOTICE INCLUDED: the notice is what keeps this paper from
    /// passing for an invoice, so a producer that forgets to send it still gets it in the
    /// customer's language rather than in Spanish.
    #[test]
    fn the_bill_and_its_default_notice_can_be_printed_in_english() {
        let text = printed(
            DocumentType::Prebill,
            &json!({
                "locale": "en",
                "customer_name": "Table 4",
                "items": [{ "name": "Coffee", "quantity": 1, "total": 1.8 }],
                "total": 1.8,
            }),
        );
        assert!(text.contains("BILL"), "the title:\n{text}");
        assert!(text.contains("Table/Customer: Table 4"), "the table:\n{text}");
        assert!(text.contains("not an invoice"), "the notice is in English too:\n{text}");
        for spanish in ["CUENTA", "Mesa/Cliente:", "no es una factura"] {
            assert!(!text.contains(spanish), "nothing left in Spanish ({spanish:?}):\n{text}");
        }
    }

    /// The delivery note and the cash-up report are labels of this file too.
    #[test]
    fn the_delivery_note_and_the_cash_report_can_be_printed_in_english() {
        let note = printed(
            DocumentType::DeliveryNote,
            &json!({
                "locale": "en", "receipt_id": "A-1",
                "customer_name": "Bob", "delivery_address": "Main St 1",
                "items": [{ "name": "Coffee", "quantity": 1 }],
            }),
        );
        for wanted in ["DELIVERY NOTE", "No: A-1", "Date: ", "Customer: Bob", "Addr: Main St 1", "Signature"] {
            assert!(note.contains(wanted), "the delivery note says {wanted:?}:\n{note}");
        }
        let report = printed(
            DocumentType::CashSessionReport,
            &json!({ "locale": "en", "receipt_id": "S-1", "cashier": "Ana",
                     "opening_balance": 100.0, "closing_balance": 150.0 }),
        );
        for wanted in ["CASH REPORT", "Session: S-1", "Cashier: Ana", "Opening", "Closing", "Difference"] {
            assert!(report.contains(wanted), "the cash report says {wanted:?}:\n{report}");
        }
    }

    /// 🔴 The compatibility guarantee: a document WITHOUT `locale` prints byte for byte what it
    /// printed before this change, and `locale: "es"` prints the very same bytes. Every deployed
    /// `erplora-app` sends no locale, so if this moves, every kitchen in the fleet notices.
    #[test]
    fn a_document_without_a_locale_prints_exactly_the_spanish_paper_of_today() {
        let cases: Vec<(DocumentType, serde_json::Value, Vec<&str>)> = vec![
            (
                DocumentType::KitchenOrder,
                json!({ "table": "4", "waiter": "Ana", "round_number": 2, "priority": "HIGH",
                        "items": [{ "name": "Cafe", "quantity": 1 }] }),
                vec!["COCINA", "Mesa: 4", "Camarero: Ana", "Ronda 2", "Hora: ", "!! URGENTE !!"],
            ),
            (
                DocumentType::Receipt,
                json!({ "receipt_id": "T-42", "cashier": "Ana", "customer_name": "Bob",
                        "vat_number": "B1", "phone": "600", "discount": 0.2, "paid": 2.0,
                        "change": 0.2, "payment_method": "Tarjeta", "subtotal": 1.5,
                        "tax_amount": 0.3, "total": 1.8,
                        "items": [{ "name": "Cafe", "quantity": 1, "total": 1.8 }] }),
                vec!["NIF: B1", "Ticket: T-42", "Fecha: ", "Cajero: Ana", "Cliente: Bob",
                     "IVA", "Descuento", "TOTAL", "Pago: Tarjeta", "Entregado", "Cambio",
                     "Gracias por su compra"],
            ),
            (
                DocumentType::Prebill,
                json!({ "customer_name": "Mesa 4", "total": 1.8,
                        "items": [{ "name": "Cafe", "quantity": 1, "total": 1.8 }] }),
                // Asserted in pieces: the notice is wrapped to 32 columns, so the whole
                // sentence never lives on one line.
                vec!["CUENTA", "Mesa/Cliente: Mesa 4", "no es una factura", "tiquet fiscal"],
            ),
            (
                DocumentType::DeliveryNote,
                json!({ "receipt_id": "A-1", "customer_name": "Bob", "delivery_address": "Calle 1",
                        "items": [{ "name": "Cafe", "quantity": 1 }] }),
                vec!["ALBARAN", "N: A-1", "Cliente: Bob", "Dir: Calle 1", "Firma: _______________"],
            ),
            (
                DocumentType::CashSessionReport,
                json!({ "receipt_id": "S-1", "cashier": "Ana",
                        "opening_balance": 100.0, "closing_balance": 150.0 }),
                vec!["CIERRE DE CAJA", "Sesion: S-1", "Cajero: Ana", "Apertura", "Cierre",
                     "Diferencia"],
            ),
        ];
        for (doc, data, spanish) in cases {
            let text = printed(doc, &data);
            for wanted in &spanish {
                assert!(text.contains(wanted), "{doc:?} still says {wanted:?}:\n{text}");
            }
            // …and asking for `es` explicitly is the same paper, byte for byte.
            let mut with_es = data.clone();
            with_es["locale"] = json!("es");
            assert_eq!(
                render_document(doc, &data).expect("valid"),
                render_document(doc, &with_es).expect("valid"),
                "{doc:?}: `locale: es` is the default paper, byte for byte",
            );
        }
    }

    /// A language this printer does not carry falls back to the paper of today instead of
    /// printing an empty label or refusing the job: paper that does not come out loses a service.
    #[test]
    fn an_unknown_language_falls_back_to_the_paper_of_today() {
        let base = json!({ "items": [{ "name": "Cafe", "quantity": 1 }], "waiter": "Ana" });
        let today = render_document(DocumentType::KitchenOrder, &base).expect("valid");
        for odd in [json!("fr"), json!("zz-ZZ"), json!(42), json!(null)] {
            let mut data = base.clone();
            data["locale"] = odd.clone();
            assert_eq!(
                render_document(DocumentType::KitchenOrder, &data).expect("valid"),
                today,
                "`locale: {odd}` prints the default paper",
            );
        }
        // A region is not a language: `es-ES`/`en_GB` are the language before the separator.
        let mut english = base.clone();
        english["locale"] = json!("en-GB");
        assert!(
            strip_escpos(&render_document(DocumentType::KitchenOrder, &english).expect("valid"))
                .contains("KITCHEN"),
            "the region is dropped and the language honoured",
        );
    }


    // ── hub#1138 · the SUPPLEMENT on the customer's ticket and on the bill ───────────────────────
    //
    // The chit already prints supplements one per line (hub#1156). The ticket and the bill still
    // read only `notes`, where `sales` chains them with « · » — so with two or three the line wraps
    // at 32 columns and the continuation starts at the margin, reading as another article. Same
    // complaint hub#1165 fixed for the menu's `components`, and it is fixed the same way: the
    // structured list wins and the joined note steps aside, so nothing is said twice.

    /// One indented line per supplement, and NO amount of its own: in ERPlora the delta is already
    /// inside the line's `unit_price` (`authoritative_modifiers`), so a second number in that
    /// column would not add up with the rest — the Odoo bug the issue cites.
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

    /// With `modifiers` present the joined `notes` is NOT printed: until `sales` separates them,
    /// the note still carries the same supplements chained with « · », and printing both would say
    /// everything twice — the precedence the issue asks to decide.
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

    /// `locale` is a field of the ENVELOPE, not a line of the document: the generic renderer
    /// prints every unknown key, so without this it would print «locale: en» on the paper.
    #[test]
    fn the_locale_is_never_printed_as_a_line_of_the_document() {
        let text = printed(
            DocumentType::Generic,
            &json!({ "title": "Aviso", "locale": "en", "cosa": "valor" }),
        );
        assert!(text.contains("cosa: valor"), "the real fields still print:\n{text}");
        assert!(!text.contains("locale"), "the envelope field is not a line:\n{text}");
    }

    // ── The paper is signed by a product that EXISTS (hub#1735) ─────────────────────────────────

    /// **The test page used to sign itself «ERPlora Bridge»** — the standalone daemon ADR-0196
    /// retired and hub#340 deleted from the tree. It is the one sheet a shop owner prints on
    /// purpose, to check the printer answers, so the dead name went straight into their hands on
    /// paper: nothing corrects a sheet once it is cut. It now signs with the same name the receipt
    /// next to it falls back to (`render_receipt`) — the product they actually installed.
    #[test]
    fn the_test_page_is_signed_with_a_product_that_still_exists() {
        // An EMPTY document: the sheet an `erplora-app` prints when whatever asked for it is older
        // than this binary and sends no envelope (hub#1803). That is the case this guard is for —
        // with a `business_name` the header is the shop's, which the next test pins.
        let paper = strip_escpos(&render_test_page("network:10.0.2.2:9100", &json!({})));
        assert!(
            paper.lines().any(|l| l.trim() == "ERPlora"),
            "the sheet is signed by the product that printed it:\n{paper}"
        );
        // Everything the sheet is FOR stays on it: a header fix that quietly dropped the outcome
        // or the printer's address would satisfy the line above and lose the diagnosis, which is
        // the only reason anybody prints this page. The wording is now the catalogue's (hub#1803),
        // so the assertion follows the fallback language rather than the retired English literal.
        assert!(
            paper.contains("Prueba de impresion correcta"),
            "the outcome is still printed:\n{paper}"
        );
        assert!(
            paper.contains("network:10.0.2.2:9100"),
            "the sheet still names the printer it came out of:\n{paper}"
        );
    }

    // ── The test page speaks the hub's language and carries its name (hub#1803) ──────────────────

    /// **The one sheet that was never translated.**
    ///
    /// Every other document this crate renders takes its language off the envelope
    /// ([`Locale::from_document`], hub#1159); the test page could not, because it was rendered
    /// from a printer id with NO document behind it. So a Spanish salon pressed «Probar» and got
    /// «Test Print OK» out of the printer while the ticket beside it came out in Spanish. English
    /// is the SOURCE language, not the paper's (ADR-0055/0199) — a sheet hardcoded in either
    /// language is what the rule forbids.
    #[test]
    fn the_test_page_is_printed_in_the_language_the_document_carries() {
        let english = strip_escpos(&render_test_page(
            "network:10.0.2.2:9100",
            &json!({ "locale": "en" }),
        ));
        assert!(
            english.contains("Test Print OK"),
            "an English hub gets the English outcome:\n{english}"
        );
        assert!(
            english.contains("Printer: network:10.0.2.2:9100"),
            "and the English label for the printer it came out of:\n{english}"
        );

        let spanish = strip_escpos(&render_test_page(
            "network:10.0.2.2:9100",
            &json!({ "locale": "es" }),
        ));
        assert!(
            spanish.contains("Prueba de impresion correcta"),
            "a Spanish hub gets the Spanish outcome:\n{spanish}"
        );
        assert!(
            spanish.contains("Impresora: network:10.0.2.2:9100"),
            "and the Spanish label for the printer it came out of:\n{spanish}"
        );

        // The point of the whole change: the two sheets are NOT the same paper. A translation that
        // resolved both keys to the same literal would satisfy every assertion above one by one.
        assert_ne!(english, spanish, "the sheet actually changes with the language");
    }

    /// The region is dropped like everywhere else — `es-ES` is Spanish, `en_GB` is English.
    ///
    /// Worth its own case because the shell hands out full BCP-47 tags (`es-ES`), not bare
    /// languages: a test page that only understood `es` would be English for every real hub.
    #[test]
    fn the_test_page_reads_a_language_tag_with_its_region() {
        let spanish = strip_escpos(&render_test_page("usb:001:002", &json!({ "locale": "es-ES" })));
        assert!(
            spanish.contains("Prueba de impresion correcta"),
            "`es-ES` is Spanish:\n{spanish}"
        );
        let english = strip_escpos(&render_test_page("usb:001:002", &json!({ "locale": "en_GB" })));
        assert!(english.contains("Test Print OK"), "`en_GB` is English:\n{english}");
    }

    /// **Headed by the business, like the ticket next to it** (hub#1803).
    ///
    /// hub#1735 replaced a dead product name («ERPlora Bridge») with a live one, which was the
    /// bug of the day but left the sheet signed by the SOFTWARE. The shop owner holding it wants
    /// to know which of their printers answered, and the name they recognise is their own — the
    /// receipt has headed itself that way all along (`render_receipt`), from the very same
    /// `business_name` field.
    #[test]
    fn the_test_page_is_headed_by_the_business_that_printed_it() {
        let paper = strip_escpos(&render_test_page(
            "network:10.0.2.2:9100",
            &json!({ "business_name": "SALON AURORA SL", "locale": "es" }),
        ));
        assert!(
            paper.lines().any(|l| l.trim() == "SALON AURORA SL"),
            "the shop's own name heads the sheet:\n{paper}"
        );
        assert!(
            !paper.lines().any(|l| l.trim() == "ERPlora"),
            "and it replaces the product name rather than being added under it:\n{paper}"
        );
        // The diagnosis survives the new header, same as above.
        assert!(
            paper.contains("network:10.0.2.2:9100"),
            "the sheet still names the printer it came out of:\n{paper}"
        );
    }

    /// **No paper this crate renders names the retired Bridge**, not just the test page — the
    /// guard that keeps the dead product from coming back through another renderer (hub#1735).
    ///
    /// It reads the RAW bytes rather than the stripped paper on purpose: a receipt carries a QR
    /// (`GS ( k`), a command of variable length that [`strip_escpos`] refuses by design, so a
    /// guard written on the stripped text could not cover every document — and the document it
    /// could not cover is the one the customer takes home.
    #[test]
    fn no_paper_this_crate_renders_names_the_retired_bridge() {
        // A single document that every renderer accepts: `Prebill` is the strict one (it refuses
        // a bill with no `items`), the rest read what they know and ignore the rest.
        let data = json!({
            "business_name": "SALON AURORA SL",
            "tax_id": "12345678Z",
            // What an invoice cannot be printed without (hub#2005).
            "vat_number": "B11111111",
            "receipt_id": "F-1",
            "customer_name": "Cliente SL",
            "customer_tax_id": "B12345678",
            "tax_breakdown": [{ "rate": 21, "base": 9.92, "tax": 2.08 }],
            "ticket_number": "TICKET-2026-000001",
            "title": "Aviso",
            "items": [{ "name": "Corte", "quantity": 1, "total": 12.0 }],
            "total": 12.0,
        });
        let mut papers: Vec<(String, Vec<u8>)> = Vec::new();
        for doc in [
            DocumentType::Receipt,
            DocumentType::KitchenOrder,
            DocumentType::Invoice,
            DocumentType::DeliveryNote,
            DocumentType::BarcodeLabel,
            DocumentType::CashSessionReport,
            DocumentType::Prebill,
            DocumentType::Generic,
        ] {
            papers.push((
                format!("{doc:?}"),
                render_document(doc, &data).expect("a valid document"),
            ));
        }
        papers.push((
            "test page".to_string(),
            render_test_page("network:10.0.2.2:9100", &json!({})),
        ));
        for (what, bytes) in papers {
            assert!(
                !bytes.windows(b"Bridge".len()).any(|w| w == b"Bridge"),
                "{what}: the paper names the Bridge, a product retired by ADR-0196 and deleted \
                 from the tree by hub#340"
            );
        }
    }

    const PROMO_URL: &str = "https://g.page/r/bar-manolo/review";

    fn ticket_with_promo() -> serde_json::Value {
        json!({
            "business_name": "Bar Manolo",
            "receipt_id": "T-44",
            "items": [{ "name": "Cafe", "quantity": 1, "total": 1.2 }],
            "total": 1.2,
            "qr_data": "https://prewww2.aeat.es/wlpl/TIKE-CONT/ValidarQR?nif=B1",
            "claim_note": "Pide tu factura",
            "claim_qr_data": "https://bar.erplora.com/p/ABCD1234ABCD1234",
            "claim_locator": "ABCD1234ABCD1234",
            "receipt_footer": "Vuelva pronto",
            "promo_qr": PROMO_URL,
            "promo_note": "Escanea y dejanos una resena",
        })
    }

    fn position(bytes: &[u8], needle: &str) -> usize {
        bytes
            .windows(needle.len())
            .position(|w| w == needle.as_bytes())
            .unwrap_or_else(|| panic!("`{needle}` is not on the paper"))
    }

    /// The module size (`GS ( k … 49 67 n`, fn 167) of every QR on the paper, in printing order.
    fn qr_module_sizes(bytes: &[u8]) -> Vec<u8> {
        bytes
            .windows(8)
            .filter(|w| w[..7] == [0x1d, 0x28, 0x6b, 3, 0, 49, 67])
            .map(|w| w[7])
            .collect()
    }

    /// **The business's promotional QR reaches the thermal paper** (hub#2009, sale of sales#345).
    /// The screen (`<ok-receipt>`) and the browser paper already carry it; the roll printed nothing
    /// and the business believed its ticket did.
    #[test]
    fn a_ticket_prints_the_promotional_qr_with_its_note() {
        let bytes =
            render_document(DocumentType::parse("receipt").unwrap(), &ticket_with_promo()).unwrap();
        assert_eq!(qr_symbols(&bytes), 3, "fiscal + «pide tu factura» + promotional");
        position(&bytes, PROMO_URL);
        assert!(
            position(&bytes, "Escanea y dejanos una resena") < position(&bytes, PROMO_URL),
            "the note goes ABOVE its QR, or nobody knows what the code is for"
        );
    }

    /// Where it goes: it CLOSES the paper, after the fiscal QR, after «pide tu factura» and after
    /// the footer and the thanks — the same order as the screen and the browser paper.
    #[test]
    fn the_promotional_qr_closes_the_ticket() {
        let bytes =
            render_document(DocumentType::parse("receipt").unwrap(), &ticket_with_promo()).unwrap();
        let note = position(&bytes, "Escanea y dejanos una resena");
        assert!(note > position(&bytes, "ValidarQR"), "after the fiscal QR");
        assert!(note > position(&bytes, "/p/ABCD1234ABCD1234"), "after «pide tu factura»");
        assert!(note > position(&bytes, "Vuelva pronto"), "after the footer");
        assert!(note > position(&bytes, "Gracias por su compra"), "after the thanks");
        let cut = bytes
            .windows(2)
            .rposition(|w| w == [0x1d, 0x56])
            .expect("the ticket is cut");
        assert!(position(&bytes, PROMO_URL) < cut, "and before the cut");
    }

    /// Smaller than the fiscal QR, like `<ok-receipt>` draws it: the fiscal proof is the code
    /// the paper is for, the promotion is an extra.
    #[test]
    fn the_promotional_qr_is_smaller_than_the_fiscal_one() {
        let bytes =
            render_document(DocumentType::parse("receipt").unwrap(), &ticket_with_promo()).unwrap();
        let sizes = qr_module_sizes(&bytes);
        assert_eq!(sizes.len(), 3, "{sizes:?}");
        assert!(sizes[2] < sizes[0], "promotional {sizes:?} not smaller than the fiscal");
        assert!(sizes[2] > 0, "a zero module size prints nothing");
    }

    /// No URL, no promotion: a note alone names nothing, and a ticket of a business that did not
    /// configure it prints byte for byte what it printed before.
    #[test]
    fn a_promotional_note_without_its_qr_prints_nothing() {
        let mut with_note = ticket_with_promo();
        with_note.as_object_mut().unwrap().remove("promo_qr");
        let mut without = with_note.clone();
        without.as_object_mut().unwrap().remove("promo_note");
        let receipt = DocumentType::parse("receipt").unwrap();
        assert_eq!(
            render_document(receipt, &with_note).unwrap(),
            render_document(receipt, &without).unwrap()
        );
    }

    /// The full invoice is formal: no promotion on it, like the A4 (sales#345). A producer that
    /// builds the invoice from its ticket (sales `saleToInvoicePrintDocument`) must not smuggle
    /// the review link onto an invoice.
    #[test]
    fn a_full_invoice_carries_no_promotional_qr() {
        let mut doc = full_invoice();
        doc["promo_qr"] = json!(PROMO_URL);
        doc["promo_note"] = json!("Escanea y dejanos una resena");
        let bytes = render_document(DocumentType::parse("invoice").unwrap(), &doc).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains(PROMO_URL), "no promotional QR on an invoice");
        assert!(!text.contains("Escanea"), "nor its note");
    }

    /// The module size of every QR on the paper is a contract, not a relation (rv-2012): the fiscal
    /// QR and «pide tu factura» keep the 4 dots (0.5 mm at 203 dpi) they printed at before hub#2009,
    /// and the promotional one prints at 3. «Smaller than the fiscal» alone lets the fiscal one drift
    /// to 5 or the promotional one to 2 (0.25 mm, which a phone no longer reads) without a test going red.
    #[test]
    fn every_qr_prints_at_its_contracted_module_size() {
        let receipt = DocumentType::parse("receipt").unwrap();
        let with_promo = render_document(receipt, &ticket_with_promo()).unwrap();
        assert_eq!(qr_module_sizes(&with_promo), vec![4, 4, 3], "fiscal, claim, promotional");
        let mut without = ticket_with_promo();
        without.as_object_mut().unwrap().remove("promo_qr");
        let bytes = render_document(receipt, &without).unwrap();
        assert_eq!(qr_module_sizes(&bytes), vec![4, 4], "byte for byte what it printed before");
    }

    // ── The paper prints money in the currency's own scale (hub#2129) ──────────────────────────

    /// The paper's text lines, whatever the document type.
    fn money_paper(doc: DocumentType, data: &serde_json::Value) -> Vec<String> {
        let bytes = render_document(doc, data).expect("a valid document");
        lines_with_modes(&bytes).into_iter().map(|(text, _, _)| text).collect()
    }

    /// The line that starts with `label`, trimmed — the amount is what is right of it.
    fn line_of<'a>(lines: &'a [String], label: &str) -> &'a str {
        lines
            .iter()
            .map(|l| l.trim())
            .find(|l| l.starts_with(label))
            .unwrap_or_else(|| panic!("no line starting with {label:?} in {lines:#?}"))
    }

    fn yen_ticket() -> serde_json::Value {
        json!({
            "business_name": "Sushi Tanaka",
            "decimals": 0,
            "items": [{ "name": "Nigiri", "quantity": 2, "total": 1500.0 }],
            "subtotal": 1364.0,
            "tax_amount": 136.0,
            "discount": 100.0,
            "total": 1400.0,
            "payment_method": "Efectivo",
            "paid": 2000.0,
            "change": 600.0,
        })
    }

    /// **A yen shop's ticket has no cents** (hub#2129): `sales` sends the amounts already divided
    /// by the currency's scale and says which scale it is; the paper used to add «.00» to every
    /// one of them while the screen and the HTML ticket showed «1500».
    #[test]
    fn a_yen_ticket_prints_every_amount_without_decimals() {
        let lines = money_paper(DocumentType::Receipt, &yen_ticket());
        assert!(!lines.iter().any(|l| l.contains(".00")), "{lines:#?}");
        assert!(line_of(&lines, "2x Nigiri").ends_with(" 1500"), "{lines:#?}");
        assert!(line_of(&lines, "TOTAL").ends_with(" 1400"), "{lines:#?}");
        assert!(line_of(&lines, "Subtotal").ends_with(" 1364"), "{lines:#?}");
        assert!(line_of(&lines, "Descuento").ends_with(" -100"), "{lines:#?}");
        assert!(line_of(&lines, "Entregado").ends_with(" 2000"), "{lines:#?}");
        assert!(line_of(&lines, "Cambio").ends_with(" 600"), "{lines:#?}");
    }

    /// The bill the waiter takes to the table, same scale.
    #[test]
    fn a_yen_bill_prints_every_amount_without_decimals() {
        let lines = money_paper(DocumentType::Prebill, &yen_ticket());
        assert!(!lines.iter().any(|l| l.contains(".00")), "{lines:#?}");
        assert!(line_of(&lines, "2x Nigiri").ends_with(" 1500"), "{lines:#?}");
        assert!(line_of(&lines, "TOTAL").ends_with(" 1400"), "{lines:#?}");
    }

    /// **A dinar has three decimals, and the third one is money** (hub#2129): `1.234` KWD used to
    /// print as `1.23`, a fil lost on every amount — the VAT breakdown included.
    #[test]
    fn a_dinar_invoice_keeps_its_third_decimal_on_every_amount() {
        let mut data = full_invoice();
        data["decimals"] = json!(3);
        data["items"] = json!([{ "name": "Menu", "quantity": 1, "total": 1.234 }]);
        data["tax_breakdown"] = json!([{ "rate": 10, "base": 1.122, "tax": 0.112 }]);
        data["subtotal"] = json!(1.122);
        data["tax_amount"] = json!(0.112);
        data["total"] = json!(1.234);
        let lines = money_paper(DocumentType::Invoice, &data);
        assert!(line_of(&lines, "1x Menu").ends_with(" 1.234"), "{lines:#?}");
        assert!(line_of(&lines, "TOTAL").ends_with(" 1.234"), "{lines:#?}");
        let row = line_of(&lines, "IVA 10%");
        assert!(row.contains(" 1.122") && row.ends_with(" 0.112"), "{lines:#?}");
    }

    /// The cash report and the shelf label are money too: a yen till closes in yen.
    #[test]
    fn a_yen_cash_report_and_shelf_label_print_without_decimals() {
        let report = money_paper(
            DocumentType::CashSessionReport,
            &json!({
                "decimals": 0,
                "opening_balance": 10000.0,
                "closing_balance": 25000.0,
                "transactions": [{ "label": "Venta", "amount": 15000.0 }],
            }),
        );
        assert!(!report.iter().any(|l| l.contains(".00")), "{report:#?}");
        assert!(line_of(&report, "Venta").ends_with(" 15000"), "{report:#?}");
        let label = money_paper(
            DocumentType::BarcodeLabel,
            &json!({ "decimals": 0, "product_name": "Te verde", "price": 480.0 }),
        );
        assert!(label.iter().any(|l| l.trim() == "480"), "{label:#?}");
    }

    /// **A document without `decimals` prints exactly as before** — two decimals. Every module
    /// in the fleet that predates hub#2129 sends none, and a euro ticket must not change a byte.
    /// A scale that is not a whole number between 0 and 4 (the widest ISO 4217 scale) is not
    /// trusted either: two decimals, never a ticket with nine.
    #[test]
    fn a_document_without_a_valid_scale_prints_two_decimals() {
        let mut data = yen_ticket();
        data.as_object_mut().unwrap().remove("decimals");
        let without = render_document(DocumentType::Receipt, &data).expect("valid");
        let lines = money_paper(DocumentType::Receipt, &data);
        assert!(line_of(&lines, "TOTAL").ends_with(" 1400.00"), "{lines:#?}");
        for bad in [json!(5), json!(-1), json!("0"), json!(2.5), json!(null)] {
            data["decimals"] = bad.clone();
            let bytes = render_document(DocumentType::Receipt, &data).expect("valid");
            assert_eq!(bytes, without, "decimals = {bad} must print as if absent");
        }
        data["decimals"] = json!(2);
        assert_eq!(
            render_document(DocumentType::Receipt, &data).expect("valid"),
            without,
            "an explicit 2 is today's paper"
        );
    }

    // ── hub#2462 · the business's own text wraps by words on the roll ───────────────────────────
    //
    // The header, the footer and the notes of the ticket are prose the business (or the producer)
    // writes, of any length. Printed as one line, the printer folds them at its column count
    // without looking where — «…de 2,00 E» / «UR, ya aplicado…» on a 58 mm roll, «ya apl» /
    // «icado…» on an 80 mm one at 42 columns. Wrapped by words at the paper's 32 columns, no roll
    // the hub drives ever has to fold them.

    /// Columns a roll prints per line: 58 mm (32) and 80 mm (42 or 48, depending on its font).
    const ROLL_COLUMNS: [usize; 3] = [32, 42, 48];

    /// The paper as the printer receives it, one entry per line, decoded from cp437 so accented
    /// text reads as written. Unlike [`strip_escpos`] it steps over the QR commands
    /// (`GS ( k pL pH …`), which the ticket carries around its notes.
    fn roll_lines(bytes: &[u8]) -> Vec<String> {
        use oem_cp::StringExt;
        let (mut out, mut cur) = (Vec::new(), Vec::new());
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                0x1d if bytes.get(i + 1) == Some(&0x28) => {
                    let len = bytes[i + 3] as usize + ((bytes[i + 4] as usize) << 8);
                    i += 5 + len;
                }
                0x1b | 0x1d => i += 3,
                b'\n' => {
                    out.push(String::from_cp_lossy::<Cp437>(&cur));
                    cur.clear();
                    i += 1;
                }
                b => {
                    cur.push(b);
                    i += 1;
                }
            }
        }
        out
    }

    /// What a printer with `columns` columns does to a line longer than that: it folds it there.
    fn folded(line: &str, columns: usize) -> Vec<String> {
        let chars: Vec<char> = line.chars().collect();
        chars.chunks(columns).map(|c| c.iter().collect()).collect()
    }

    /// Every line of the roll fits the paper, and on every roll width each word of `text` reaches
    /// the paper whole, each of its own lines starting a printed line of its own.
    fn assert_wrapped_by_words(lines: &[String], field: &str, text: &str) {
        for l in lines {
            assert!(
                l.chars().count() <= LINE_WIDTH,
                "{field}: every line fits the paper ({LINE_WIDTH} columns): {l:?}\n{lines:#?}"
            );
        }
        for columns in ROLL_COLUMNS {
            let pieces: Vec<String> = lines.iter().flat_map(|l| folded(l, columns)).collect();
            for word in to_printable(text).split_whitespace() {
                assert!(
                    pieces.iter().any(|p| p.split_whitespace().any(|w| w == word)),
                    "{field}: «{word}» is cut on a {columns}-column roll:\n{pieces:#?}"
                );
            }
        }
        for own_line in to_printable(text).lines() {
            assert!(
                lines.iter().any(|l| !l.trim().is_empty() && own_line.starts_with(l.trim())),
                "{field}: «{own_line}» starts a printed line of its own:\n{lines:#?}"
            );
        }
    }

    /// The business's text on the ticket, long enough not to fit one line, with accents, `€`
    /// (printed `EUR`, three columns for one character) and the line break Ventas puts between
    /// the discount note and the business's footer (sales#505).
    const LONG_TEXTS: [(&str, &str); 5] = [
        ("receipt_header", "Vale de 5 € para tu próxima cita en el salón"),
        (
            "receipt_footer",
            "Descuento en el ticket de 2,00 €, ya aplicado en los importes de arriba.\n\
             Gracias por su visita a Peluquería Begoña, ¡hasta pronto!",
        ),
        ("business_address", "Calle de la Constitución 125, bajo izquierda, 29001 Málaga"),
        ("claim_note", "Pide tu factura completa escaneando este código con el móvil"),
        ("promo_note", "Escanea y déjanos una reseña en Google: nos ayudas muchísimo"),
    ];

    fn ticket_with(field: &str, text: &str) -> serde_json::Value {
        let mut data = json!({
            "business_name": "Bar Manolo",
            "receipt_id": "T-45",
            "items": [{ "name": "Cafe", "quantity": 1, "total": 1.2 }],
            "total": 1.2,
            "qr_data": "https://prewww2.aeat.es/wlpl/TIKE-CONT/ValidarQR?nif=B1",
            "claim_qr_data": "https://bar.erplora.com/p/ABCD1234ABCD1234",
            "promo_qr": PROMO_URL,
        });
        data[field] = json!(text);
        data
    }

    /// **The point of the issue**: the business's text reaches the ticket cut between words, never
    /// inside one, on a 58 mm roll and on an 80 mm one.
    #[test]
    fn the_business_text_on_the_ticket_wraps_by_words_on_every_roll() {
        for (field, text) in LONG_TEXTS {
            let bytes = render_document(DocumentType::Receipt, &ticket_with(field, text))
                .expect("a valid ticket");
            assert_wrapped_by_words(&roll_lines(&bytes), field, text);
        }
    }

    /// The full invoice prints the same header, footer and address, so it wraps them the same way.
    #[test]
    fn the_business_text_on_the_full_invoice_wraps_by_words_too() {
        for (field, text) in LONG_TEXTS.iter().filter(|(f, _)| !f.ends_with("_note")) {
            let mut data = full_invoice();
            data[*field] = json!(text);
            let bytes = render_document(DocumentType::Invoice, &data).expect("a valid invoice");
            assert_wrapped_by_words(&roll_lines(&bytes), field, text);
        }
    }

    /// The bill taken to the table prints the business's address too, wrapped the same way.
    #[test]
    fn the_address_on_the_bill_wraps_by_words_too() {
        let (field, text) = LONG_TEXTS[2];
        let data = json!({
            "items": [{ "name": "Cafe", "quantity": 1, "total": 1.2 }],
            "total": 1.2,
            field: text,
        });
        let bytes = render_document(DocumentType::Prebill, &data).expect("a valid bill");
        assert_wrapped_by_words(&roll_lines(&bytes), field, text);
    }

    /// A blank line the business typed between two paragraphs of its footer stays on the paper.
    #[test]
    fn a_blank_line_in_the_footer_stays_on_the_paper() {
        let bytes = render_document(
            DocumentType::Receipt,
            &ticket_with("receipt_footer", "Gracias por su visita\n\nVuelva pronto"),
        )
        .expect("a valid ticket");
        let lines = roll_lines(&bytes);
        let at = lines.iter().position(|l| l == "Gracias por su visita").expect("the footer prints");
        assert_eq!(lines[at + 1..at + 3], ["", "Vuelva pronto"], "{lines:#?}");
    }

    /// A text that already fits prints exactly as it did: one line, as written.
    #[test]
    fn a_short_footer_prints_exactly_as_today() {
        let data = ticket_with("receipt_footer", "Vuelva pronto");
        let bytes = render_document(DocumentType::Receipt, &data).expect("a valid ticket");
        let lines = roll_lines(&bytes);
        assert_eq!(
            lines.iter().filter(|l| l.contains("Vuelva")).collect::<Vec<_>>(),
            vec!["Vuelva pronto"],
            "{lines:#?}"
        );
    }
}

/// **Typographic punctuation → what a cp437 printer can actually produce.**
///
/// The encoder replaces whatever cp437 cannot represent with `?`, so the translated bill notice
/// («Cuenta — no es una factura», with the em dash a translator naturally types) reached the paper
/// as «Cuenta ? no es una factura». Curly quotes, ellipses and non-breaking spaces do the same, and
/// they come from people writing in a text editor — not a habit anybody is going to stop having, so
/// this is the layer that copes. Accented Spanish needs none of this: cp437 has it.
fn to_printable(txt: &str) -> String {
    let mut out = String::with_capacity(txt.len());
    for c in txt.chars() {
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
            other => out.push(other),
        }
    }
    out
}

/// Parte un texto en líneas de como mucho `width` columnas, **por palabras**.
///
/// La impresora corta por columna sin mirar dónde: sin esto, el aviso de la cuenta salía partido a
/// mitad de palabra. Una palabra más larga que el ancho se deja tal cual — partirla la haría
/// ilegible y es preferible que la impresora la doble.
///
/// It measures the text as it reaches the paper ([`to_printable`]): `€` prints as `EUR`, three
/// columns for one character, and counting it as one let a line overflow by two (hub#2462).
fn wrap_to_width(txt: &str, width: usize) -> Vec<String> {
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
/// wrapped by words to the paper, keeping the line breaks it already has (hub#2462).
///
/// Printed as one line, the printer folds it at its own column count without looking where, and
/// the customer gets words cut in two on a 58 mm roll and on an 80 mm one alike.
fn print_wrapped(b: &mut EscposBuilder, txt: &str) {
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

/// Formatea la cantidad como lo hace `f"{qty}x …"` en Python: si el JSON trae un entero
/// (`2`) se imprime `2`, no `2.0`. Si trae decimal (`1.5`) se imprime `1.5`.
fn fmt_qty(raw: Option<&serde_json::Value>, fallback: f64) -> String {
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
            // Sin valor → el default de Python es el entero 1.
            if fallback.fract() == 0.0 {
                format!("{}", fallback as i64)
            } else {
                fallback.to_string()
            }
        }
    }
}
