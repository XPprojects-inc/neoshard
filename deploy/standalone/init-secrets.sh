#!/bin/sh
# First start only: random secrets for this server, kept in the "secrets" volume.
# Nothing here ever leaves the machine.
set -eu
dir=/secrets
if [ ! -s "$dir/env" ]; then
  hex() { head -c "$1" /dev/urandom | od -An -tx1 | tr -d ' \n'; }
  db="$(hex 24)"
  umask 022
  cat > "$dir/env.tmp" <<VARS
DB_PASSWORD=$db
NEOSHARD_SECRET_KEY=$(head -c 32 /dev/urandom | base64 | tr -d '\n')
LIVEKIT_API_KEY=NS$(hex 8)
LIVEKIT_API_SECRET=$(hex 32)
VARS
  printf %s "$db" > "$dir/db_password"
  mv "$dir/env.tmp" "$dir/env"
  echo "secrets generated"
else
  echo "secrets already present"
fi
# The server and PostgreSQL run as other users; the volume is not shared outside the stack.
chmod 0444 "$dir/env" "$dir/db_password"
