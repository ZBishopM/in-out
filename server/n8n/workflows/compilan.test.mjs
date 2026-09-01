// Todo Code node exportado tiene que COMPILAR. Suena obvio; no lo era.
//
// El workflow "Error Alerts" -- el que avisa a Discord cuando algo falla --
// estuvo 14 ejecuciones seguidas fallando, cero éxitos, porque su Code node no
// compilaba: un '\n' había quedado guardado como un salto de línea REAL dentro
// de una cadena de comilla simple (legal dentro de backticks, ilegal ahí), y
// n8n lo reportaba como `SyntaxError: Invalid or unexpected token` en
// `new Script()`. Nadie se enteró durante días por la peor razón posible: lo
// que estaba roto era justamente el aviso de errores.
//
// Este test es barato y cubre TODOS los workflows del directorio, incluidos
// los que se añadan después. No prueba la lógica de cada nodo -- para eso está
// gmail-ingest.test.mjs -- solo que el código que se despliega es código
// válido.
//
// Sin dependencias: runner nativo de Node. Los .json van por nombre y no por
// carpeta, que en modo directorio el runner intenta cargarlos como tests.
//   node --test server/n8n/workflows/*.test.mjs

import test from 'node:test';
import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const aquí = dirname(fileURLToPath(import.meta.url));
const exports_ = readdirSync(aquí).filter((f) => f.endsWith('.json'));

test('hay workflows que revisar', () => {
  assert.ok(exports_.length > 0, 'no se encontró ningún export .json');
});

for (const archivo of exports_) {
  const crudo = readFileSync(join(aquí, archivo), 'utf8');
  const wf = JSON.parse(crudo);

  for (const nodo of wf.nodes.filter((n) => n.parameters?.jsCode)) {
    test(`${archivo}: el Code node "${nodo.name}" compila`, () => {
      assert.doesNotThrow(
        () => new Function(nodo.parameters.jsCode),
        `no compila; suele ser un salto de línea real dentro de un literal de comillas`,
      );
    });
  }

  // El repo es público: un webhook incrustado en un export es una fuga. Deben
  // ir por entorno ({{ $env.… }}), como hace error-alerts.json.
  test(`${archivo}: no lleva webhooks incrustados`, () => {
    assert.ok(
      !/https:\/\/discord(app)?\.com\/api\/webhooks\//.test(crudo),
      'hay una URL de webhook de Discord escrita a pelo; pásala por $env',
    );
  });
}
