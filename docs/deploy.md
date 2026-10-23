# Развёртывание NeoShard

Всё работает на **443**: веб-клиент, REST, Gateway (WebSocket) за одним TLS-терминатором
(Caddy). Голос — LiveKit (UDP 7882, 50000–50100, TCP 7881) и coturn (3478, 5349;
для сетей «только 443» — отдельный IP с TURN/TLS на 443).

## Вариант 1. Один сервер, одной командой (docker compose)

```bash
git clone https://github.com/neoshard/neoshard && cd neoshard/deploy
cp .env.example .env        # домен, секреты (openssl rand -base64 32), S3, SMTP
docker compose up -d --build                         # ядро: caddy, web, server, postgres, nats, valkey, livekit, coturn
docker compose --profile monitoring --profile backup --profile local-s3 up -d   # + Prometheus/Grafana, бэкапы, MinIO
```

DNS: A-запись домена → IP сервера. Caddy сам получит TLS-сертификат.
Миграции БД применяются сервером при старте. Обновление — `git pull && docker compose up -d --build`.

## Вариант 2. Yandex Cloud (официальный инстанс, 152-ФЗ)

`deploy/terraform/yandex`: VPC, ВМ (docker compose через cloud-init), Yandex Managed
PostgreSQL 16 (бэкапы 14 дней), Object Storage для файлов, статический IP, группа безопасности.

```bash
export YC_TOKEN=$(yc iam create-token) YC_CLOUD_ID=... YC_FOLDER_ID=...
cd deploy/terraform/yandex
terraform init
terraform apply -var folder_id=$YC_FOLDER_ID -var domain=chat.example.ru \
  -var ssh_public_key="$(cat ~/.ssh/id_ed25519.pub)"
```

Все данные — в регионе ru-central1. Внешних сервисов с персональными данными нет:
метрики — свой Prometheus, логи — stdout (JSON) → свой Loki/ELK при желании,
push — свой Web Push (VAPID). Cloudflare — только как опциональный CDN для
зарубежной аудитории (ADR-0011). Резерв: те же образы и compose на Selectel / VK Cloud.

> В этом окружении ключей Yandex Cloud не было, поэтому `terraform apply` не
> выполнялся; конфигурация проверена `docker compose config`, Helm-чарт и
> Terraform — по структуре (см. PROGRESS.md).

## Вариант 3. Kubernetes (Helm)

```bash
kubectl create secret generic neoshard-secrets \
  --from-literal=DATABASE_URL=postgres://... --from-literal=NEOSHARD_SECRET_KEY=... \
  --from-literal=LIVEKIT_API_SECRET=... --from-literal=NEOSHARD_TURN_SECRET=... \
  --from-literal=S3_ACCESS_KEY=... --from-literal=S3_SECRET_KEY=...
helm install neoshard deploy/helm/neoshard --set domain=chat.example.ru
```

Серверы — StatefulSet (worker id для snowflake = номер пода), события между
узлами — NATS (`NATS_URL`), сборка с `--features nats`.

## Мониторинг и бэкапы

- `/gateway/metrics` (Prometheus): соединения, сессии, события in/out, отключённые
  медленные клиенты, онлайн. Дашборд Grafana — `deploy/grafana/dashboards/neoshard.json`.
- Бэкапы: `pg_dump` раз в сутки (`--profile backup`), хранение `BACKUP_KEEP_DAYS`;
  копию в Object Storage — `aws s3 sync /var/lib/docker/volumes/neoshard_backups/_data s3://bucket --endpoint-url https://storage.yandexcloud.net`.
  Восстановление: `pg_restore -d neoshard neoshard-<дата>.dump`.

## Сетевые ограничения

- Клиент знает только базовый URL инстанса; эндпоинты берёт из `/.well-known/neoshard`,
  пользователь может указать свой сервер на экране входа.
- TURN: временные учётные данные выдаёт `GET /api/v1/turn` (coturn `use-auth-secret`),
  включите `turns:` на 443 для сетей, где закрыт UDP.

## Минимальные требования

1 vCPU / 2 ГБ — сообщество до ~2 000 онлайн. Gateway: ~24 КБ памяти на соединение
(замер — `docs/benchmarks.md`).
