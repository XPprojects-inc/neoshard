#!/usr/bin/env bash
# Builds the package for install.ps1: deploy/windows/pack.sh <domain> <out.zip>
# Needs: a release build of neoshard-server (x86_64-pc-windows), PostgreSQL for Windows
# (PGSQL_DIR, default: scoop's), livekit-server.exe (LIVEKIT_EXE) and caddy.exe (CADDY_EXE).
set -euo pipefail
cd "$(dirname "$0")/../.."
domain="$1"; out="$(realpath -m "$2")"
: "${PGSQL_DIR:=$HOME/scoop/apps/postgresql/current}"
: "${LIVEKIT_EXE:?path to livekit-server.exe}"
: "${CADDY_EXE:?path to caddy.exe}"

. scripts/env.sh >/dev/null
cargo build -p neoshard-server --release
NEOSHARD_WEB_BASE=app VITE_NEOSHARD_INSTANCE="https://$domain" npm run build -w @neoshard/web

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage"/{bin,livekit,caddy,scripts,pgsql,site}
cp target/release/neoshard-server.exe "$stage/bin/"
cp -r apps/web/dist "$stage/web"
# Landing site (downloads/ there is only a local preview stub)
(cd apps/site && find . -path ./downloads -prune -o -type f -print | while read -r f; do mkdir -p "$stage/site/$(dirname "$f")"; cp "$f" "$stage/site/$f"; done)
cp "$LIVEKIT_EXE" "$stage/livekit/"
cp "$CADDY_EXE" "$stage/caddy/"
cp deploy/windows/run.ps1 "$stage/scripts/"
for d in bin lib share; do cp -r "$PGSQL_DIR/$d" "$stage/pgsql/"; done
rm -f "$out"
(cd "$stage" && powershell -NoProfile -Command "Compress-Archive -Path * -DestinationPath '$(cygpath -w "$out")'")
ls -la "$out"
