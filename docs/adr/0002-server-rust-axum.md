# ADR-0002: Сервер на Rust (axum + tokio)
- Статус: принято · 2026-10-18
## Контекст
Чат-сервер — десятки тысяч долгоживущих соединений и fan-out. GC-паузы дают хвостовые задержки.
## Решение
axum 0.8 + tokio. Crates в `server/`: `common` (конфиг, БД, auth, шина, ID), `api` (REST), `gateway` (WS),
`presence`, `media-auth` (токены LiveKit), `federation`, бинарник `neoshard-server`. Один процесс поднимает всё
(self-host на одном VPS); флаг `--only gateway|api` — для кластера. Шина событий — трейт `EventBus`:
in-process (`tokio::broadcast`) и NATS (feature `nats`).
## Последствия
Один бинарник для малых инсталляций — ключевое для self-hosting; масштабирование без переписывания.
