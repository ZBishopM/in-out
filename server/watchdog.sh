#!/usr/bin/env bash
# Vigila el pipeline de in_out: contenedores arriba + transacciones nuevas de
# verdad llegando. Corre por cron en el host, NO dentro de n8n -- si n8n mismo
# se cae, un workflow suyo no puede avisar de su propia muerte. Este script
# sigue funcionando aunque n8n, ingest-api o postgres esten caidos, porque solo
# depende de docker y curl.
#
# Un solo aviso por transicion (sano->roto, roto->sano), y un recordatorio
# cada REALERTA_HORAS mientras siga roto -- ni silencio total ni un mensaje
# por cada corrida de cron.
#
# REGLA: solo se avisa de lo que se ha COMPROBADO que esta roto. Nunca de un
# silencio.
#
# Habia dos chequeos que deducian la averia de que no llegaran datos: "sin
# transacciones hace 3 dias" y "sin ningun correo hace 6h". El segundo salto un
# martes a las 5 de la mañana y dijo "posible trigger IMAP muerto". No habia
# nada muerto: no habia compras porque su dueño estaba durmiendo. Ese es el
# fallo de fondo de medir la salud por el silencio -- no hay umbral en horas
# que distinga "el trigger se cayo" de "hoy no gastaste", porque el dato que
# mira es el mismo en los dos casos.
#
# Asi que se fueron los dos, y en su lugar hay una comprobacion de verdad: si
# el trigger IMAP tiene o no un socket abierto contra Gmail. Eso es un hecho
# observable, vale a las 5 de la mañana igual que a mediodia, y no depende de
# que hayas comprado algo.
#
# Lo que queda, entonces, son dos hechos y ninguna corazonada:
#   1. un contenedor que no esta corriendo
#   2. el trigger IMAP sin conexion, confirmado dos veces
#
# El precio, dicho claro: un fallo que deje la conexion viva pero deje de
# guardar (un parser roto, por ejemplo) ya no se detecta aqui. Ese caso toca
# cazarlo del lado del ingest, donde SI es un evento, en vez de adivinarlo
# desde fuera contando horas de silencio.
#
# Siguiente en caer: el chequeo de contenedores, que
# "docker events --filter event=die --filter event=health_status" ya entrega
# como flujo de eventos en vez de como sondeo.

set -euo pipefail

ENV_FILE="/home/bicho/in-out/server/.env"
STATE_FILE="/home/bicho/in-out/server/.watchdog-state"
REALERTA_HORAS=12
# Segundos entre la primera comprobacion del IMAP y la de confirmacion. Un
# reconecte normal dura segundos; con este margen no se avisa por pillarlo
# justo en medio.
CONFIRMAR_TRAS=45
CONTENEDORES=(server-ingest-api-1 server-n8n-1 server-postgres-1)

# Canal de errores y caidas, aparte del de consumos (esos los manda ingest-api
# con DISCORD_WEBHOOK_URL). Con respaldo al de siempre: un .env sin la clave
# nueva debe mandar al canal equivocado, no dejar al watchdog mudo.
webhook=$(grep -E '^DISCORD_ALERTS_WEBHOOK_URL=' "$ENV_FILE" | cut -d= -f2-)
if [ -z "$webhook" ]; then
  webhook=$(grep -E '^DISCORD_WEBHOOK_URL=' "$ENV_FILE" | cut -d= -f2-)
fi
if [ -z "$webhook" ]; then
  echo "sin DISCORD_ALERTS_WEBHOOK_URL ni DISCORD_WEBHOOK_URL en $ENV_FILE" >&2
  exit 1
fi

json_escape() {
  python3 -c 'import json,sys; print(json.dumps(sys.argv[1]))' "$1"
}

enviar() {
  local msg="$1"
  curl -sf -X POST -H "Content-Type: application/json" \
    -d "{\"content\": $(json_escape "$msg")}" \
    "$webhook" >/dev/null
}

problemas=()

for c in "${CONTENEDORES[@]}"; do
  estado=$(docker inspect -f '{{.State.Status}}' "$c" 2>/dev/null || echo "ausente")
  if [ "$estado" != "running" ]; then
    problemas+=("contenedor **$c**: $estado")
  fi
done

# ¿Tiene el trigger IMAP un socket abierto contra Gmail?
#
# Esto es lo que sustituye a los umbrales por tiempo. No se deduce nada: o hay
# una conexion en el 993 o no la hay. Y como el nodo lleva socketTimeout
# puesto (ver server/n8n-imap-fix.js), una conexion zombie -- viva en TCP pero
# muerta por arriba -- se cierra sola en 10 minutos, asi que "hay socket"
# significa de verdad "el trigger esta escuchando".
imap_conectado() {
  docker exec server-n8n-1 sh -c "netstat -tn 2>/dev/null | grep -q ':993 .*ESTABLISHED'"
}

# Solo se mira si los contenedores estan arriba -- si no, ya se sabe que algo
# esta mal y esto solo daria un segundo error confuso.
if [ ${#problemas[@]} -eq 0 ]; then
  if ! imap_conectado; then
    # Segunda opinion antes de avisar: el nodo reconecta cada 30 min por su
    # cuenta, y esa ventana son segundos. Sin esta confirmacion, el cron podria
    # caer justo dentro y avisar de una caida que no existe.
    sleep "$CONFIRMAR_TRAS"
    if ! imap_conectado; then
      problemas+=("el trigger IMAP no tiene conexion con Gmail -- comprobado dos veces con ${CONFIRMAR_TRAS}s de diferencia")
    fi
  fi
fi

ahora=$(date +%s)
estado_previo="ok"
ultima_alerta=0
if [ -f "$STATE_FILE" ]; then
  read -r estado_previo ultima_alerta < "$STATE_FILE" || true
fi

if [ ${#problemas[@]} -gt 0 ]; then
  detalle=$(printf '%s\n' "${problemas[@]}" | sed 's/^/- /')
  if [ "$estado_previo" = "ok" ]; then
    enviar "🔴 **in_out dejo de registrar** ($(date '+%Y-%m-%d %H:%M')):"$'\n'"$detalle"
    echo "roto $ahora" > "$STATE_FILE"
  else
    transcurrido=$(( (ahora - ultima_alerta) / 3600 ))
    if [ "$transcurrido" -ge "$REALERTA_HORAS" ]; then
      enviar "🔴 **in_out sigue roto** ($(date '+%Y-%m-%d %H:%M')):"$'\n'"$detalle"
      echo "roto $ahora" > "$STATE_FILE"
    fi
  fi
else
  if [ "$estado_previo" != "ok" ]; then
    enviar "🟢 **in_out se recupero** ($(date '+%Y-%m-%d %H:%M')) -- contenedores arriba, transacciones al dia."
  fi
  echo "ok $ahora" > "$STATE_FILE"
fi
