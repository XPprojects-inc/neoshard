# ADR-0018: Rate limiting — GCRA
- Статус: принято · 2026-10-18
## Решение
GCRA: одна метка (TAT) на ключ. In-memory для одного узла; Valkey (Lua) для кластера.
Заголовки `RateLimit-*` + `Retry-After`. См. `docs/research/rate-limits.md`.
