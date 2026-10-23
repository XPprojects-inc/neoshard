# NeoShard для Android

Это тот же клиент, что и десктопный: интерфейс из `apps/web` открывается в системном WebView,
а `neoshard-core` работает нативно, без WASM. Код приложения общий — `src-tauri/src/lib.rs`
подключает `apps/desktop/src-tauri/src/app.rs`. На узких экранах интерфейс переключается на
Material-раскладку: нижняя навигация, FAB, нижние листы.

Отдельный проект нужен по технической причине. Android загружает приложение как `cdylib`.
Если собрать `cdylib` с openmls под windows-gnu, переполняется таблица экспортов PE, поэтому
в десктопной сборке такой библиотеки нет.

## Сборка

Нужны Android SDK, NDK, JDK 17+ и Rust-таргет `aarch64-linux-android`:

```bash
source scripts/env.sh
source scripts/android-env.sh
cd apps/android
# gen/android лежит в git (там MainActivity с отступами под системные панели)
npm run build   # APK для arm64
```

Gradle выдаёт неподписанный APK: `gen/android/app/build/outputs/apk/universal/release/`.
Подпишите его своим ключом:

```bash
apksigner sign --ks my.jks --out NeoShard.apk app-universal-release-unsigned.apk
```

## Ограничения

- Адрес сервера вводится на экране входа и должен быть доступен с телефона. `127.0.0.1` на
  телефоне — это сам телефон.
- Картинки и файлы WebView загружает только по `https:` (так задаёт CSP). Для локального
  HTTP-сервера используйте HTTPS-прокси.
- Голос: приложение не запрашивает разрешение на микрофон (`RECORD_AUDIO`), поэтому в голосовых
  каналах работает только прослушивание.
- Push-уведомлений нет (см. `docs/PROGRESS.md`).
