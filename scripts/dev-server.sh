#!/usr/bin/env bash
# Run a local NeoShard server for development (needs PostgreSQL; see docs/deploy.md).
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/env.sh
export DATABASE_URL="${DATABASE_URL:-postgres://neoshard:neoshard@127.0.0.1:5432/neoshard}"
# Dev-only key; production keys come from the deployment secrets.
export NEOSHARD_SECRET_KEY="${NEOSHARD_SECRET_KEY:-AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=}"
export NEOSHARD_PUBLIC_URL="${NEOSHARD_PUBLIC_URL:-http://localhost:5173}"
export NEOSHARD_BIND="${NEOSHARD_BIND:-127.0.0.1:8080}"
export NEOSHARD_DEV=true
# Avatars, banners and attachments without S3 (served at /files/*).
export NEOSHARD_FILES_DIR="${NEOSHARD_FILES_DIR:-$PWD/data/files}"
exec cargo run -p neoshard-server --release
