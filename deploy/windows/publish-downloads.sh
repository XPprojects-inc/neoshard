#!/usr/bin/env bash
# Publishes app builds to the landing site's download section:
#   deploy/windows/publish-downloads.sh <version> <setup.exe> <app.apk>
# Copies the files to C:\NeoShard\downloads on the server and writes latest.json
# (version, date, size, SHA-256) that apps/site/site.js reads.
# Server access: NEOSHARD_SSH (default below) and the deploy key NEOSHARD_SSH_KEY.
set -euo pipefail
version="$1"; win="$2"; apk="$3"
: "${NEOSHARD_SSH:?user@host of the server, e.g. admin@203.0.113.7}"
: "${NEOSHARD_SSH_PORT:=22}"
: "${NEOSHARD_SSH_KEY:=$HOME/.ssh/neoshard_deploy}"

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
entry() { # platform file
  local name; name="$(basename "$2")"
  cp "$2" "$stage/$name"
  printf '"%s": { "file": "%s", "size": %s, "sha256": "%s" }' \
    "$1" "$name" "$(stat -c %s "$2")" "$(sha256sum "$2" | cut -d' ' -f1)"
}
{
  printf '{ "version": "%s", "date": "%s", "files": {\n  ' "$version" "$(date -u +%Y-%m-%d)"
  entry windows "$win"; printf ',\n  '
  entry android "$apk"
  printf '\n} }\n'
} > "$stage/latest.json"
cat "$stage/latest.json"

ssh_opts=(-i "$NEOSHARD_SSH_KEY" -o BatchMode=yes -o LogLevel=ERROR)
# latest.json goes last, so the page never points at a file that is still uploading.
scp -q -P "$NEOSHARD_SSH_PORT" "${ssh_opts[@]}" "$stage/$(basename "$win")" "$stage/$(basename "$apk")" "$NEOSHARD_SSH:C:/NeoShard/downloads/"
scp -q -P "$NEOSHARD_SSH_PORT" "${ssh_opts[@]}" "$stage/latest.json" "$NEOSHARD_SSH:C:/NeoShard/downloads/"
echo "published $version"
