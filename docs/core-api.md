# neoshard-core API (для форков)

Ядро — sans-IO машина (`crates/neoshard-core/src/core.rs`): `Command` внутрь,
`UpdateBatch` наружу, драйвер исполняет `Effect` (WebSocket, HTTP, сохранение).
Схема — `spec/proto/neoshard/v1/core.proto` (SemVer, только добавления в 0.x-минорах).

| Command | что делает |
|---|---|
| `Login` / `Register` / `SubmitMfa` / `Logout` | сессия; токен хранит ядро (снапшот IndexedDB/SQLite) |
| `Connect` | подключиться готовым токеном (боты, CLI) |
| `SendMessage` | оптимистичная отправка: сразу `PendingMessage`, затем `PendingMessageSent` или `PendingMessageFailed{retryable}`; nonce, ретраи, порядок по каналу |
| `RetryMessage`, `EditMessage`, `DeleteMessage`, `React` | действия с сообщениями |
| `LoadHistory` | из кэша или сервера → `History` |
| `SelectChannel` | ядро считает права → `ChannelPermissions` (и пересчитывает при изменениях) |
| `SetTyping`, `AckChannel`, `SetPresence`, `RequestMembers`, `SetVoiceState` | запросы Gateway |
| `SetNetworkOnline` | офлайн: отправки ждут в очереди, reconnect — по возвращении сети |
| `ApiRequest` | любой REST-вызов (управление гильдией и т.п.) с авторизацией ядра → `ApiResponse` |

`Update`: `AuthState`, `ConnectionStateChanged`, `Ready`, `Event` (все события
Gateway после применения к кэшу), `Pending*`, `History`, `ChannelPermissions`,
`CommandResult`, `ApiResponse`.

Граница JS↔WASM (ADR-0009): `dispatch(bytes)` и `drain() → bytes` раз в кадр;
бенчмарк — `cargo bench -p neoshard-core --bench boundary`, результаты — `docs/benchmarks.md`.
