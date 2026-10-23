# ADR-0003: neoshard-core — ядро клиента по модели TDLib
- Статус: принято · 2026-10-18
## Контекст
Telegram-форки живут потому, что TDLib отделяет всю логику от UI.
## Решение
Crate `neoshard-core` без UI. Публичный API — команды (`Command`) и события (`Update`), оба protobuf.
Транспорт и хранилище — трейты: WASM → `web-sys` WebSocket + IndexedDB; native → tokio-tungstenite + SQLite;
mobile → uniffi поверх native. Модули: `gateway` (reconnect/resume/heartbeat — чистая state-machine,
тестируется без сети), `cache`, `outbox` (оптимистичные отправки с nonce), `permissions` (из proto), `ratelimit`,
`e2ee` (feature), `audio` (feature). SemVer; TS-типы генерируются из `.proto` (`@neoshard/core`).
## Последствия
Логика тестируется в Rust один раз. Цена — сложнее сборка (wasm-bindgen), закрывается скриптами.
