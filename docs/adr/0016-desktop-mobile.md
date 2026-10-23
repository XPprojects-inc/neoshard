# ADR-0016: Десктоп — Tauri 2; мобильные — PWA, затем нативные поверх uniffi
- Статус: принято · 2026-10-18
## Решение
Tauri 2: фронтенд — тот же `apps/web`, ядро — нативно (Tauri commands), без WASM.
Мобильные: этап 1 — PWA; этап 2 — Kotlin (Compose) и Swift (SwiftUI) поверх uniffi-биндингов ядра.
Tauri mobile отклонён — WebView на слабых Android медленнее нативного списка.
