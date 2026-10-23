# Recon map — Discord → NeoShard

> Clean-room. Источники — только публичные: документация для разработчиков Discord
> (discord.com/developers/docs), help center (support.discord.com), публичные
> маркетинговые страницы и общеизвестное поведение клиента. JS-бандлы, приватные
> эндпоинты, сетевые дампы, ассеты **не читались и не сохранялись**.
>
> REA (MCP `rea`) в этой сессии не подключился (CONNECT_TIMEOUT). Поскольку правило
> recon запрещает декомпиляцию и логирование сетевых вызовов для копирования
> эндпоинтов, поведенческие спеки в `docs/research/` составлены по публичной
> документации Gateway/REST — этого достаточно, протокол NeoShard свой.

## 1. Scope

| Вопрос | Ответ |
|---|---|
| Приложение / платформа | Discord, веб-клиент + десктоп; мобильные — как PWA |
| Срез | Полный: аккаунты, гильдии, каналы, роли/права, сообщения, ЛС, presence, голос/видео/стрим, боты/вебхуки, + федерация (у оригинала нет) |
| Для кого | Открытый проект, официальный инстанс в РФ, self-hosting, форки |

Core loop: **открыть клиент → увидеть гильдии и непрочитанное → зайти в канал → прочитать/написать → (позвать в голос)**.

## 2. Sources

| source | URL | что дало |
|---|---|---|
| Dev docs: Gateway | https://discord.com/developers/docs/events/gateway | опкоды, heartbeat, identify/resume, intents, сжатие |
| Dev docs: Permissions | https://discord.com/developers/docs/topics/permissions | битовые права, алгоритм overwrites |
| Dev docs: Rate limits | https://discord.com/developers/docs/topics/rate-limits | бакеты, заголовки, глобальный лимит |
| Dev docs: Resources (Channel, Guild, Message, User) | https://discord.com/developers/docs/resources/channel | модель данных |
| Dev docs: Interactions / Slash commands | https://discord.com/developers/docs/interactions/application-commands | slash-команды |
| Dev docs: Webhooks | https://discord.com/developers/docs/resources/webhook | вебхуки |
| Dev docs: Snowflakes | https://discord.com/developers/docs/reference#snowflakes | формат ID |
| Help center | https://support.discord.com | 2FA, аудит, автомод, форумы, треды |
| Marketing | https://discord.com | позиционирование |

## 3. Screen inventory

| ID | screen | how | purpose | key components | states |
|---|---|---|---|---|---|
| S01 | Login | `/login` | вход | form, input, button, link | empty, error, 2FA-required, loading |
| S02 | Register | `/register` | регистрация | form, date input | error (занято), loading |
| S03 | 2FA prompt | после S01 | TOTP/backup | code input | error |
| S04 | Recover | `/recover` | сброс пароля | form | sent |
| S05 | App shell | `/channels/@me` | каркас | guild rail, sidebar, main, member list | loading (splash), offline banner |
| S06 | DM list / Friends | `/channels/@me` | ЛС | list, tabs, search | empty, filled |
| S07 | Text channel | `/channels/:g/:c` | чат | header, message list (virtual), composer, typing | empty, loading history, filled, no-permission (read-only composer), jump-to-present |
| S08 | Thread panel | правая панель | треды | message list | — |
| S09 | Forum channel | канал типа forum | посты | post cards, tags | empty |
| S10 | Voice channel | клик по голосовому | голос/видео | participant grid, controls | connecting, connected, reconnecting, permission denied |
| S11 | Screen share picker | из S10 | стрим | picker modal | — |
| S12 | Guild settings | меню гильдии | настройки | tabs: overview, roles, emoji, stickers, audit log, bans, automod, invites, webhooks | — |
| S13 | Role editor | S12 | роли | role list (reorder), permission toggles | — |
| S14 | Channel settings | шестерёнка канала | overwrites | overwrite editor (allow/neutral/deny) | — |
| S15 | User settings | шестерёнка | аккаунт | tabs: account, security(2FA), sessions, appearance(theme), notifications, voice, plugins, server endpoints | — |
| S16 | Invite modal | «пригласить» | приглашения | link, expire select | — |
| S17 | Create/join guild | `+` в rail | онбординг | modal steps | — |
| S18 | Profile popout | клик по аватару | профиль | card, roles | — |
| S19 | Search | поле поиска | поиск | filters `from:`, `in:`, `has:` | empty, results |
| S20 | Context menus | ПКМ | действия | menu | — |
| S21 | Pins popover | шапка канала | закрепы | list | empty |
| S22 | Emoji/sticker picker | composer | реакции | grid (virtual), search | — |
| S23 | Notifications inbox | шапка | упоминания | list | empty |
| S24 | Mobile layout | <768px | PWA | swipe drawers | — |

## 4. User flows

```
F01 Регистрация и первый вход            S02 → S05 → S17 (5 кликов)
    edge: email занят, слабый пароль, rate limit
F02 Вход с 2FA                           S01 → S03 → S05 (3)
F03 Создать гильдию и пригласить друга   S05 → S17 → S16 (4)
F04 Принять приглашение                  /invite/:code → S05 (1–2)
F05 Отправить сообщение                  S07 (1: Enter); optimistic, retry при сбое сети
    edge: нет права SEND_MESSAGES, slowmode, оффлайн → очередь
F06 Ответить / реакция / редактировать / удалить   S07 + S20
F07 Создать тред из сообщения            S07 → S08
F08 Настроить роль и overwrite           S12 → S13 → S14
F09 Модерация: тайм-аут/кик/бан + аудит  S18/S20 → S12(audit)
F10 Голос: зайти, мьют, видео, стрим     S10 → S11
F11 ЛС и групповые ЛС (E2EE)             S06 → S07
F12 Подключить бота / вебхук / slash     S12(webhooks) ; API
F13 Сменить сервер (self-hosted инстанс) S01 → «сервер: …» → S01
F14 Сменить тему / подключить плагин     S15
F15 Resume после обрыва сети             S05 offline banner → auto-resume без перезагрузки
```

## 5. Components

Button (primary/secondary/danger/ghost/icon; disabled/loading), Input/Textarea,
Toggle, Tri-state toggle (overwrites), Select, Modal, Toast, Tooltip, Context menu,
Popover, Tabs, Avatar (+status dot), Badge (unread/mention), GuildIcon (+pill
indicator), ChannelRow (text/voice/announcement/forum; unread, muted), MemberRow,
MessageRow (grouped/ungrouped, reply preview, attachments, embeds, reactions),
Composer, TypingIndicator, VirtualList, EmojiPicker, VoiceTile, Splash.

## 6. Inferred data model (public API docs → high confidence)

```
User       id(snowflake), username, display_name, avatar, flags, mfa_enabled     high
Session    id, user_id, device, ip, created_at, last_seen                         high
Guild      id, name, icon, owner_id, features                                     high
Role       id, guild_id, name, color, position, permissions(u64), hoist, mentionable high
Member     guild_id, user_id, nick, roles[], joined_at, communication_disabled_until high
Channel    id, guild_id?, type, name, topic, position, parent_id, nsfw, rate_limit_per_user,
           permission_overwrites[{id,type(role|member),allow,deny}]               high
Message    id, channel_id, author_id, content, edited_at, reference(reply), attachments[],
           embeds[], mentions[], pinned, flags                                    high
Reaction   message_id, emoji, user_id                                             high
Invite     code, guild_id, channel_id, inviter_id, max_uses, uses, expires_at     high
AuditLog   id, guild_id, user_id, action, target_id, changes(json), reason        high
Webhook    id, channel_id, token, name                                            high
ReadState  user_id, channel_id, last_read_message_id, mention_count               medium (guess)
```

## 7. Feature matrix

См. `features.csv`. `clone` заполняется по ходу; `replica-diff` оценивает.

## 8. Что нельзя/не нужно клонировать

| что | почему |
|---|---|
| Nitro / монетизация | платная модель оригинала, не наша цель |
| Сеть пользователей, Discovery-каталог | принадлежит оригиналу |
| Бренд, звуки, иконки, шрифт gg sans, Wumpus | проприетарные ассеты |
| Activities (игры в голосе) | партнёрские интеграции |
| Rich presence игр | интеграции/SDK оригинала |

## 9. Size

24 экрана, 15 потоков, 13 сущностей. Сложное: (1) realtime Gateway с resume на
100k соединений, (2) голос/видео/стрим через SFU + TURN в ограниченных сетях,
(3) MLS E2EE и федерация. Размер: **XL** — честно больше, чем один запуск агента;
в этом запуске строится максимально возможный работающий объём, остальное — в
`docs/PROGRESS.md`.
