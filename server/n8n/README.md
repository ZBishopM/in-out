# n8n — email ingestion (F2)

Architecture: **n8n reads Gmail and forwards raw emails to the Rust `ingest-api`
service**, which parses them (single source of truth: the `email-parse` crate),
writes `raw_events`, and reconciles. n8n does no parsing.

```
Gmail Trigger (poll)  ->  Code (build EmailIn[])  ->  HTTP Request (POST ingest-api)
```

## 1. Gmail credential

Create a Google Cloud project, enable the Gmail API, and add an **OAuth2**
credential in n8n with scope `https://www.googleapis.com/auth/gmail.readonly`.

## 2. Gmail Trigger node

Poll every few minutes. Restrict to the transactional senders (see
[email-sources.md](../email-sources.md)) so marketing is never fetched:

```
from:(service@intl.paypal.com OR notificaciones@notificacionesbcp.com.pe OR
      servicioalcliente@netinterbank.com.pe OR no-reply@operaciones.agora.pe OR
      bancadigital@scotiabank.com.pe)
```

Enable "Download" so the node returns the message body.

## 3. Code node — build the payload

Map each Gmail item to an `EmailIn`. Prefer the plaintext body; else strip HTML.

```js
return items.map(i => {
  const j = i.json;
  const text = j.textPlain || (j.textHtml || '').replace(/<[^>]+>/g, ' ');
  return { json: {
    gmail_msg_id: j.id,
    sender: (j.from?.value?.[0]?.address) || j.from || '',
    subject: j.subject || '',
    text,
    received_at: new Date((j.date || Date.now())).toISOString(),
  }};
});
```

## 4. HTTP Request node

- Method **POST**, URL `http://ingest-api:8090/ingest` (same Docker network).
- Header `Authorization: Bearer <INGEST_TOKEN>` (from `server/.env`).
- Body: **JSON**, send all items as one array (enable "Send all items as one
  request" / build the array in the Code node).

The service dedupes on `gmail_msg_id`, so overlapping polls are safe. It returns
`{received, parsed, inserted, created_transactions}`.

## Backfill

To import history once, widen the Gmail search (`newer_than:1y`) for a single
run, then narrow the trigger for steady state.
