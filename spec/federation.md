# NeoShard federation v1 (draft, implemented subset)

*License: CC-BY-4.0. Implementation: `server/federation`, `server/api/src/routes/federation.rs`.*

## Identity

- An instance is identified by its domain (`host[:port]`, no other characters).
- Its Ed25519 public key is published in `/.well-known/neoshard` →
  `federation.public_key` (base64, 32 bytes). Receivers pin it on first contact
  (TOFU); operators can block or replace keys.
- A federated user address is `username@domain`. Locally, remote users appear as
  accounts with `instance = domain`.

## Signed requests

Every server-to-server request carries:

| header | value |
|---|---|
| `NeoShard-Origin` | sender domain |
| `NeoShard-Destination` | receiver domain |
| `NeoShard-Date` | unix seconds; rejected if |skew| > 300 s |
| `NeoShard-Nonce` | random, single use per origin |
| `NeoShard-Signature` | base64 Ed25519 over the canonical string |

Canonical string (UTF-8, `\n`-separated):

```
neoshard-federation-v1
<METHOD>
<path>
<origin>
<destination>
<date>
<nonce>
<base64(SHA-256(body))>
```

## Endpoints

- `GET /federation/v1/users/{username}` — public profile of a local user.
- `POST /federation/v1/inbox` — signed events. Implemented:
  `{"type":"dm.message","id":"…","from":"alice","to":"bob","content":"…"}` —
  delivers a DM from `alice@origin` to local `bob`; idempotent by `(origin, id)`.

## Not yet specified (roadmap)

Federated guild membership (remote members of a home-instance guild), federated
MLS key directory, account migration. See `docs/PROGRESS.md`.
