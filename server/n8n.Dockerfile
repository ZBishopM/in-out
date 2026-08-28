# Patches the one missing config value that lets the native IMAP trigger
# (n8n-nodes-base.emailReadImap) hang forever on a dead connection instead of
# detecting and recovering from it.
#
# Root cause (read directly from the installed source, 2026-08-28): n8n never
# passes `socketTimeout` when it opens the IMAP connection, so the underlying
# mscdex `imap` library defaults it to 0 -- which explicitly DISABLES the
# socket's idle-timeout event. That event's handler already exists in the
# library and already does the right thing (cleanly closes + reports the
# error); it's just switched off. Without it, a connection that goes "TCP
# still ACKing, IMAP server behind it gone" (typical for a long-lived
# connection crossing a NAT/load balancer for hours) is invisible forever: no
# error, no close event, and `forceReconnect`'s own reconnect attempt hangs on
# its first un-timeout-boxed round-trip instead of ever reaching its own catch
# block. Only a full process restart ever cleared it -- twice, ~22h and
# unknown duration, before this.
#
# 600000ms (10 min): the library's own internal IDLE-renewal keepalive writes
# real traffic every 5 min on a healthy connection, so 10 min gives comfortable
# margin against false positives while still cutting a real hang down from
# "forever" to minutes.
FROM n8nio/n8n:2.31.4
USER root
# pnpm keeps stale, unused duplicate copies of a package under different
# .pnpm/<hash> dirs (confirmed: this image ships two for n8n-nodes-base).
# `find | head -1` picks whichever one the filesystem happens to list first,
# which patched the WRONG copy the first time this Dockerfile was written --
# the fix silently did nothing. Resolving node_modules/n8n-nodes-base's own
# symlink first guarantees this patches the exact file Node actually
# `require()`s, not a look-alike sitting unused next to it.
RUN base=$(readlink -f /usr/local/lib/node_modules/n8n/node_modules/n8n-nodes-base) \
    && f="$base/dist/nodes/EmailReadImap/v2/EmailReadImapV2.node.js" \
    && test -f "$f" \
    && sed -i "s/authTimeout: 20000,/authTimeout: 20000, socketTimeout: 600000,/" "$f" \
    && grep -q "socketTimeout: 600000" "$f"
USER node
