//! Parse Peruvian bank / wallet notification emails into normalized fields.
//!
//! Senders and formats were captured from real inbox samples (2026-07). Each
//! parser is keyed by the sender address and pulls amount / merchant / currency
//! / direction from the message text (plaintext preferred; strip HTML first).
//!
//! The transaction time is NOT parsed here — the ingester uses the email's
//! received date, which is what reconciliation matches on.

use regex::Regex;
use serde::{Deserialize, Serialize};

/// Normalized result of parsing one notification email.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parsed {
    /// Logical source key, e.g. "paypal", "bcp", "interbank", "scotiabank".
    pub source: String,
    /// Account this movement belongs to, e.g. "paypal", "bcp_debito",
    /// "interbank", "interbank_amex", "scotiabank".
    pub account_hint: String,
    pub amount_cents: i64,
    pub currency: String,
    /// "in" or "out".
    pub direction: String,
    pub merchant: String,
}

/// Dispatch to the right parser by sender. Returns `None` if the sender is
/// unknown or the body doesn't match (e.g. a marketing email).
pub fn parse(sender: &str, _subject: &str, text: &str) -> Option<Parsed> {
    // HTML-stripped text (the common case for single-part-HTML bank emails)
    // leaves ragged whitespace where tags used to sit -- e.g. "tu <b>Tarjeta"
    // becomes "tu  Tarjeta". Several parsers below match literal single
    // spaces, so collapse intra-line runs of whitespace before dispatching.
    // Newlines are kept as line breaks (not collapsed away): parse_paypal's
    // `(?m)^` anchor relies on them to find the payer's line.
    let text = &strip_markdown_emphasis(text)
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n");
    let s = sender.to_ascii_lowercase();
    if s.contains("paypal.com") {
        parse_paypal(text)
    } else if s.contains("notificacionesbcp.com.pe") {
        parse_bcp(text)
            .or_else(|| parse_bcp_yapeo_received(text))
            .or_else(|| parse_bcp_own_card_payment(text))
            .or_else(|| parse_bcp_other_bank_card_payment(text))
            .or_else(|| parse_bcp_service_payment(text))
            .or_else(|| parse_bcp_directed_payment(text))
            .or_else(|| parse_bcp_transfer_to_third_party(text))
    } else if s.contains("netinterbank.com.pe") {
        parse_interbank_card(text)
    } else if s.contains("sip.pe") {
        parse_sip(text)
    } else if s.contains("operaciones.agora.pe") {
        parse_interbank_op(text)
    } else if s.contains("scotiabank.com.pe") {
        parse_scotiabank(text)
            .or_else(|| parse_scotiabank_plin_sent(text))
            .or_else(|| parse_scotiabank_transport_recharge(text))
            .or_else(|| parse_scotiabank_qr_payment(text))
    } else {
        None
    }
}

/// Some BCP notification templates ("Banca Móvil BCP") arrive as
/// multipart/alternative with a text/plain part written in Markdown-ish
/// emphasis: "Realizaste un pago dirigido de *S/ 415.63* hacia tu *VISA
/// Clásica*." The upstream email pipeline (both this backfill's `texto()` and
/// n8n's mailparser) prefers that plaintext part over text/html when both
/// exist, so these asterisks show up in `text` and break every literal-space
/// match below. Strip them -- but two other real uses of `*` must survive:
/// a merchant descriptor like "DLC*UBER RIDES" or "MDOPAGO*MERCADO PAGO"
/// (single asterisk, no space on either side), and a masked card/account
/// number like "**** 9602" (a run of 3+ asterisks, used by both templates,
/// always whitespace-bounded -- indistinguishable from emphasis by adjacency
/// alone, so runs that long are kept unconditionally instead).
fn strip_markdown_emphasis(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let is_word = |c: char| !c.is_whitespace() && c != '*';
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '*' {
            let start = i;
            while i < chars.len() && chars[i] == '*' {
                i += 1;
            }
            let len = i - start;
            let prev_is_word = start > 0 && is_word(chars[start - 1]);
            let next_is_word = i < chars.len() && is_word(chars[i]);
            if len >= 3 || (prev_is_word && next_is_word) {
                out.extend(std::iter::repeat('*').take(len));
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// "1,234.56" / "10.90" / "45.30" -> cents.
fn amount_to_cents(raw: &str) -> Option<i64> {
    let cleaned: String = raw.chars().filter(|c| c.is_ascii_digit() || *c == '.' || *c == ',').collect();
    let cleaned = cleaned.replace(',', ""); // drop thousands separators
    let value: f64 = cleaned.parse().ok()?;
    Some((value * 100.0).round() as i64)
}

fn clean(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").trim_end_matches('.').trim().to_string()
}

/// PayPal — incoming funds: "Scale Labs le envió $ 535.32 USD".
fn parse_paypal(text: &str) -> Option<Parsed> {
    // The payer is on its own line in the headline; anchor to line start so the
    // greeting ("Hola, <recipient>") on the previous line isn't swallowed.
    let re = Regex::new(r"(?im)^\s*(.+?)\s+le envió\s*\$\s*([\d.,]+)\s*USD").ok()?;
    let c = re.captures(text)?;
    Some(Parsed {
        source: "paypal".into(),
        account_hint: "paypal".into(),
        amount_cents: amount_to_cents(&c[2])?,
        currency: "USD".into(),
        direction: "in".into(),
        merchant: clean(&c[1]),
    })
}

/// BCP card consumption (débito or crédito), in soles or dollars: "Realizaste
/// un consumo de S/ 10.90 con tu Tarjeta de Crédito BCP en DLC*UBER RIDES." /
/// "...consumo de $ 10.00 con tu Tarjeta de Crédito BCP en ANOMALY." The card
/// type routes to a different account.
fn parse_bcp(text: &str) -> Option<Parsed> {
    let re = Regex::new(
        r"(?i)consumo de\s*(S/|\$)\s*([\d.,]+)\s*con tu Tarjeta de (Cr[ée]dito|D[ée]bito) BCP en\s*(.+?)[\.\n]",
    )
    .ok()?;
    let c = re.captures(text)?;
    let is_credito = c[3].to_lowercase().starts_with("cr");
    Some(Parsed {
        source: "bcp".into(),
        account_hint: if is_credito { "bcp_credito" } else { "bcp_debito" }.into(),
        amount_cents: amount_to_cents(&c[2])?,
        currency: if &c[1] == "$" { "USD" } else { "PEN" }.into(),
        direction: "out".into(),
        merchant: clean(&c[4]),
    })
}

/// BCP Yapeo received: "Recibiste un yapeo de S/ 200.00 de Milagros Ricapa
/// Tolentino."
fn parse_bcp_yapeo_received(text: &str) -> Option<Parsed> {
    let re = Regex::new(r"(?i)Recibiste un yapeo de\s*(S/|\$)\s*([\d.,]+)\s*de\s*(.+?)[\.\n]").ok()?;
    let c = re.captures(text)?;
    Some(Parsed {
        source: "bcp".into(),
        account_hint: "bcp_debito".into(),
        amount_cents: amount_to_cents(&c[2])?,
        currency: if &c[1] == "$" { "USD" } else { "PEN" }.into(),
        direction: "in".into(),
        merchant: clean(&c[3]),
    })
}

/// BCP own-card bill payment from the checking account: "Realizaste un pago a
/// tu tarjeta de S/ 936.10 desde tu Cuenta digital ... Pagado a VISA Clásica
/// **** 9602 Tipo de pago Pago total".
fn parse_bcp_own_card_payment(text: &str) -> Option<Parsed> {
    let re = Regex::new(r"(?i)Realizaste un pago a tu tarjeta de\s*(S/|\$)\s*([\d.,]+)\s*desde tu Cuenta digital")
        .ok()?;
    let c = re.captures(text)?;
    let merchant = Regex::new(r"(?i)Pagado a\s*(.+?)\s+Tipo de pago")
        .ok()?
        .captures(text)
        .map(|m| clean(&m[1]))
        .unwrap_or_else(|| "Pago tarjeta propia BCP".into());
    Some(Parsed {
        source: "bcp".into(),
        account_hint: "bcp_debito".into(),
        amount_cents: amount_to_cents(&c[2])?,
        currency: if &c[1] == "$" { "USD" } else { "PEN" }.into(),
        direction: "out".into(),
        merchant,
    })
}

/// BCP payment toward a card at another bank: "Realizaste un Pago de tarjeta
/// a otro banco de S/ 206.81 a INTERBANK desde tu Cuenta digital ... Comisión
/// S/ 4.00 Total cobrado S/ 210.81". "Total cobrado" (payment + fee) is what
/// actually left the BCP account; falls back to "Monto pagado" for the rare
/// case with no fee line.
fn parse_bcp_other_bank_card_payment(text: &str) -> Option<Parsed> {
    let re = Regex::new(
        r"(?i)Pago de tarjeta a otro banco de\s*(S/|\$)\s*[\d.,]+\s*a\s*(.+?)\s+desde tu Cuenta digital",
    )
    .ok()?;
    let c = re.captures(text)?;
    let bank = clean(&c[2]);
    let amount = Regex::new(r"(?i)Total cobrado\s*(?:S/|\$)\s*([\d.,]+)")
        .ok()?
        .captures(text)
        .map(|m| m[1].to_string())
        .or_else(|| {
            Regex::new(r"(?i)Monto pagado\s*(?:S/|\$)\s*([\d.,]+)").ok()?.captures(text).map(|m| m[1].to_string())
        })?;
    Some(Parsed {
        source: "bcp".into(),
        account_hint: "bcp_debito".into(),
        amount_cents: amount_to_cents(&amount)?,
        currency: if &c[1] == "$" { "USD" } else { "PEN" }.into(),
        direction: "out".into(),
        merchant: format!("Pago tarjeta {bank}"),
    })
}

/// BCP utility/service bill payment: "Operación realizada: Pago de servicios
/// ... Empresa: ENTEL PERU S.A. Servicio: PAGO CON NUMERO TELEFONO ... Monto
/// total: S/ 36.40". This template's plaintext part wraps unrelated fields
/// (account number, expiry) across real line breaks between "Pago de
/// servicios" and "Monto total"; `(?s)` lets `.` cross them.
fn parse_bcp_service_payment(text: &str) -> Option<Parsed> {
    let re = Regex::new(
        r"(?is)Pago de servicios.*?Empresa:\s*(.+?)\s+Servicio:.*?Monto total:\s*(S/|\$)\s*([\d.,]+)",
    )
    .ok()?;
    let c = re.captures(text)?;
    Some(Parsed {
        source: "bcp".into(),
        account_hint: "bcp_debito".into(),
        amount_cents: amount_to_cents(&c[3])?,
        currency: if &c[2] == "$" { "USD" } else { "PEN" }.into(),
        direction: "out".into(),
        merchant: clean(&c[1]),
    })
}

/// BCP "pago dirigido" (directed payment, e.g. to a card via a savings
/// account rather than the checking "Cuenta digital"): "Realizaste un pago
/// dirigido de S/ 415.63 hacia tu VISA Clásica ... Pagado a VISA Clásica
/// **** 9602 Desde CUENTAS DE AHORRO **** 3124". Uses "Monto pagado", not
/// "Total cobrado" -- unlike the other-bank-card case, the two can be in
/// different currencies here (paying a PEN bill from a USD account), and the
/// bill amount is the more meaningful figure to record.
fn parse_bcp_directed_payment(text: &str) -> Option<Parsed> {
    let re = Regex::new(r"(?i)Realizaste un pago dirigido de\s*(S/|\$)\s*([\d.,]+)\s*hacia").ok()?;
    let c = re.captures(text)?;
    let merchant = Regex::new(r"(?i)Pagado a\s*(.+?)\s+Desde")
        .ok()?
        .captures(text)
        .map(|m| clean(&m[1]))
        .unwrap_or_else(|| "Pago dirigido BCP".into());
    Some(Parsed {
        source: "bcp".into(),
        account_hint: "bcp_debito".into(),
        amount_cents: amount_to_cents(&c[2])?,
        currency: if &c[1] == "$" { "USD" } else { "PEN" }.into(),
        direction: "out".into(),
        merchant,
    })
}

/// BCP transfer to a third party: "Realizaste una transferencia de S/ 240.00
/// desde tu Clasica ... Enviado a Solari De Hurtado Eda V." The recipient's
/// masked account number sits on the very next line, so the merchant capture
/// stops at end-of-line (`(?m)$`) rather than at a "Moneda" landmark -- this
/// template repeats/garbles fields after that point (seen in real samples).
fn parse_bcp_transfer_to_third_party(text: &str) -> Option<Parsed> {
    let re = Regex::new(r"(?i)Realizaste una transferencia de\s*(S/|\$)\s*([\d.,]+)\s*desde").ok()?;
    let c = re.captures(text)?;
    let merchant = Regex::new(r"(?im)Enviado a\s*(.+)$")
        .ok()?
        .captures(text)
        .map(|m| clean(&m[1]))
        .unwrap_or_else(|| "Transferencia BCP".into());
    Some(Parsed {
        source: "bcp".into(),
        account_hint: "bcp_debito".into(),
        amount_cents: amount_to_cents(&c[2])?,
        currency: if &c[1] == "$" { "USD" } else { "PEN" }.into(),
        direction: "out".into(),
        merchant,
    })
}

/// Scotiabank outgoing Plin: "Transferencia Plin ... Monto enviado: S/ 50.00
/// ... Enviado a: Carlos Obi*** *** *** 921". Counterpart of
/// `parse_scotiabank`'s "Monto recibido" (incoming).
fn parse_scotiabank_plin_sent(text: &str) -> Option<Parsed> {
    let c = Regex::new(r"(?i)Monto enviado:\s*(S/|\$)\s*([\d.,]+)").ok()?.captures(text)?;
    let merchant = Regex::new(r"(?i)Enviado a:\s*(.+?)[\.\n]")
        .ok()?
        .captures(text)
        .map(|m| clean(&m[1]))
        .unwrap_or_else(|| "Plin".into());
    Some(Parsed {
        source: "scotiabank".into(),
        account_hint: "scotiabank".into(),
        amount_cents: amount_to_cents(&c[2])?,
        currency: if &c[1] == "$" { "USD" } else { "PEN" }.into(),
        direction: "out".into(),
        merchant,
    })
}

/// Scotiabank transit-card recharge: "Recarga con Plin Monto: S/ 7.00
/// Número de tarjeta: ... Tipo de tarjeta: Tarjeta Metropolitano". This
/// template puts every label/value on its own line with blank lines between
/// them, so the connective needs `(?s)` to cross the real newlines.
fn parse_scotiabank_transport_recharge(text: &str) -> Option<Parsed> {
    let re = Regex::new(r"(?is)Recarga con Plin.*?Monto:\s*(S/|\$)\s*([\d.,]+)").ok()?;
    let c = re.captures(text)?;
    let merchant = Regex::new(r"(?i)Tipo de tarjeta:\s*(.+?)[\.\n]")
        .ok()?
        .captures(text)
        .map(|m| format!("Recarga transporte — {}", clean(&m[1])))
        .unwrap_or_else(|| "Recarga transporte".into());
    Some(Parsed {
        source: "scotiabank".into(),
        account_hint: "scotiabank".into(),
        amount_cents: amount_to_cents(&c[2])?,
        currency: if &c[1] == "$" { "USD" } else { "PEN" }.into(),
        direction: "out".into(),
        merchant,
    })
}

/// Scotiabank QR payment: "Pago con QR Pagaste con: Débito Mastercard ****
/// **** **** 0465 Monto: S/ 9.90 Pagaste a: T7108MOLINA". Same one-field-
/// per-line template as the transport recharge above.
fn parse_scotiabank_qr_payment(text: &str) -> Option<Parsed> {
    let re = Regex::new(r"(?is)Pago con QR.*?Monto:\s*(S/|\$)\s*([\d.,]+)").ok()?;
    let c = re.captures(text)?;
    let merchant = Regex::new(r"(?i)Pagaste a:\s*(.+?)[\.\n]")
        .ok()?
        .captures(text)
        .map(|m| clean(&m[1]))
        .unwrap_or_else(|| "Pago QR".into());
    Some(Parsed {
        source: "scotiabank".into(),
        account_hint: "scotiabank".into(),
        amount_cents: amount_to_cents(&c[2])?,
        currency: if &c[1] == "$" { "USD" } else { "PEN" }.into(),
        direction: "out".into(),
        merchant,
    })
}

/// Sip credit card: "Establecimiento: PUKU PUKU EL POLO  Monto: S/. 29.00".
fn parse_sip(text: &str) -> Option<Parsed> {
    let amount = Regex::new(r"(?i)Monto:\s*S/\.?\s*([\d.,]+)").ok()?.captures(text)?[1].to_string();
    let merchant = Regex::new(r"(?i)Establecimiento:\s*(.+?)\s+Monto:").ok()?
        .captures(text)
        .map(|c| clean(&c[1]))
        .unwrap_or_else(|| "Consumo Sip".into());
    Some(Parsed {
        source: "sip".into(),
        account_hint: "sip".into(),
        amount_cents: amount_to_cents(&amount)?,
        currency: "PEN".into(),
        direction: "out".into(),
        merchant,
    })
}

/// Interbank Amex card: "Comercio: Openpay  Monto: S/. 45.30".
fn parse_interbank_card(text: &str) -> Option<Parsed> {
    let amount = Regex::new(r"(?i)Monto:\s*S/\.?\s*([\d.,]+)").ok()?.captures(text)?[1].to_string();
    let merchant = Regex::new(r"(?i)Comercio:\s*(.+?)\s+Monto:").ok()?
        .captures(text)
        .map(|c| clean(&c[1]))
        .unwrap_or_else(|| "Consumo Amex".into());
    Some(Parsed {
        source: "interbank".into(),
        account_hint: "interbank_amex".into(),
        amount_cents: amount_to_cents(&amount)?,
        currency: "PEN".into(),
        direction: "out".into(),
        merchant,
    })
}

/// Interbank app operation (agora): "Monto S/ 26.00 ... Enviado a {name}".
fn parse_interbank_op(text: &str) -> Option<Parsed> {
    let amount = Regex::new(r"(?i)Monto\s*S/\s*([\d.,]+)").ok()?.captures(text)?[1].to_string();
    let merchant = Regex::new(r"(?i)Enviado a\s*(.+)").ok()?
        .captures(text)
        .map(|c| clean(&c[1]))
        .or_else(|| {
            Regex::new(r"(?i)Operaci[óo]n realizada:\s*(.+)").ok()?
                .captures(text)
                .map(|c| clean(&c[1]))
        })
        .unwrap_or_else(|| "Operación Interbank".into());
    Some(Parsed {
        source: "interbank".into(),
        account_hint: "interbank".into(),
        amount_cents: amount_to_cents(&amount)?,
        currency: "PEN".into(),
        direction: "out".into(),
        merchant,
    })
}

/// Scotiabank incoming Plin: "Recepción Transferencia Plin ... Monto recibido: S/ 49.00".
fn parse_scotiabank(text: &str) -> Option<Parsed> {
    let amount = Regex::new(r"(?i)Monto recibido:\s*S/\s*([\d.,]+)").ok()?.captures(text)?[1].to_string();
    Some(Parsed {
        source: "scotiabank".into(),
        account_hint: "scotiabank".into(),
        amount_cents: amount_to_cents(&amount)?,
        currency: "PEN".into(),
        direction: "in".into(),
        merchant: "Plin".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paypal_income() {
        let p = parse(
            "service@intl.paypal.com",
            "Scale Labs has sent you money",
            "Hola, Carlos Obispo Ricapa\nScale Labs le envió $ 535.32 USD\nId. de transacción 504021165E004010S",
        )
        .unwrap();
        assert_eq!(p.direction, "in");
        assert_eq!(p.currency, "USD");
        assert_eq!(p.amount_cents, 53532);
        assert_eq!(p.merchant, "Scale Labs");
    }

    #[test]
    fn bcp_debito_consumo() {
        let p = parse(
            "notificaciones@notificacionesbcp.com.pe",
            "Realizaste un consumo con tu Tarjeta de Débito BCP",
            "Hola Carlos Enrique, Realizaste un consumo de S/ 10.90 con tu Tarjeta de Débito BCP en PLIN-GUSTAVO YNJANTE. Por tu seguridad",
        )
        .unwrap();
        assert_eq!(p.direction, "out");
        assert_eq!(p.amount_cents, 1090);
        assert_eq!(p.merchant, "PLIN-GUSTAVO YNJANTE");
        assert_eq!(p.account_hint, "bcp_debito");
    }

    #[test]
    fn bcp_credito_uber() {
        let p = parse(
            "notificaciones@notificacionesbcp.com.pe",
            "Realizaste un consumo con tu Tarjeta de Crédito BCP",
            "Hola Carlos Enrique, Realizaste un consumo de S/ 11.50 con tu Tarjeta de Crédito BCP en DLC*UBER RIDES. Por tu seguridad",
        )
        .unwrap();
        assert_eq!(p.direction, "out");
        assert_eq!(p.amount_cents, 1150);
        assert_eq!(p.merchant, "DLC*UBER RIDES");
        assert_eq!(p.account_hint, "bcp_credito");
    }

    #[test]
    fn bcp_credito_usd() {
        let p = parse(
            "notificaciones@notificacionesbcp.com.pe",
            "Realizaste un consumo con tu Tarjeta de Crédito BCP - Servicio",
            "Hola Carlos Enrique, Realizaste un consumo de $ 10.00 con tu Tarjeta de Crédito BCP en ANOMALY. Por tu seguridad",
        )
        .unwrap();
        assert_eq!(p.direction, "out");
        assert_eq!(p.currency, "USD");
        assert_eq!(p.amount_cents, 1000);
        assert_eq!(p.merchant, "ANOMALY");
    }

    #[test]
    fn bcp_yapeo_received() {
        let p = parse(
            "notificaciones@notificacionesbcp.com.pe",
            "Constancia de recepción de Yapeo a celular BCP",
            "Hola Carlos Enrique, Recibiste un yapeo de S/ 200.00 de Milagros Ricapa Tolentino. Por tu seguridad te enviamos los datos de tu yapeo.",
        )
        .unwrap();
        assert_eq!(p.direction, "in");
        assert_eq!(p.amount_cents, 20000);
        assert_eq!(p.merchant, "Milagros Ricapa Tolentino");
        assert_eq!(p.account_hint, "bcp_debito");
    }

    #[test]
    fn bcp_own_card_payment() {
        let p = parse(
            "notificaciones@notificacionesbcp.com.pe",
            "Constancia de Pago de Tarjeta de Crédito Propia",
            "Hola Carlos Enrique, Realizaste un pago a tu tarjeta de S/ 936.10 desde tu Cuenta digital . A continuación, te enviamos los datos de tu operación. Montos Monto pagado S/ 936.10 Datos de la operación Operación realizada Pago de tarjeta propia BCP Fecha y hora 04 de Agosto de 2026 - 10:57 AM Pagado a VISA Clásica **** 9602 Tipo de pago Pago total",
        )
        .unwrap();
        assert_eq!(p.direction, "out");
        assert_eq!(p.amount_cents, 93610);
        assert_eq!(p.merchant, "VISA Clásica **** 9602");
    }

    #[test]
    fn bcp_other_bank_card_payment() {
        let p = parse(
            "notificaciones@notificacionesbcp.com.pe",
            "Constancia de Pago de Tarjeta de Crédito de Otros Bancos",
            "Hola Carlos Enrique, Realizaste un Pago de tarjeta a otro banco de S/ 206.81 a INTERBANK desde tu Cuenta digital . A continuación, te enviamos los datos de tu operación. Montos Monto pagado S/ 206.81 Comisión S/ 4.00 Total cobrado S/ 210.81 Tipo de cambio S/ 3.3760 Total cobrado al tipo de cambio $ 62.44",
        )
        .unwrap();
        assert_eq!(p.direction, "out");
        // "Total cobrado" (payment + fee), not the bare "Monto pagado".
        assert_eq!(p.amount_cents, 21081);
        assert_eq!(p.merchant, "Pago tarjeta INTERBANK");
    }

    #[test]
    fn bcp_card_payment_favorite_ignored() {
        assert!(parse(
            "notificaciones@notificacionesbcp.com.pe",
            "Constancia de Favorito: Agregaste un Pago de Tarjeta de Crédito de Otros",
            "Hola Carlos Enrique, Tu pago favorito \"amex gold\" fue creado correctamente. A continuación, te brindamos los detalles de tu operación. Detalle de favorito Fecha y hora 04 de Agosto de 2026 - 07:24 PM Operación Pago de Tarjeta Otros Bancos Titular Carlos Enrique Obispo R. Banco INTERBANK Tarjeta **** 6472",
        )
        .is_none());
    }

    #[test]
    fn bcp_service_payment() {
        let p = parse(
            "notificaciones@notificacionesbcp.com.pe",
            "ENVIO AUTOMATICO - CONSTANCIA DE PAGO DE SERVICIO - BANCA MOVIL BCP",
            "Hola CARLOS ENRIQUE, ¡Tu operación se realizó con éxito! Operación realizada: Pago de servicios Número de operación: 07411717 Fecha y hora: Sábado, 01 Agosto 2026 - 09:21 P. M. Empresa: ENTEL PERU S.A. Servicio: PAGO CON NUMERO TELEFONO Titular del servicio: CARLOS ENRIQUE OBISP Código de usuario: 946325921 Cuenta de origen: Tarjeta de crédito **** 9602 CARLOS ENRIQUE Monto total: S/ 36.40",
        )
        .unwrap();
        assert_eq!(p.direction, "out");
        assert_eq!(p.amount_cents, 3640);
        // clean() trims a trailing period (usually the sentence terminator);
        // here it happens to eat the last dot of the abbreviation too.
        assert_eq!(p.merchant, "ENTEL PERU S.A");
    }

    #[test]
    fn bcp_directed_payment() {
        let p = parse(
            "notificaciones@notificacionesbcp.com.pe",
            "Constancia de Pago Dirigido - Banca Móvil BCP",
            "Hola CARLOS ENRIQUE, Realizaste un pago dirigido de S/ 415.63 hacia tu VISA Clásica . Por tu seguridad, te enviamos los datos de tu operación. Monto Monto pagado S/ 415.63 Total cobrado $ 123.15 Datos de la operación Operación realizada Pago dirigido Fecha y hora 04/08/2026 - 11:00 a. m. Pagado a VISA Clásica **** 9602 Desde CUENTAS DE AHORRO **** 3124 Movimiento MDOPAGO*MERCA",
        )
        .unwrap();
        assert_eq!(p.direction, "out");
        assert_eq!(p.amount_cents, 41563);
        assert_eq!(p.currency, "PEN");
        assert_eq!(p.merchant, "VISA Clásica **** 9602");
    }

    // No standalone "clean single-line" test for transfer-to-third-party:
    // real samples confirm this BCP template is always the multipart
    // markdown-plaintext shape (see the _markdown_plaintext test below),
    // never single-part HTML on one line.

    #[test]
    fn bcp_service_payment_markdown_plaintext() {
        // Real text/plain body (BCP's "Banca Móvil BCP" templates ship this
        // alongside text/html, and the upstream pipeline prefers it) --
        // Markdown-style *emphasis* asterisks and real \r\n line breaks
        // between the fields this parser needs.
        let p = parse(
            "notificaciones@notificacionesbcp.com.pe",
            "ENVIO AUTOMATICO - CONSTANCIA DE PAGO DE SERVICIO - BANCA MOVIL BCP",
            "   \r\n\r\n*Hola CARLOS ENRIQUE,*\r\n\r\n¡Tu operación se realizó con éxito!\r\n\r\nOperación realizada:\r\n\r\n*Pago de servicios*\r\n\r\nNúmero de operación:\r\n\r\n*07411717*\r\n\r\n     \r\n\r\nFecha y hora: *Sábado, 01 Agosto 2026 - 09:21 P. M.* Empresa: *ENTEL PERU S.A.* Servicio: *PAGO CON NUMERO TELEFONO* Titular del servicio: *CARLOS ENRIQUE OBISP* Código de usuario: *946325921* Comisión: ** Cuenta de origen: *Tarjeta de crédito\r\n**** 9602\r\nCARLOS ENRIQUE* Vigencia: ** Valor venta: ** IGV: ** Subtotal: ** Monto total: *S/ 36.40* Tipo de cambio: ** Monto transferido al cambio: **",
        )
        .unwrap();
        assert_eq!(p.direction, "out");
        assert_eq!(p.amount_cents, 3640);
        assert_eq!(p.merchant, "ENTEL PERU S.A");
    }

    #[test]
    fn bcp_directed_payment_markdown_plaintext() {
        let p = parse(
            "notificaciones@notificacionesbcp.com.pe",
            "Constancia de Pago Dirigido - Banca Móvil BCP",
            "Hola *CARLOS ENRIQUE,*\r\nRealizaste un pago dirigido de *S/ 415.63* hacia tu *VISA Clásica*.\r\nPor tu seguridad, te enviamos los *datos de tu operación.*\r\n*Monto*\r\n \r\n\r\nMonto pagado *S/ 415.63* Total cobrado *$ 123.15*\r\n\r\n*Datos de la operación*\r\n \r\n\r\nOperación realizada *Pago dirigido* Fecha y hora *04/08/2026 - 11:00 a. m.* Pagado a *VISA Clásica* ***** 9602* Desde *CUENTAS DE AHORRO* ***** 3124* Movimiento *MDOPAGO*MERCADO PAGO   LIMA          PE* Tipo de cambio *$ 3.3750* Monto cobrado *$ 123.15* Tipo de pago *Pago completo* Número de operación *01841508*",
        )
        .unwrap();
        assert_eq!(p.direction, "out");
        assert_eq!(p.currency, "PEN");
        assert_eq!(p.amount_cents, 41563);
        // The mask run here happens to be 5 asterisks in the real sample
        // (not the usual 4) -- preserved either way, kept as observed.
        assert_eq!(p.merchant, "VISA Clásica ***** 9602");
    }

    #[test]
    fn bcp_transfer_to_third_party_markdown_plaintext() {
        let p = parse(
            "notificaciones@notificacionesbcp.com.pe",
            "Constancia de Transferencia a Terceros BCP",
            "Hola *Carlos Enrique,*\r\n\r\nRealizaste una transferencia de *S/ 240.00* desde tu *Clasica.*\r\n\r\nPor tu seguridad, te enviamos los *datos de tu operación.*\r\n\r\n*Montos*\r\n\r\n\r\n\r\nMonto transferido *S/ 240.00* Tipo de cambio ** *Total cobrado al tipo de cambio* **\r\n\r\n*Datos de la operación*\r\n\r\n    \r\n\r\nOperación realizada *Transferencia a terceros BCP* Fecha y hora *08 de Agosto de 2026 - 01:03 AM* Enviado a *Solari De Hurtado Eda V.*\r\n**** 8026\r\nMoneda Soles Desde *Clasica*\r\n**** 6096\r\nMoneda Soles Desde *Clasica*\r\n**** 6096 Enviado a **\r\n**** 8026 Mensaje *Luz julio* Canal *Banca Móvil BCP* Número de operación *00077551*",
        )
        .unwrap();
        assert_eq!(p.direction, "out");
        assert_eq!(p.amount_cents, 24000);
        assert_eq!(p.merchant, "Solari De Hurtado Eda V");
    }

    #[test]
    fn merchant_code_asterisk_survives_markdown_strip() {
        // "DLC*UBER RIDES" must NOT be treated as emphasis and stripped --
        // the asterisk there is part of the payment-gateway merchant code
        // (no whitespace on either side of it).
        let p = parse(
            "notificaciones@notificacionesbcp.com.pe",
            "Realizaste un consumo con tu Tarjeta de Crédito BCP",
            "Hola Carlos Enrique, Realizaste un consumo de S/ 11.50 con tu Tarjeta de Crédito BCP en DLC*UBER RIDES. Por tu seguridad",
        )
        .unwrap();
        assert_eq!(p.merchant, "DLC*UBER RIDES");
    }

    #[test]
    fn scotiabank_plin_sent() {
        let p = parse(
            "bancadigital@scotiabank.com.pe",
            "Constancia de operación - Transferencia Plin",
            "Hola Carlos, Esta es la constancia de tu transferencia Plin: 05 ago., 12:53 pm Transferencia Plin Número de operación 784.444.022.9730 Cuenta de origen: Cuenta Digital SBP *** ***8896 Monto enviado: S/ 50.00 Destino: Yape Comisión: Gratis Enviado a: Carlos Obi*** *** *** 921. Con Plin envías dinero gratis.",
        )
        .unwrap();
        assert_eq!(p.direction, "out");
        assert_eq!(p.amount_cents, 5000);
        assert_eq!(p.source, "scotiabank");
    }

    #[test]
    fn scotiabank_transport_recharge() {
        // Real body: this template puts every label and value on its own
        // line, blank lines in between -- needs (?s) to bridge them.
        let p = parse(
            "bancadigital@scotiabank.com.pe",
            "Constancia de operación - Recarga de transporte con Plin",
            "Hola Carlos , Scotiabank te envía la constancia de recarga de transporte.\n\n     Recarga con Plin \r\n                     \r\n                 \r\n                            Monto:\r\n                         \r\n                     \r\n                         S/ 7.00 \r\n                     \r\n                 \r\n                            Número de tarjeta:\r\n                         \r\n                         3655385387 \r\n                     \r\n                 \r\n                            Tipo de tarjeta:\r\n                         \r\n                         Tarjeta Metropolitano \r\n                     \r\n                 \r\n                            Cuenta de origen:\r\n                         \r\n                         Cuenta Ahorro *** ***8896 \r\n",
        )
        .unwrap();
        assert_eq!(p.direction, "out");
        assert_eq!(p.amount_cents, 700);
        assert_eq!(p.source, "scotiabank");
        assert_eq!(p.merchant, "Recarga transporte — Tarjeta Metropolitano");
    }

    #[test]
    fn scotiabank_qr_payment() {
        let p = parse(
            "bancadigital@scotiabank.com.pe",
            "Pago con QR",
            "     Pago con QR \n\n                     \n\n                          Pagaste con:  \n\n                     \n\n                         Débito Mastercard \n\n                         **** **** **** 0465 \n\n                     \n\n                          Monto:  \n\n                     \n\n                             S/ 9.90 \n\n                         \n\n                     \n\n                          Pagaste a:  \n\n                     \n\n                         T7108MOLINA \n\n                     \n\n",
        )
        .unwrap();
        assert_eq!(p.direction, "out");
        assert_eq!(p.amount_cents, 990);
        assert_eq!(p.merchant, "T7108MOLINA");
    }

    #[test]
    fn sip_consumo() {
        let p = parse(
            "no-reply@servicioalcliente.sip.pe",
            "Sip, realizaste un consumo con tu Tarjeta de Crédito Sip",
            "Hola, CARLOS. Has realizado una transacción con tu Tarjeta de Crédito Sip. Tarjeta Titular: XXXXXXXXXXXX2514 Establecimiento: PUKU PUKU EL POLO Monto: S/. 29.00 Fecha de operación: 19/07/2026",
        )
        .unwrap();
        assert_eq!(p.direction, "out");
        assert_eq!(p.amount_cents, 2900);
        assert_eq!(p.merchant, "PUKU PUKU EL POLO");
        assert_eq!(p.source, "sip");
    }

    #[test]
    fn interbank_card_consumo() {
        let p = parse(
            "servicioalcliente@netinterbank.com.pe",
            "realizaste un consumo con tu Tarjeta American Express",
            "Tarjeta: ****472 Comercio: Openpay Monto: S/. 45.30 Fecha: 21/07/2026 Hora: 01:04 PM",
        )
        .unwrap();
        assert_eq!(p.amount_cents, 4530);
        assert_eq!(p.merchant, "Openpay");
        assert_eq!(p.direction, "out");
    }

    #[test]
    fn interbank_op_send() {
        let p = parse(
            "no-reply@operaciones.agora.pe",
            "Realizaste una operación",
            "Operación realizada: Enviar a celular Monto S/ 26.00 Enviado a Elizabeth Rosario Destino Yape",
        )
        .unwrap();
        assert_eq!(p.amount_cents, 2600);
        assert_eq!(p.direction, "out");
        assert!(p.merchant.starts_with("Elizabeth"));
    }

    #[test]
    fn scotiabank_plin_reception() {
        let p = parse(
            "bancadigital@scotiabank.com.pe",
            "Constancia de operación - Recepción Transferencia Plin",
            "Hola CARLOS, Esta es la constancia de la transferencia que has recibido: Monto recibido: S/ 49.00 Destino: Cuenta Digital Scotiabank",
        )
        .unwrap();
        assert_eq!(p.direction, "in");
        assert_eq!(p.amount_cents, 4900);
        assert_eq!(p.source, "scotiabank");
    }

    #[test]
    fn marketing_email_ignored() {
        assert!(parse("bcpcomunica@email.bcp.com.pe", "Cyber BCP", "Participa por un viaje").is_none());
    }
}
