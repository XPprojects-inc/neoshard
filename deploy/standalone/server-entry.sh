#!/bin/sh
# NeoShard server in the standalone stack: local storage, no federation.
set -eu
. /secrets/env
if [ -z "${NEOSHARD_DOMAIN:-}" ] || [ "$NEOSHARD_DOMAIN" = localhost ]; then
  port="${NEOSHARD_HTTP_PORT:-80}"
  [ "$port" = 80 ] && suffix="" || suffix=":$port"
  export NEOSHARD_DOMAIN=localhost
  export NEOSHARD_PUBLIC_URL="http://localhost$suffix"
  export LIVEKIT_URL="ws://localhost:7880"
else
  export NEOSHARD_PUBLIC_URL="https://$NEOSHARD_DOMAIN"
  export LIVEKIT_URL="wss://$NEOSHARD_DOMAIN:7880"
fi
export DATABASE_URL="postgres://neoshard:${DB_PASSWORD}@postgres:5432/neoshard"
export NEOSHARD_SECRET_KEY LIVEKIT_API_KEY LIVEKIT_API_SECRET
exec /usr/local/bin/neoshard-server
