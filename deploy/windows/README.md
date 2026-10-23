# NeoShard на своём компьютере с Windows

Схема без Docker. Пользователю не нужно входить в систему: всё стартует при загрузке Windows.

| компонент | как запускается | порт |
|---|---|---|
| PostgreSQL | служба `NeoShardPostgres` | 5432, только localhost |
| сервер NeoShard | задача `NeoShard-server` от SYSTEM | 8080, только localhost |
| LiveKit (голос) | задача `NeoShard-livekit` | 7890 (за Caddy), RTC 7881/tcp, 7882 и 50000–50100/udp |
| Caddy (HTTPS: сайт на `/`, веб-клиент на `/app/`, сборки на `/downloads/`) | задача `NeoShard-caddy` | 80, 443, 7880 |

`run.ps1` перезапускает упавший процесс через 5 секунд. Логи пишутся в `C:\NeoShard\logs\`.
Секреты генерирует `install.ps1` на самом сервере: `C:\NeoShard\config\secrets.json`,
читать могут только администраторы и SYSTEM.

## Сайт и загрузки

- Сайт (`apps/site`) — статика без внешних скриптов и шрифтов. Ссылки на пожертвования
  хранятся в `C:\NeoShard\site\site.json` (`donate: [{title, url, note}]`, только https).
  При переустановке этот файл на сервере не перезаписывается.
- Новые сборки выкладываются так:
  ```bash
  deploy/windows/publish-downloads.sh 0.1.3 NeoShard-0.1.3-setup.exe NeoShard-0.1.3-arm64.apk
  ```
  Скрипт загружает файлы и пишет `latest.json` (версия, размер, SHA-256), а кнопки на
  сайте берут ссылки оттуда.

## Установка и обновление

1. На машине сборки:
   ```bash
   LIVEKIT_EXE=… CADDY_EXE=… deploy/windows/pack.sh chat.example.ru pkg.zip
   ```
   Адрес сервера вшивается в веб-клиент. Клиенты Tauri собираются с тем же
   `VITE_NEOSHARD_INSTANCE=https://chat.example.ru`.
2. Скопируйте `pkg.zip` и `install.ps1` на сервер (`scp`), затем запустите от администратора:
   ```bash
   powershell -ExecutionPolicy Bypass -File install.ps1 -Package pkg.zip -Domain chat.example.ru -PublicIp 203.0.113.7
   ```
   Повторный запуск обновляет файлы; база и секреты остаются.

## Что делает владелец машины (один раз)

- `host-setup.ps1` от администратора: правила брандмауэра Windows и отключение сна от сети.
- `openwrt-ports.sh` на роутере: постоянный адрес сервера в локальной сети и проброс портов:
  ```bash
  ssh root@192.168.1.1 "HOST_IP=192.168.1.50 HOST_MAC=AA:BB:CC:DD:EE:FF sh -s" < openwrt-ports.sh
  ```

## Ограничения

- `node_ip` LiveKit и домен `*.sslip.io` привязаны к внешнему IP. Если провайдер его сменит,
  перезапустите `install.ps1` с новым `-PublicIp`. Если IP меняется часто, нужен
  DDNS-домен, и тогда клиенты придётся пересобрать.
- TURN (coturn) здесь не ставится. Из сетей, где закрыт UDP, голос идёт через TCP 7881.
