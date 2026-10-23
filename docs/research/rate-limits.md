# Поведенческая спека: rate limits

Источник: публичная документация Discord «Rate Limits».

## Поведение оригинала

- Лимиты по «бакетам»: маршрут + major-параметр (channel_id/guild_id/webhook).
- Заголовки ответа: limit, remaining, reset, reset-after, bucket, scope.
- 429 с телом `{ retry_after, global }`.
- Глобальный лимит на токен (~50 запросов/с).
- Слишком много 401/403/429 с одного IP ⇒ временный бан IP (защита от абуза).

## Решение NeoShard

- Алгоритм GCRA (generic cell rate algorithm) — один ключ Redis на бакет,
  атомарно, без гонок. В режиме одного узла — in-memory (`governor`-подобная
  реализация в `server/api/src/ratelimit.rs`).
- Заголовки: `RateLimit-Limit`, `RateLimit-Remaining`, `RateLimit-Reset`
  (IETF draft-ietf-httpapi-ratelimit-headers) + `Retry-After`.
- Бакеты: `auth:*` (жёстко, на IP), `msg:create:{channel}` (5/5с на пользователя),
  `global:{user}` 50/с.
- Ядро клиента (`neoshard-core`) уважает `Retry-After` и сам ставит запросы в
  очередь — UI об этом не знает.
