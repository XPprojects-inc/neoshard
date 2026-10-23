# Compatibility policy

*Applies to the wire protocol (`spec/`), REST API `/api/v1`, Gateway `v=1`, and the
client core API (`core.proto`, `neoshard-core`). License: CC-BY-4.0.*

1. **Additive only within a major version.** New messages, fields, enum values,
   oneof variants, endpoints and optional request fields may be added at any time.
2. **Never reused.** Protobuf field numbers and enum values are never reused or
   repurposed; removed fields are `reserved`.
3. **Clients must ignore what they do not know**: unknown fields, unknown enum
   values (treat as the zero value), unknown event kinds (no-op), unknown JSON keys.
4. **Deprecation** of anything requires an announcement in `CHANGELOG.md`, a
   `deprecated` marker in the spec, and at least **6 months** before removal in the
   next major version (`/api/v2`, `v=2`).
5. **Servers support the previous major version** for at least 12 months after a
   new one ships.
6. **JSON encoding** is the canonical proto3 JSON mapping with original field
   names (snake_case): 64-bit integers are strings, enums are names, default
   values may be omitted.
7. **`neoshard-core`** follows SemVer: breaking changes to `Command`/`Update` or the
   Rust API bump the major version (0.x: minor).
