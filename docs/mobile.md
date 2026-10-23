# Мобильные клиенты

**Этап 1 (сейчас): PWA** — `apps/web` с манифестом и service worker; работает на
Android/iOS из браузера, ставится на домашний экран.

**Этап 2: нативные клиенты поверх ядра** (ADR-0016). Ядро собирается с `--features ffi`
и даёт через uniffi объект `NeoShardClient { new(clientName, dbPath), dispatch(bytes), poll(timeoutMs) -> bytes }`.

```bash
cargo rustc -p neoshard-core --lib --crate-type cdylib --features ffi,audio,e2ee --release --target aarch64-linux-android
cargo run --bin uniffi-bindgen generate --library target/aarch64-linux-android/release/libneoshard_core.so --language kotlin --out-dir android/
# iOS: --target aarch64-apple-ios, --language swift
```

Kotlin: корутина в цикле `client.poll(1000)` → `UpdateBatch.parseFrom(bytes)` → StateFlow;
команды — `client.dispatch(Command.newBuilder()…build().toByteArray())` (классы
генерируются из `spec/proto` через protoc/Wire). Swift — аналогично через SwiftProtobuf.
UI — Compose / SwiftUI; вся логика (reconnect, кэш SQLite, права, E2EE, RNNoise) — в ядре.
