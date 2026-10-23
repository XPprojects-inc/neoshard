# Проверка медиапути с настоящим LiveKit

`livekit-e2e.mjs` проверяет связку NeoShard → LiveKit без моков, через официальный Node SDK LiveKit (`@livekit/rtc-node`):

1. NeoShard выдаёт токены LiveKit (`POST /channels/{id}/voice`): имя комнаты, TURN-учётка, identity = id пользователя.
2. Два участника входят в комнату. Один публикует микрофон (синус 440 Гц), камеру и демонстрацию экрана.
   Второй получает все три дорожки: звук с RMS > 1000, кадры видео нужного размера.
3. Overwrite канала запрещает @everyone SPEAK и STREAM. Новый грант разрешает только камеру,
   и **сам LiveKit** отклоняет публикацию микрофона и экрана (`NOT_ALLOWED`), а камеру пропускает.
4. Без CONNECT токен не выдаётся (403).

```bash
livekit-server --dev                    # https://github.com/livekit/livekit/releases, ключи devkey/secret
LIVEKIT_URL=ws://127.0.0.1:7880 LIVEKIT_API_KEY=devkey LIVEKIT_API_SECRET=secret bash scripts/dev-server.sh
cd tests/media && npm install && npm test
```

`publish.mjs` подключается к голосовому каналу и публикует тон и тестовую картинку, чтобы вручную
проверить клиент: `NEOSHARD_TOKEN=… node publish.mjs <id-канала> 60`.

Каталог не входит в npm-workspaces, и его зависимости не попадают в бандл приложения.
