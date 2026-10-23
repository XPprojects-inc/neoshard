# ADR-0011: Хостинг — Yandex Cloud, без Cloudflare в критическом пути, 152-ФЗ
- Статус: принято · 2026-10-18
## Решение
- Официальный инстанс: Yandex Cloud ru-central1 (Compute, Managed PostgreSQL, Object Storage, Cloud CDN).
  Резерв — Selectel / VK Cloud (те же образы, приложение не знает о провайдере).
- Cloudflare — только опциональный CDN для зарубежной аудитории, с health-check и откатом DNS.
- Никакой сторонней аналитики/логов/ошибок за рубежом: Prometheus, Loki, ошибки — собственный эндпоинт (opt-in).
  Push — self-hosted Web Push (VAPID) и UnifiedPush; FCM/APNs — только при явной настройке оператором.
- Сетевые ограничения: всё на 443 за одним TLS-терминатором, TURN/TLS на 443 отдельного IP,
  эндпоинты клиента настраиваемы, `/.well-known/neoshard` для автоконфигурации.
