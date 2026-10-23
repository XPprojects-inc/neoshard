# Как участвовать

Спасибо! Короткие правила, чтобы ваш PR приняли быстро.

## Перед началом

- Крупные изменения — сначала issue или черновик ADR (`docs/adr/0000-template.md`).
- Протокол меняется только добавлениями (`spec/COMPATIBILITY.md`). Изменение `.proto`
  или REST — это изменение `spec/` в том же PR.

## Окружение

См. «Быстрый старт» в README. На Windows без Visual Studio: `. scripts/env.sh`
(GNU-тулчейн Rust + MSYS2 gcc).

## Правила кода

- **Rust**: `cargo fmt`, `cargo clippy --workspace --all-targets -D warnings`,
  SQL — только через `sqlx::query!` (проверка на этапе компиляции); после изменения
  запросов: `cargo sqlx prepare --workspace -- --all-targets`.
- **TypeScript/Solid**: никакой бизнес-логики в UI — она живёт в `neoshard-core`.
  `npm run typecheck && npm test`.
- **Тесты обязательны**: новая логика — с модульными тестами; новые эндпоинты и
  события Gateway — со сквозным тестом в `server/neoshard-server/tests`.
- **Безопасность**: проверка прав — через `authz.rs`; ничего, что приходит от
  пользователя, не попадает в `innerHTML`, SQL-строки или URL без валидации.
- **Анимации**: только `transform`/`opacity`, токены из `packages/ui/src/motion/tokens.ts`,
  уважать `prefers-reduced-motion`.

## Коммиты и PR

Conventional Commits (`feat(gateway): …`, `fix(core): …`). В описании PR — что и
зачем, как проверено. Подписывая коммит, вы подтверждаете, что имеете право
передать код под лицензией соответствующего каталога (DCO: `git commit -s`).
