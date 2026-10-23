# ADR-0008: Анимации — vanilla `motion` + собственные Solid-примитивы
- Статус: принято · 2026-10-18
## Контекст
Навыки motion написаны под `motion/react`; React запрещён.
## Проверка готовых обёрток (npm, 2026-10-18)
- `solid-motionone@1.0.4` — на `@motionone/dom@10` (предыдущее поколение Motion One), а не на `motion@14`;
  даёт `<Motion>` и `<Presence>`, **но не layout/FLIP, drag, reorder**.
- `@solid-primitives/presence@0.1.4` — только состояние mount/unmount, без анимаций.
Ни одна не покрывает четыре пункта → пишем свои.
## Решение
`packages/ui/src/motion/`: `tokens.ts`, `Presence`, `layout()` (FLIP + shared element по `layoutId`), `drag()`
(Pointer Events, constraints, инерция, spring-возврат), `reorder()` (поверх drag+layout). Всё через `animate()`
из `motion`, только transform/opacity, `prefers-reduced-motion`. Тесты — vitest + jsdom.
