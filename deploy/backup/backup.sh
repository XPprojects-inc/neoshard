#!/bin/sh
# Nightly logical backup of PostgreSQL; keeps BACKUP_KEEP_DAYS days.
# For off-site copies sync /backups to Object Storage (e.g. `aws s3 sync` with
# --endpoint-url https://storage.yandexcloud.net) from the host.
set -eu
while true; do
  ts=$(date -u +%Y%m%dT%H%M%SZ)
  pg_dump -Fc -f "/backups/neoshard-$ts.dump"
  find /backups -name 'neoshard-*.dump' -mtime "+${BACKUP_KEEP_DAYS:-14}" -delete
  echo "backup neoshard-$ts.dump done"
  sleep 86400
done
