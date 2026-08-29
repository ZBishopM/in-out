// Regresión para los dos bugs que dejaron al pipeline tirando correos en
// silencio durante cinco semanas (ver el commit del 2026-08-29). Los dos
// vivían en el Code node "Build payload" y ninguno rompía nada de forma
// visible: el workflow seguía marcando sus ejecuciones como "success"
// mientras descartaba absolutamente todos los correos bancarios.
//
// El test carga el jsCode DEL EXPORT REAL, no una copia: si alguien edita el
// nodo en la interfaz de n8n y reexporta, esto sigue probando lo que de
// verdad corre en producción, y no una versión paralela que se quedó vieja.
//
// Sin dependencias: runner nativo de Node.
//   node --test server/n8n/workflows/gmail-ingest.test.mjs
//
// El archivo, no la carpeta: en modo directorio el runner intenta cargar
// también el .json de al lado y falla antes de correr nada.

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const aquí = dirname(fileURLToPath(import.meta.url));
const wf = JSON.parse(readFileSync(join(aquí, 'gmail-ingest.json'), 'utf8'));
const jsCode = wf.nodes.find((n) => n.name === 'Build payload').parameters.jsCode;

/** Corre el Code node tal cual, con los items que le daría el trigger IMAP. */
function correrNodo(items) {
  const $input = { all: () => items.map((json) => ({ json })) };
  return new Function('$input', jsCode)($input)[0].json.emails;
}

/** Un correo con la forma EXACTA que entrega el nodo IMAP de n8n. */
function correoImap({ from, textHtml }) {
  return {
    from,
    subject: 'Realizaste un consumo con tu Tarjeta de Crédito BCP',
    textPlain: '', // presente pero vacío: BCP manda single-part HTML
    textHtml,
    date: 'Fri, 28 Aug 2026 17:46:22 +0000 (UTC)',
    metadata: { 'message-id': '<XL2a-vULSFKpWboLyZpwZA@geopod-ismtpd-14>' },
    attributes: { uid: 85536 },
  };
}

const HTML_OK = '<p>Realizaste un consumo de <b>S/ 399.00</b> con tu <b>Tarjeta de Crédito BCP</b> en <b>MP*SOLEPERU.</b></p>';
// El mismo cuerpo tal como llega de verdad: el nodo IMAP decodifica los bytes
// UTF-8 como Latin-1, así que la "é" aparece como los dos caracteres C3 A9.
const HTML_MOJIBAKE = HTML_OK.replace(/é/g, 'Ã©');

test('acepta el header From completo, no solo la dirección pelada', () => {
  // El bug original: el nodo IMAP manda 'Nombre <dirección>' y el código,
  // escrito para el Gmail Trigger, lo comparaba entero contra una lista de
  // direcciones peladas. Nunca coincidía y el lote salía vacío.
  const emails = correrNodo([
    correoImap({ from: 'BCP Notificaciones <notificaciones@notificacionesbcp.com.pe>', textHtml: HTML_OK }),
  ]);
  assert.equal(emails.length, 1, 'el correo se descartó: el remitente no coincidió');
  assert.equal(emails[0].sender, 'notificaciones@notificacionesbcp.com.pe');
});

test('sigue aceptando la dirección pelada (formato del Gmail Trigger)', () => {
  const emails = correrNodo([
    correoImap({ from: 'notificaciones@notificacionesbcp.com.pe', textHtml: HTML_OK }),
  ]);
  assert.equal(emails.length, 1);
  assert.equal(emails[0].sender, 'notificaciones@notificacionesbcp.com.pe');
});

test('repara el mojibake del cuerpo antes de mandarlo al parser', () => {
  // Segundo bug: sin reparar, "Crédito" llega como "CrÃ©dito" y los regex de
  // crates/email-parse (que buscan "Cr[ée]dito") no matchean, así que el
  // correo termina en discarded_events como si fuera propaganda.
  const [email] = correrNodo([
    correoImap({ from: 'BCP <notificaciones@notificacionesbcp.com.pe>', textHtml: HTML_MOJIBAKE }),
  ]);
  assert.match(email.text, /Tarjeta de Crédito BCP/);
  assert.doesNotMatch(email.text, /CrÃ©dito/, 'quedó mojibake sin reparar');
});

test('no toca un cuerpo que ya venía bien', () => {
  // La reparación solo debe actuar ante la firma del mojibake. Si se aplicara
  // a ciegas, corrompería el texto correcto — de ahí el TextDecoder en modo
  // fatal, que devuelve el original cuando la conversión no da UTF-8 válido.
  const [email] = correrNodo([
    correoImap({ from: 'BCP <notificaciones@notificacionesbcp.com.pe>', textHtml: HTML_OK }),
  ]);
  assert.match(email.text, /Tarjeta de Crédito BCP/);
  assert.match(email.text, /MP\*SOLEPERU/);
});

test('saca el message-id de metadata: es la clave de deduplicación', () => {
  // El nodo IMAP no expone message-id arriba del todo. Si esto se rompe,
  // gmail_msg_id va vacío para todos y colisionan entre sí en la base
  // (unique user_id + gmail_msg_id), quedando una sola fila.
  const [email] = correrNodo([
    correoImap({ from: 'BCP <notificaciones@notificacionesbcp.com.pe>', textHtml: HTML_OK }),
  ]);
  assert.equal(email.gmail_msg_id, '<XL2a-vULSFKpWboLyZpwZA@geopod-ismtpd-14>');
  assert.equal(email.received_at, '2026-08-28T17:46:22.000Z');
});

test('descarta lo que no es de un banco conocido', () => {
  const emails = correrNodo([
    correoImap({ from: 'Ofertas <promos@tienda.com>', textHtml: HTML_OK }),
  ]);
  assert.equal(emails.length, 0);
});
