# NeoShard — дизайн-система

Токены: `docs/replica/design/tokens.dark.json` (по умолчанию) и `tokens.light.json`.
Проверка контраста (`contrast.py`): **обе темы — 21 пара, 0 нарушений WCAG AA**.
В коде: `packages/ui/src/theme/tokens.css` (CSS-переменные `--ns-*`), темы-JSON
для форков — `packages/ui/themes/*.json`.

Раскладка привычна пользователям чатов этого класса (rail → сайдбар → лента →
участники), но визуально NeoShard самостоятелен: бирюзовый акцент вместо
фиолетового, иконки гильдий в форме «осколка» (скошенный угол), шрифт —
системный стек с Inter (OFL), иконки — Lucide (ISC). Никаких чужих ассетов.

## Раскладка

| зона | ширина | примечание |
|---|---|---|
| Guild rail | 72 px | вертикальный список «осколков», drag-reorder |
| Sidebar | 240 px | категории, каналы; снизу панель пользователя 52 px |
| Main | fluid, min 360 | шапка 48 px, лента (virtual), composer |
| Members | 240 px | скрыт < 1100 px |
| Mobile < 768 | 1 колонка | rail+sidebar — выезжающий drawer (`drag()`), участники — правый drawer |

Брейкпоинты: 768, 1100, 1440. Отступы — шкала 4 px.

## Компоненты

```
Button      variants primary | secondary | ghost | danger;  sizes sm 32 / md 40
            states   hover, active (scale .97 press), focus-visible (2px ring --ns-focus), disabled, loading (aria-busy, label сохраняется)
            a11y     настоящий <button>; иконочные — aria-label
Input       states   focus (ring), invalid (aria-invalid + текст ошибки aria-describedby), disabled
GuildIcon   56×56 «осколок» (radius 14, правый верхний угол срезан clip-path); hover → radius 18, 120 ms
            pill-индикатор слева: unread 8px, selected 40px, hover 20px — spring snappy
            a11y     role=treeitem в role=tree «Серверы», aria-current на выбранном, badge упоминаний озвучивается
ChannelRow  иконки: # текст, 🔊 голос, 📣 анонсы, 💬 форум; unread — жирный текст + точка; muted — faint
            a11y     ссылка (<a href>) — работает Ctrl/Cmd+click, клавиатура ↑/↓ в списке
MessageRow  группировка подряд идущих сообщений автора (< 7 мин); hover — панель действий (реакция, ответ, правка, удалить)
            pending — opacity .6; failed — красная иконка + «Повторить»
            a11y     role=article, aria-setsize/posinset в виртуальном списке; время в <time datetime>
Composer    textarea autosize (1–12 строк), Enter — отправить, Shift+Enter — перенос, ↑ — правка последнего
            disabled при отсутствии SEND_MESSAGES: плейсхолдер «Нет прав на отправку сообщений»
Typing      «Алиса печатает…» / «Несколько человек печатают…»; три точки — CSS keyframes, отключаются при reduced-motion
Modal       Presence: scale .96→1 + fade 200 ms, выход 150 ms; focus trap; Esc закрывает; aria-modal
Toast       снизу справа; spring gentle; авто-скрытие 4 с (пауза при hover/focus); role=status
ContextMenu ПКМ / Shift+F10; стрелки, Enter, Esc; role=menu
Avatar      круг; статус-точка 10 px (online #3ddc84, idle жёлтый, dnd красный) + текстовая метка для SR
VoiceTile   сетка участников; говорящий — кольцо accent; видео — <video> плитка
Splash      логотип + «Подключаемся…»; состояние соединения в шапке (баннер «Нет сети — переподключение через N с»)
```

## Состояния экранов (из recon S01–S24)

Пустые состояния — свои тексты: «Здесь пока тихо. Напишите первое сообщение», «Нет закрепов»,
«Ничего не найдено». Ошибка загрузки истории — «Не удалось загрузить сообщения · Повторить».
Нет прав — «У вас нет доступа к этому каналу».

## Движение

Токены `packages/ui/src/motion/tokens.ts`; правила — `docs/research/ui-timings.md`.
Только `transform`/`opacity`; `prefers-reduced-motion` → мгновенные переходы.
