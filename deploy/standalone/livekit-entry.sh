#!/bin/sh
# Voice/video SFU. The address it advertises to clients: 127.0.0.1 on localhost,
# otherwise what the domain resolves to (the domain must point at this machine),
# or NEOSHARD_PUBLIC_IP when set.
set -eu
. /secrets/env
ip="${NEOSHARD_PUBLIC_IP:-}"
if [ -z "$ip" ]; then
  if [ -z "${NEOSHARD_DOMAIN:-}" ] || [ "$NEOSHARD_DOMAIN" = localhost ]; then
    ip=127.0.0.1
  else
    ip="$(getent hosts "$NEOSHARD_DOMAIN" 2>/dev/null | awk '{print $1; exit}')"
    [ -n "$ip" ] || ip="$(nslookup "$NEOSHARD_DOMAIN" 2>/dev/null | awk '/^Address/ {a=$NF} END {print a}')"
  fi
fi
export LIVEKIT_KEYS="$LIVEKIT_API_KEY: $LIVEKIT_API_SECRET"
exec /livekit-server --config /etc/livekit.yaml --node-ip "$ip"
