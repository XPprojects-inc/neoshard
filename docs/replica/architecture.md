# NeoShard — архитектура

Стек задан требованиями проекта (модель TDLib). Дефолтный стек replica-architect
(Next.js/Supabase/Vercel) **не используется**: React запрещён, а зарубежные SaaS
противоречат 152-ФЗ. Каждое решение — ADR в `docs/adr/`.

## 1. Стек

| слой | выбор | почему | ADR |
|---|---|---|---|
| протокол | `neoshard-proto` (Rust, prost) + `spec/` (proto, OpenAPI, AsyncAPI) | один источник истины для сервера и ядра | 0004 |
| сервер | Rust, axum + tokio, Cargo workspace | нет GC-пауз, 100k WS на узел | 0002 |
| ядро клиента | `neoshard-core` (Rust) → WASM / native / uniffi | бизнес-логика пишется один раз, форки меняют только UI | 0003 |
| UI | SolidJS + TypeScript + Vite | fine-grained reactivity, маленький бандл | 0007 |
| анимации | `motion` (vanilla) + свои Solid-примитивы | React запрещён | 0008 |
| десктоп | Tauri 2 | <15 МБ, ядро нативно | 0016 |
| мобильные | PWA → uniffi (Kotlin/Swift) | | 0016 |
| БД | PostgreSQL 16; сообщения — hash-партиционированная таблица | одна СУБД для self-host; Scylla — адаптер позже | 0006 |
| кэш/presence/limits | Valkey (Redis-совместимый, BSD) | | 0018 |
| шина | NATS (+JetStream для resume в кластере) | | 0002 |
| медиа | LiveKit SFU + coturn (443/TCP+TLS) | | 0012 |
| файлы | S3-совместимое (Yandex Object Storage / MinIO) | | 0011 |
| E2EE | MLS (openmls) | | 0013 |
| ID | Snowflake i64 (epoch 2026-01-01) | сортируемы, без координации, компактны в protobuf | 0005 |
| auth | argon2id + opaque session tokens (SHA-256 в БД) + TOTP | отзыв сессий мгновенный, без JWT-ротаций | 0017 |
| хостинг | Yandex Cloud, резерв Selectel/VK Cloud; без Cloudflare в критическом пути | 152-ФЗ, доступность из РФ | 0011 |

## 2. Схема

`server/migrations/0001_init.sql`. 22 таблицы. Авторизация — **одна проверка на
запрос в слое данных** (`server/api/src/authz.rs`) через `neoshard_proto::permissions`,
не RLS (сервер — единственный клиент БД).

Ключевые инварианты, решённые в БД:
- уникальность `username` — `unique (lower(username))`;
- одна реакция на (сообщение, эмодзи, пользователь) — PK;
- инвайт: `uses <= max_uses` проверяется атомарным `UPDATE … WHERE uses < max_uses RETURNING`;
- `messages` партиционирована `PARTITION BY HASH (channel_id)` на 16 частей,
  индекс `(channel_id, id DESC)` — пагинация истории без сортировки.

## 3. API

Полный список — `spec/openapi.yaml`. Gateway — `spec/asyncapi.yaml` + `spec/proto/gateway.proto`.

| method path | что | кто | flow |
|---|---|---|---|
| GET /.well-known/neoshard | эндпоинты инстанса, версия протокола | все | F13 |
| POST /api/v1/auth/register | регистрация | все (rate-limit по IP) | F01 |
| POST /api/v1/auth/login | вход (→ mfa_ticket при 2FA) | все | F02 |
| POST /api/v1/auth/mfa/totp | второй фактор | держатель ticket | F02 |
| POST /api/v1/auth/mfa/enable / disable | включить 2FA | пользователь | — |
| GET/DELETE /api/v1/sessions[/:id] | сессии устройств | пользователь | — |
| POST /api/v1/auth/recover, /recover/confirm | восстановление | все | — |
| POST/GET/PATCH/DELETE /api/v1/guilds[/:id] | гильдии | MANAGE_GUILD/owner | F03 |
| POST/PATCH/DELETE /api/v1/guilds/:id/channels, /channels/:id | каналы | MANAGE_CHANNELS | F08 |
| PUT/DELETE /api/v1/channels/:id/overwrites/:target | overwrites | MANAGE_ROLES | F08 |
| POST/PATCH/DELETE /api/v1/guilds/:id/roles[/:rid] | роли | MANAGE_ROLES + иерархия | F08 |
| PUT/DELETE /api/v1/guilds/:id/members/:uid/roles/:rid | выдача ролей | MANAGE_ROLES + иерархия | F08 |
| DELETE /api/v1/guilds/:id/members/:uid | кик | KICK_MEMBERS | F09 |
| PUT/DELETE /api/v1/guilds/:id/bans/:uid | бан | BAN_MEMBERS | F09 |
| PATCH /api/v1/guilds/:id/members/:uid {timeout_until} | тайм-аут | MODERATE_MEMBERS | F09 |
| GET /api/v1/guilds/:id/audit-log | аудит | VIEW_AUDIT_LOG | F09 |
| POST /api/v1/channels/:id/invites, POST /api/v1/invites/:code | инвайты | CREATE_INVITE / все | F03, F04 |
| GET/POST /api/v1/channels/:id/messages | история / отправка | READ_HISTORY / SEND | F05 |
| PATCH/DELETE /api/v1/channels/:id/messages/:mid | правка/удаление | автор / MANAGE_MESSAGES | F06 |
| PUT/DELETE …/messages/:mid/reactions/:emoji | реакции | ADD_REACTIONS | F06 |
| PUT/DELETE /api/v1/channels/:id/pins/:mid | закрепы | MANAGE_MESSAGES | F06 |
| POST /api/v1/channels/:id/threads | треды | CREATE_THREADS | F07 |
| GET /api/v1/guilds/:id/messages/search | поиск | VIEW_CHANNEL на каждый канал | — |
| POST /api/v1/users/@me/channels | ЛС / групповые ЛС | пользователь | F11 |
| POST /api/v1/channels/:id/voice-token | токен LiveKit | CONNECT | F10 |
| POST /api/v1/webhooks/:id/:token | вебхук | держатель токена | F12 |
| POST /api/v1/applications/:id/commands | slash-команды | владелец бота | F12 |
| POST /federation/v1/* | федерация (подписанные запросы) | инстансы | — |

## 4. Что кусается

- **Realtime/resume:** порядок `seq` на сессию, буфер реплея; в кластере — fan-out через NATS по subject `guild.{id}`.
- **Гонки:** одновременное редактирование ролей — `position` пересчитывается транзакционно.
- **Rate limits:** GCRA, Retry-After, ядро клиента ставит в очередь.
- **Файлы:** presigned PUT в S3, лимит 25 МБ по умолчанию (конфиг), проверка MIME сервером.
- **Поиск:** Postgres FTS (`tsvector`, `simple` + `russian`), GIN-индекс; Meilisearch — опционально.
- **Оффлайн:** очередь оптимистичных отправок в ядре с `nonce` → сервер идемпотентен по `(author_id, nonce)`.
- **Multi-tenancy:** изоляция по guild_id в каждой проверке прав.
- **Удаление данных (152-ФЗ / GDPR):** `DELETE /users/@me` — анонимизация автора сообщений, удаление ПД.

## 5. Порядок сборки

1. Вертикальный срез: register → create guild → send message → получить по Gateway.
2. Must-haves по областям (accounts → permissions → messages → realtime → media → platform).
3. Should/could.
