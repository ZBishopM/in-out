# Email sources (transaction notifications)

Real senders + formats observed in the user's inbox (2026-07). These drive the
`email-parse` crate. Marketing senders (e.g. `bcpcomunica@email.bcp.com.pe`,
`alertas@email.bcp.com.pe`, `noreply@novedadesyape.pe`) are **not** transactional
and must be ignored.

| Source | Sender | Anchor(s) | Dir | Account |
|--------|--------|-----------|-----|---------|
| PayPal | `service@intl.paypal.com` | `<payer> le envió $ <amount> USD` | in | paypal (USD) |
| BCP débito | `notificaciones@notificacionesbcp.com.pe` | `Realizaste un consumo de S/ <amount> ... en <merchant>` | out | bcp_debito |
| Interbank card (Amex) | `servicioalcliente@netinterbank.com.pe` | `Comercio: <m>  Monto: S/. <amount>` | out | interbank_amex |
| Interbank app op | `no-reply@operaciones.agora.pe` | `Monto S/ <amount> ... Enviado a <name>` / `Origen <acct>` | out | interbank |
| Scotiabank (Plin in) | `bancadigital@scotiabank.com.pe` | `Recepción Transferencia Plin ... Monto recibido: S/ <amount>` | in | scotiabank |

## Notes

- **Yape / Plin** are not separate senders — they appear *inside* the above:
  BCP débito shows `en PLIN-<name>`, Interbank shows `Destino: Yape`.
- Transaction **time** is taken from the email's received date, not parsed from
  the body (reconciliation matches on that).
- The Interbank card `Comercio:` field is the merchant used for dedup against a
  matching merchant receipt (e.g. an Uber email + the Amex charge).
- Ingester should feed the **plaintext** body (or HTML stripped to text) to
  `email_parse::parse(sender, subject, text)`.
