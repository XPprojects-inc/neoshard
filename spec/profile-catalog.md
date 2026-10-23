# Каталог косметики профиля

SPDX-License-Identifier: CC-BY-4.0

Идентификаторы, которые принимает `PATCH /api/v1/users/@me` и которые клиенты
рисуют сами. Источник истины — `crates/neoshard-proto/src/profile.rs`. Сервер
отклоняет неизвестные id (400), клиент неизвестные id молча игнорирует. Поэтому
форк может добавлять свои варианты, не ломая чужие клиенты.

Графика NeoShard — собственная: SVG и CSS в `apps/web/src/profile/cosmetics.tsx`.
Всё бесплатно, платных уровней нет.

| поле | id |
|---|---|
| `avatar_decoration` — рамка аватара | `orbit` `petals` `frost` `flame` `circuit` `stardust` `leaves` `crown` `neon` `shards` `cat_ears` `headphones` `halo` `hearts` |
| `profile_effect` — анимация карточки | `snowfall` `sparkles` `aurora` `bubbles` `embers` `rain` `confetti` `starfield` |
| `nameplate` — фон имени в списках | `dusk` `lagoon` `ember` `forest` `nebula` `paper` `grid` `waves` |
| `name_style.font` | `sans` `serif` `mono` `rounded` `slab` `hand` (системные шрифты, ничего не загружается) |
| `name_style.effect` | `solid` `gradient` `neon` `toon` `pop` `shimmer` |
| `tag_badge` (сервер) | `shard` `star` `leaf` `flame` `wave` `bolt` `heart` `moon` |

## Ограничения

| поле | лимит |
|---|---|
| `display_name` | 32 символа |
| `pronouns` | 40 |
| `bio` | 190, Markdown |
| `theme_colors`, `name_style.colors` | 0xRRGGBB; тема — ровно 2 цвета или пусто, имя — до 2 |
| `custom_status` / эмодзи | 128 / 64 символа, срок — unix ms, 0 = бессрочно |
| тег сервера | 1–4 буквы или цифры любой письменности |
| ссылки профиля | до 6; галочка — если страница ссылается на `{public_url}/u/{username}` с `rel="me"` |
| заметка | 256 символов, видна только автору |

## Анимации

Эффекты, переливы и вращение рамок отключаются при `prefers-reduced-motion: reduce`.
