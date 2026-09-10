#!/usr/bin/env bash
# Publica los workflows exportados contra el n8n de ESTE servidor.
#
#   ./publicar.sh              enseña lo que haría y no toca nada  (por defecto)
#   ./publicar.sh --aplicar    lo hace
#
# Se corre desde `server/`, en agapornis.
#
# Por qué existe: "guardar" desde la interfaz de n8n no es lo mismo que
# publicar. Un workflow tiene una versión activa (`activeVersionId`) aparte de
# su fila en `workflow_entity`, y editar solo la fila deja al trigger corriendo
# la versión vieja -- eso ya nos mordió una vez. El CLI de n8n
# (`import:workflow`) hace el camino completo, así que se usa ese en vez de
# escribir SQL a mano contra producción.
#
# Los exports NO llevan `id` (son ficheros de repo, y el id es de esta
# instalación). Se busca por nombre aquí y se inyecta antes de importar: sin
# eso, `import:workflow` crea un workflow NUEVO en vez de actualizar el que ya
# existe, y acabás con dos "Error Alerts".
set -euo pipefail

cd "$(dirname "$0")/.."          # server/
APLICAR=0
[ "${1:-}" = "--aplicar" ] && APLICAR=1

psql() { docker compose exec -T postgres psql -U inout -d inout -tA "$@"; }

id_de() {   # id_de "<nombre>" -> el id, o vacío
  psql -c "select id from workflow_entity where name = '$(printf '%s' "$1" | sed "s/'/''/g")' limit 1;"
}

publicar() {   # publicar <archivo.json> <nombre actual en la base>
  archivo="n8n/workflows/$1"
  nombre="$2"
  id="$(id_de "$nombre" || true)"

  if [ -z "$id" ]; then
    echo "  !! no hay ningún workflow llamado '$nombre' -- me lo salto"
    echo "     (si cambió de nombre, corrígelo aquí antes de aplicar)"
    return
  fi

  activo="$(psql -c "select active from workflow_entity where id = '$id';")"
  echo "  $1  ->  id=$id  activo=$activo"

  if [ "$APLICAR" -eq 0 ]; then
    echo "     (en seco: no se importa)"
    return
  fi

  # El id va inyectado en una COPIA temporal; el fichero del repo no se toca.
  tmp="$(mktemp)"
  jq --arg id "$id" '. + {id: $id}' "$archivo" > "$tmp"
  docker compose cp "$tmp" n8n:/tmp/publicar.json
  docker compose exec -T n8n n8n import:workflow --input=/tmp/publicar.json
  docker compose exec -T n8n rm -f /tmp/publicar.json
  rm -f "$tmp"
  echo "     importado"
}

echo "== workflows =="
publicar error-alerts.json   'Error Alerts'
publicar gmail-ingest.json   'gmail bank mail extractor'

# El backfill se renombra: "My workflow" es el nombre que le puso n8n solo, y no
# se borra porque es lo que usamos para recuperar correos perdidos.
echo
echo "== renombrar el backfill =="
viejo='My workflow'
nuevo='in-out Gmail backfill (run once)'
id_bf="$(id_de "$viejo" || true)"
if [ -n "$id_bf" ]; then
  echo "  '$viejo' (id=$id_bf)  ->  '$nuevo'"
  if [ "$APLICAR" -eq 1 ]; then
    psql -c "update workflow_entity set name = '$nuevo' where id = '$id_bf';" >/dev/null
    echo "     renombrado"
  else
    echo "     (en seco)"
  fi
else
  echo "  ya no hay ningún 'My workflow' -- nada que renombrar"
fi

if [ "$APLICAR" -eq 1 ]; then
  # n8n mantiene los triggers vivos en memoria: sin reiniciar, el IMAP sigue
  # corriendo la versión anterior aunque la base ya tenga la nueva.
  echo
  echo "== reiniciando n8n para que recoja los triggers =="
  docker compose restart n8n
  echo
  echo "comprobar:"
  echo "  docker compose logs -f --tail=50 n8n"
  echo "  docker compose exec -T postgres psql -U inout -d inout -c \\"
  echo "    \"select w.name, e.status, count(*) from execution_entity e\\"
  echo "     join workflow_entity w on w.id = e.\\\"workflowId\\\"\\"
  echo "     group by 1,2 order by 1;\""
else
  echo
  echo "nada tocado. Para aplicarlo:  ./n8n/publicar.sh --aplicar"
fi
