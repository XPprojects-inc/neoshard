# ADR-0012: Медиа — LiveKit (self-hosted SFU) + coturn
- Статус: принято · 2026-10-18
## Решение
Сервер не проксирует медиа. `media-auth` проверяет `CONNECT/SPEAK/STREAM` и выдаёт JWT LiveKit с грантами
(canPublish — только при SPEAK; источник screen_share — только при STREAM). Комната = `voice:{channel_id}`.
coturn: UDP 3478, TCP/TLS 443 на отдельном IP. Клиент: веб — `livekit-client` (ленивый чанк);
native — `livekit` Rust SDK в ядре (feature `media`). Шумоподавление — `nnnoiseless` в ядре
(в вебе — WASM в AudioWorklet).
