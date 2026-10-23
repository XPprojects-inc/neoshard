# NeoShard permissions v1

*License: CC-BY-4.0. Reference implementation: `crates/neoshard-proto/src/permissions.rs`.*

Permissions are a 64-bit mask (decimal **string** in JSON). Bits are append-only;
unknown bits must be preserved.

| bit | name | bit | name |
|---|---|---|---|
| 0 | VIEW_CHANNEL | 20 | MUTE_MEMBERS |
| 1 | SEND_MESSAGES | 21 | DEAFEN_MEMBERS |
| 2 | READ_MESSAGE_HISTORY | 22 | MOVE_MEMBERS |
| 3 | ADD_REACTIONS | 23 | PRIORITY_SPEAKER |
| 4 | ATTACH_FILES | 32 | CREATE_INVITE |
| 5 | EMBED_LINKS | 33 | CHANGE_NICKNAME |
| 6 | MENTION_EVERYONE | 34 | MANAGE_NICKNAMES |
| 7 | MANAGE_MESSAGES | 35 | KICK_MEMBERS |
| 8 | CREATE_THREADS | 36 | BAN_MEMBERS |
| 9 | SEND_IN_THREADS | 37 | MODERATE_MEMBERS (timeouts) |
| 10 | USE_EXTERNAL_EMOJI | 40 | MANAGE_CHANNELS |
| 11 | USE_COMMANDS | 41 | MANAGE_ROLES |
| 12 | MANAGE_THREADS | 42 | MANAGE_GUILD |
| 16 | CONNECT | 43 | MANAGE_WEBHOOKS |
| 17 | SPEAK | 44 | MANAGE_EMOJIS |
| 18 | VIDEO | 45 | VIEW_AUDIT_LOG |
| 19 | STREAM (screen share) | 63 | ADMINISTRATOR |

## Algorithm

```
if member is owner: ALL
base = @everyone.permissions | OR(role.permissions for member roles)
if base has ADMINISTRATOR: ALL
p = base
p = (p & ~ow[@everyone].deny) | ow[@everyone].allow
p = (p & ~OR(ow[r].deny for member roles)) | OR(ow[r].allow for member roles)
p = (p & ~ow[member].deny) | ow[member].allow
if p lacks VIEW_CHANNEL: 0
if p lacks SEND_MESSAGES: remove MENTION_EVERYONE, ATTACH_FILES, EMBED_LINKS
if member is timed out: p &= VIEW_CHANNEL | READ_MESSAGE_HISTORY
```

Threads use their parent channel's overwrites. DMs grant a fixed set
(view, send, history, reactions, attachments, embeds, voice/video/stream).

## Hierarchy

- A member may act on (kick, ban, timeout, rename, change roles of) another member
  only if their highest role position is **strictly greater**; the owner can act on
  everyone and nobody can act on the owner.
- Roles at or above one's own highest role cannot be edited, assigned, moved, or
  targeted by channel overwrites.
- Non-administrators can only grant (in roles or overwrites) bits they hold.
