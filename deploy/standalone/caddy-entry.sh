#!/bin/sh
# Plain HTTP on localhost; automatic HTTPS (Let's Encrypt) for a real domain.
set -eu
if [ -z "${NEOSHARD_DOMAIN:-}" ] || [ "$NEOSHARD_DOMAIN" = localhost ]; then
  export NEOSHARD_SITE=":80" NEOSHARD_VOICE_SITE=":7880"
else
  export NEOSHARD_SITE="$NEOSHARD_DOMAIN" NEOSHARD_VOICE_SITE="$NEOSHARD_DOMAIN:7880"
fi
exec caddy run --config /etc/caddy/Caddyfile --adapter caddyfile
