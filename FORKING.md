# Свой форк NeoShard за 10 минут

NeoShard устроен как экосистема Telegram: открытый протокол, общее ядро
(`neoshard-core`, аналог TDLib) и сколько угодно клиентов поверх. Три уровня форка:

## 1. Ребрендинг веб/десктоп-клиента (≈10 минут)

1. Форкните репозиторий.
2. `apps/web/neoshard.config.json`: `name`, `logo`, `defaultInstance` (ваш сервер),
   `allowCustomInstance`, `features`.
3. Логотип: замените `apps/web/public/favicon.svg` и `packages/ui/assets/logo.svg`;
   иконки десктопа — `npx tauri icon your-logo.svg` в `apps/desktop`.
4. Тема: скопируйте `packages/ui/themes/neoshard-dark.json`, поменяйте цвета, укажите
   имя в `defaultTheme`. Контраст проверяется
   `python docs/replica/design/contrast.py` (роли — в `docs/replica/design.md`).
5. Десктоп: `productName` и `identifier` в `apps/desktop/src-tauri/tauri.conf.json`.
6. `npm install && npm run proto && npm run wasm && npm run build`.

Плагины пользователей (по образцу exteraGram) работают и в форке: изолированный
iframe без доступа к токену и сети, код закреплён SHA-256 (`apps/web/src/plugins/host.ts`).

## 2. Свой UI на любом стеке поверх ядра

Ядро — это два вызова: отправить `Command`, забрать `UpdateBatch` (оба — protobuf,
`spec/proto/neoshard/v1/core.proto`). Всё остальное — reconnect, resume, офлайн-кэш,
очередь отправки, права — уже внутри.

| стек | как подключить |
|---|---|
| Web (любой фреймворк) | `@neoshard/core`: `NeoShardCore.loadWasm()`, `core.send({case:…})`, `core.subscribe(updates => …)` |
| Rust (GPUI, Iced, Slint, egui, TUI) | `neoshard_core::native::NativeClient::spawn(…)`; пример — `crates/examples/neoshard-cli` (≈150 строк) |
| Kotlin / Swift | `cargo build -p neoshard-core --features ffi` + uniffi-bindgen → `NeoShardClient.dispatch()/poll()` (`docs/mobile.md`) |
| Tauri | `apps/desktop` — ядро нативно, тот же веб-UI |

## 3. Клиент без ядра, прямо по протоколу

`spec/` — нормативная спецификация (CC-BY-4.0): `.proto`, `openapi.yaml`,
`asyncapi.yaml`, `gateway.md`, `permissions.md`, `federation.md`,
`COMPATIBILITY.md`. Пример на Python — `examples/python-client` (≈120 строк).
Такой клиент может иметь любую лицензию.

## Правила

- Клиенты на базе `neoshard-core` — GPL-3.0: исходники форка должны быть открыты.
- Не называйте форк «NeoShard» и не используйте логотип NeoShard как свой — это
  вводит пользователей в заблуждение; «основано на NeoShard» — пожалуйста.
- Протокол стабилен в пределах `v1` (только добавления, см. `spec/COMPATIBILITY.md`):
  форк, собранный сегодня, будет работать с серверами завтра.
