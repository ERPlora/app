//! The paper's language.

/// Which key of the catalogue below; the values live in [`Locale::label`].
///
/// An enum rather than free strings so the compiler, not a reviewer, is what guarantees every
/// label has both columns filled in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Label {
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
    // The full invoice.
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
    // The cash-up report: neutral, partial (X) and closing (Z) titles.
    CashReportTitle,
    PartialReportTitle,
    CashClosingTitle,
    Session,
    Opening,
    Closing,
    // Anything else.
    GenericTitle,
    // The test sheet.
    TestPageOk,
    TestPagePrinter,
}

/// **The language the paper is printed in.**
///
/// The chit and the ticket are precisely the surface the CUSTOMER and the KITCHEN read, not an
/// admin screen, so a hub running in another language must not paint a translated KDS on screen
/// and push a *Spanish* chit out of the thermal printer.
///
/// **The catalogue lives here, and the language travels in the document** (`locale`). The
/// alternative — each producer sending its ~30 labels already translated inside `data` — spreads
/// one catalogue over N module repos, turns adding a label into an N-repo change, and buys a
/// half-Spanish ticket from every module that forgets one. Odoo does it this way too: the report
/// gets a `lang` and the template's own catalogue resolves the strings.
///
/// 🔴 **English is the source and `es` is a translation**, but the FALLBACK is `es`: a document
/// without `locale` — which is every `erplora-app` deployed today — must print the paper of today,
/// byte for byte. The device contract grows; it never moves under a fleet that cannot be updated
/// as fast as a module can.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Locale {
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
    pub(super) fn from_document(data: &serde_json::Value) -> Self {
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
    /// The `es` column is, literal for literal, the Spanish paper every venue already prints — the
    /// characterisation test holds it there, because moving one of these strings moves the paper
    /// in every kitchen of the fleet at once.
    pub(super) fn label(self, key: Label) -> &'static str {
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
            // What the A4 invoice, Holded, Odoo and Business Central title an R1–R4.
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
            // A report that says neither X nor Z may be an open session's: never «CIERRE».
            Label::CashReportTitle => ("CASH REPORT", "INFORME DE CAJA"),
            // The X and the Z as the Hub's screen titles them («Partial report (X)», «Cash
            // closing (Z)»), so the paper and the browser print never disagree.
            Label::PartialReportTitle => ("PARTIAL REPORT (X)", "INFORME PARCIAL (X)"),
            Label::CashClosingTitle => ("CASH CLOSING (Z)", "CIERRE DE CAJA (Z)"),
            Label::Session => ("Session: ", "Sesion: "),
            Label::Opening => ("Opening", "Apertura"),
            Label::Closing => ("Closing", "Cierre"),
            Label::GenericTitle => ("Document", "Documento"),
            // The two lines of the test sheet. Unaccented like every other `es` entry
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
