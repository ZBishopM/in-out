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

set -euo pipefail

ENV_FILE="/home/bicho/in-out/server/.env"
STATE_FILE="/home/bicho/in-out/server/.watchdog-state"
STALE_DIAS=3
STALE_HORAS_SUAVE=6
REALERTA_HORAS=12
IMAP_VENTANA_MIN=20
CONTENEDORES=(server-ingest-api-1 server-n8n-1 server-postgres-1)

webhook=$(grep -E '^DISCORD_WEBHOOK_URL=' "$ENV_FILE" | cut -d= -f2-)
if [ -z "$webhook" ]; then
  echo "sin DISCORD_WEBHOOK_URL en $ENV_FILE" >&2
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

# Solo se consulta postgres si el propio contenedor esta arriba -- si no, ya
# se sabe que algo esta mal y consultarlo solo daria un segundo error confuso.
if [ ${#problemas[@]} -eq 0 ]; then
  ultima=$(docker exec server-postgres-1 psql -U inout -d inout -tAc \
    "select coalesce(extract(epoch from now() - max(created_at))::int, -1) from transactions;" 2>/dev/null || echo "")
  if [ -z "$ultima" ]; then
    problemas+=("no se pudo consultar la ultima transaccion en postgres")
  elif [ "$ultima" -lt 0 ]; then
    problemas+=("la tabla transactions esta vacia")
  else
    dias=$((ultima / 86400))
    if [ "$dias" -ge "$STALE_DIAS" ]; then
      problemas+=("sin transacciones nuevas hace **$dias dias** (limite $STALE_DIAS) -- revisa el credential de Gmail en n8n")
    fi
  fi

  # Muerte silenciosa del trigger IMAP, caso 2026-08-28: 22h sin un solo
  # correo -- ni procesado ni descartado -- y SIN ninguna linea de error en el
  # log de n8n. El chequeo de arriba (STALE_DIAS) no lo hubiera visto hasta el
  # dia 3, y el de abajo (el string de error) no aplica porque esta vez no
  # hubo error: la conexion IMAP simplemente dejo de entregar, en silencio,
  # incluso con "Force Reconnect Every Minutes" puesto. Este chequeo es la
  # red que faltaba: umbral en HORAS, no dias, y mira CUALQUIER correo que
  # haya llegado al ingester (raw_events + discarded_events), no solo
  # transacciones -- asi una racha sin gastar pero con correos no-monetarios
  # (OTP, estados de cuenta) no da una falsa alarma, y una racha sin ESE tipo
  # de correo tampoco se disfraza de "no gastaste nada hoy".
  ultimo_correo=$(docker exec server-postgres-1 psql -U inout -d inout -tAc \
    "select coalesce(extract(epoch from now() - greatest(
        (select max(received_at) from raw_events),
        (select max(received_at) from discarded_events)
      ))::int, -1);" 2>/dev/null || echo "")
  if [ -n "$ultimo_correo" ] && [ "$ultimo_correo" -ge 0 ]; then
    horas=$((ultimo_correo / 3600))
    if [ "$horas" -ge "$STALE_HORAS_SUAVE" ]; then
      problemas+=("sin NINGUN correo (procesado o descartado) hace **${horas}h** (limite ${STALE_HORAS_SUAVE}h) -- posible trigger IMAP muerto en silencio, sin error en el log")
    fi
  fi

  # El trigger IMAP de n8n (conexion IDLE) se murio en silencio una vez
  # (2026-08-26): un solo error en el log y despues nada por horas, sin
  # reintentar solo. Se le puso "Force Reconnect Every Minutes"=30 como
  # auto-cura, pero esto avisa mas rapido que esperar $STALE_DIAS por si
  # ese seguro tampoco alcanza.
  if docker logs server-n8n-1 --since "${IMAP_VENTANA_MIN}m" 2>&1 \
       | grep -q "Email Read Imap node encountered an error fetching new emails"; then
    problemas+=("el trigger IMAP tiro un error en los ultimos ${IMAP_VENTANA_MIN} min -- si no se auto-cura (forceReconnect), reiniciar n8n")
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
