# NeoShard Gateway protocol v1

*License: CC-BY-4.0. Normative schema: [`proto/neoshard/v1/gateway.proto`](proto/neoshard/v1/gateway.proto).*

## Transport

- Discover the URL: `GET {instance}/.well-known/neoshard` → `gateway`.
- Connect: `{gateway}?v=1[&compress=zstd]` (WebSocket, TLS in production; port 443).
- Every WebSocket message is **binary** and carries exactly one `GatewayFrame`.
- **Compression** is per frame and self-describing: a frame starting with the zstd
  magic `28 B5 2F FD` is an independent zstd frame; otherwise it is raw protobuf.
  (A `GatewayFrame` never starts with byte `0x28`.) With `compress=zstd` the server
  compresses its frames; clients may send either form. Decompressed size ≤ 8 MiB.
- Client frames: ≤ 64 KiB, ≤ 120 per minute (heartbeats included).

## Lifecycle

```
C ── connect ──────────────────────────────▶ S
C ◀──────────────────── Hello{heartbeat_interval_ms}
C ── Identify{token, intents} ─────────────▶            (within 10 s)
C ◀──────────────────── Ready{session_id, resume_url, user, guilds, …}
C ◀──────────────────── DispatchBatch{[Dispatch{seq, Event}]} …
C ── Heartbeat{last_seq} every interval ───▶ (first after interval × random[0,1))
C ◀──────────────────── HeartbeatAck
```

- `seq` starts at 1 and increases by 1 per dispatched event, per session.
- **Ephemeral events** (`typing_start`, `presence_update`, `member_list_chunk`) carry
  `seq = 0`: they are not replayed on Resume and must not update the client's
  `last_seq`.
- If no `HeartbeatAck` arrives before the next heartbeat is due, the connection is a
  zombie: close it (any code ≠ 1000) and Resume.
- If the server sees no frame for `1.5 × interval + 5 s`, it closes the connection.

## Resume

Reconnect to `resume_url` and send `Resume{token, session_id, last_seq}`:

- success: missed dispatches (seq > last_seq) arrive first, then `Resumed{replayed}`;
- failure: `InvalidSession{resumable:false}` → wait 1–5 s (random), then Identify.

Sessions survive a disconnect for **120 s** with a replay buffer of up to 1024 events /
128 KiB (encoded). A session that ever lost events (slow consumer) is never resumable.
`Reconnect{}` means "reconnect and Resume" (node restarting).

## Errors

`GatewayError{code, message, reconnect}` is sent right before the server closes
the socket with WebSocket close code `4000 + code`:

| code | name | client action |
|---|---|---|
| 0 | UNKNOWN | reconnect + Resume |
| 1 | DECODE | reconnect + Resume |
| 2 | NOT_AUTHENTICATED | Identify first |
| 3 | AUTH_FAILED | stop; the token is invalid or the session was revoked |
| 4 | ALREADY_AUTHENTICATED | bug in client |
| 5 | RATE_LIMITED | back off, reconnect |
| 7 | INVALID_VERSION | stop |

## Intents

`Identify.intents` is an OR of `Intent` bits; `0` = everything the user may see
(normal clients). Bots should subscribe narrowly. `MESSAGE_CONTENT`: without it,
bots receive messages with empty `content` unless the message mentions them, is a
DM, or is their own. Events with intent `NONE` (read states, interactions,
session revocation) are always delivered.

## Event visibility (server guarantees)

- Guild events reach only members; channel events only members who have
  `VIEW_CHANNEL` (plus the event's extra bits) in that channel.
- When an overwrite/role/member change makes a channel appear or disappear for a
  user, that user receives `channel_create` / `channel_delete` for it.
- Moderation events (`ban_add/remove`) reach only members with `BAN_MEMBERS`.

## Large guilds

Guilds with more than 250 members do not broadcast `presence_update` to every
member (that would be quadratic). Clients get presence for the visible part of
the member list through `member_list` → `member_list_chunk`.

## Client requests

`ClientRequest` frames: `presence_set`, `typing` (≤ 1 per channel per 8 s),
`voice_state` (join/leave/mute; server keeps server-mute), `member_list`
(lazy window `[start, end)`, ≤ 200, answered with `member_list_chunk`), `ack`
(read state).

## Batching

The server coalesces events that are already queued into one `DispatchBatch`
(no artificial delay). Clients apply events in order and must treat unknown
event kinds as no-ops (forward compatibility).
