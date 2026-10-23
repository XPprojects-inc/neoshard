# Поведенческая спека: модель прав

Источник: публичная документация Discord «Permissions». Значения битов у
NeoShard **свои** (см. `crates/neoshard-proto/src/permissions.rs`), алгоритм —
общепринятый для такого класса систем.

## Базовые принципы

- Права — 64-битная маска. Каждая роль несёт маску; у гильдии есть роль
  `@everyone` с `id == guild_id`.
- Владелец гильдии имеет все права.
- `ADMINISTRATOR` в итоговой маске гильдии ⇒ все права, overwrites игнорируются.

## Алгоритм вычисления прав участника в канале

```
base = everyone.permissions
for role in member.roles: base |= role.permissions
if owner or base & ADMINISTRATOR: return ALL

perms = base
# 1. overwrite для @everyone
perms &= ~ow_everyone.deny ; perms |= ow_everyone.allow
# 2. overwrite всех ролей участника — сначала суммируются, потом применяются
allow = OR(ow_role.allow for roles), deny = OR(ow_role.deny for roles)
perms &= ~deny ; perms |= allow
# 3. overwrite конкретного участника
perms &= ~ow_member.deny ; perms |= ow_member.allow
```

## Неявные правила

- Нет `VIEW_CHANNEL` ⇒ остальные права канала = 0.
- Нет `SEND_MESSAGES` ⇒ нет `MENTION_EVERYONE`, `ATTACH_FILES`, `EMBED_LINKS`
  в этом канале.
- Тайм-аут (communication_disabled_until > now) ⇒ остаются только `VIEW_CHANNEL`
  и `READ_MESSAGE_HISTORY`.
- Иерархия ролей: управлять (кик/бан/редактирование) можно только участниками и
  ролями строго ниже своей высшей роли; владелец — исключение.
- Выдать роли права, которых нет у себя, нельзя (кроме админа/владельца).

## Решение NeoShard

Алгоритм живёт **в одном месте** — `neoshard-proto::permissions` — и
используется сервером (авторизация) и `neoshard-core` (UI: что показывать).
Покрыт таблицей тестов, включая все неявные правила.
