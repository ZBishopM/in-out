# Parchea el trigger IMAP nativo de n8n (n8n-nodes-base.emailReadImap), que
# tiene dos fallos que nos dejaron sin ingesta de correos del banco dos veces
# (~22 h y ~17 h), y que solo se curaban reiniciando el contenedor.
#
# El detalle de cada fallo, con el codigo real citado, esta en n8n-imap-fix.js.
# Aqui solo se ejecuta: toda la logica y las comprobaciones viven en el script,
# donde se pueden leer y revisar como codigo en vez de como una tira de `sed`.
#
# La version del tag esta fijada a proposito (no `:latest`): el parche depende
# del texto exacto del archivo de n8n, asi que una actualizacion sorpresa debe
# fallar el build de forma visible, no aplicar a medias.
FROM n8nio/n8n:2.31.4
USER root
COPY n8n-imap-fix.js /tmp/n8n-imap-fix.js
# `node --check` sobre el resultado: si el parche dejara el archivo con un
# error de sintaxis, el build falla aqui en vez de desplegar un n8n que no
# arranca el trigger.
RUN node /tmp/n8n-imap-fix.js \
    && node --check "$(readlink -f /usr/local/lib/node_modules/n8n/node_modules/n8n-nodes-base)/dist/nodes/EmailReadImap/v2/EmailReadImapV2.node.js" \
    && rm /tmp/n8n-imap-fix.js
USER node
