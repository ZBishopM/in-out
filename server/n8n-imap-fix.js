// Parches al nodo IMAP que trae n8n, aplicados al construir la imagen.
//
// El nodo nativo (EmailReadImap v2) tiene dos fallos que nos costaron dos
// caidas largas de la ingesta de correos del banco:
//
//   a) nunca configura socketTimeout, asi que una conexion "TCP viva,
//      aplicacion muerta" no se detecta jamas;
//   b) si un reconecte falla, se traga el error y deja `connection` apuntando
//      al objeto muerto. El siguiente intento muere en la primera linea, antes
//      de reconectar, y repite para siempre cada forceReconnect. Como el error
//      se lo traga, n8n no se entera y no re-activa el workflow: solo lo cura
//      reiniciar el contenedor.
//
// Bug de n8n sin arreglar aguas arriba: https://github.com/n8n-io/n8n/issues/30871
//
// Cada edicion exige encontrar su anclaje EXACTAMENTE una vez. Si n8n cambia
// el archivo en una version futura, el build falla en vez de desplegar un
// parche que no aplico -- que es justo lo que nos paso una vez con un `sed`
// que acerto en una copia obsoleta de pnpm y no en la que se ejecutaba.

const fs = require('fs');
const path = require('path');

// realpath: la ruta real lleva un hash de pnpm que cambia entre builds aunque
// el tag de la imagen sea el mismo. El symlink es lo unico estable.
const base = fs.realpathSync('/usr/local/lib/node_modules/n8n/node_modules/n8n-nodes-base');
const archivo = path.join(base, 'dist/nodes/EmailReadImap/v2/EmailReadImapV2.node.js');

const ediciones = [
  {
    nombre: 'socketTimeout',
    // El sufijo `onMail:` distingue la config del trigger de la del test de
    // credencial, que tiene un authTimeout identico con otra indentacion.
    de: `                    authTimeout: 20000,
                },
                onMail: async (numEmails) => {`,
    a: `                    authTimeout: 20000,
                    socketTimeout: 600000,
                },
                onMail: async (numEmails) => {`,
  },
  {
    nombre: 'handleReconnect',
    de: `            try {
                isCurrentlyReconnecting = true;
                if (connection.closeBox)
                    await connection.closeBox(false);
                connection.end();
                connection = await establishConnection();
                await connection.openBox(mailbox);
            }
            catch (error) {
                this.logger.error(error);
            }`,
    a: `            try {
                isCurrentlyReconnecting = true;
                // Cerrar la conexion vieja es "mejor esfuerzo": si ya esta
                // muerta, closeBox() lanza "No mailbox is currently selected"
                // y, sin este try interno, ese throw aborta el reconecte ANTES
                // de intentarlo, dejando \`connection\` apuntando al objeto
                // muerto. El siguiente tick vuelve a morir en la misma linea:
                // bucle infinito que solo rompe un reinicio del contenedor.
                try {
                    if (connection?.closeBox)
                        await connection.closeBox(false);
                    connection?.end();
                }
                catch (errorCierre) {
                    this.logger.debug(\`Email Read Imap: cierre de la conexion vieja fallo, se ignora: \${errorCierre.message}\`);
                }
                // Sin esto, onMail podria usar la conexion muerta durante la
                // ventana entre end() y el reconecte.
                connection = undefined;
                connection = await establishConnection();
                await connection.openBox(mailbox);
            }
            catch (error) {
                this.logger.error(error);
                // Si el reconecte falla de verdad, EMITIRLO en vez de tragarlo:
                // esto es lo que convierte la caida en un evento. emitError
                // dispara la re-activacion del workflow -- el mismo camino que
                // ya funciona en conn.on('close') -- y hace saltar el Error
                // Trigger, que es quien avisa a Discord.
                connection = undefined;
                this.emitError(error);
            }`,
  },
  {
    nombre: 'closeFunction',
    // Obligatorio junto con la edicion anterior: ahora `connection` puede ser
    // undefined cuando n8n desactiva el workflow, y sin las guardas esto
    // lanzaria un TypeError nuevo al cerrar.
    de: `            try {
                if (connection.closeBox)
                    await connection.closeBox(false);
                connection.end();
            }
            catch (error) {
                throw new n8n_workflow_1.TriggerCloseError(this.getNode(), { cause: error, level: 'warning' });`,
    a: `            try {
                if (connection?.closeBox)
                    await connection.closeBox(false);
                connection?.end();
            }
            catch (error) {
                throw new n8n_workflow_1.TriggerCloseError(this.getNode(), { cause: error, level: 'warning' });`,
  },
];

let fuente = fs.readFileSync(archivo, 'utf8');
for (const ed of ediciones) {
  const veces = fuente.split(ed.de).length - 1;
  if (veces !== 1) {
    throw new Error(`${ed.nombre}: se esperaba 1 coincidencia del anclaje, hubo ${veces}. ` +
      `n8n probablemente cambio ${path.basename(archivo)}; revisa el parche antes de desplegar.`);
  }
  fuente = fuente.replace(ed.de, ed.a);
}
fs.writeFileSync(archivo, fuente);
console.log(`parcheado ${archivo}: ${ediciones.map((e) => e.nombre).join(', ')}`);
