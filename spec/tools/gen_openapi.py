"""Generate spec/openapi.yaml from the route table below (kept in sync with
server/api/src/routes/*.rs; CI checks that every router path appears here).
Usage: python spec/tools/gen_openapi.py > spec/openapi.yaml"""

ROUTES = [
    # method, path, summary, auth, permission / note
    ("get", "/.well-known/neoshard", "Instance discovery (gateway, api, media, federation key)", False, ""),
    ("post", "/api/v1/auth/register", "Create an account → {token, user}", False, "rate-limited per IP"),
    ("post", "/api/v1/auth/login", "Log in → {mfa:false, token, user} or {mfa:true, ticket}", False, "failures limited per account"),
    ("post", "/api/v1/auth/mfa/totp", "Second factor {ticket, code} → {token, user}", False, "TOTP or backup code; each code once"),
    ("post", "/api/v1/auth/mfa/totp/setup", "Start TOTP setup → {secret, otpauth_url}", True, ""),
    ("post", "/api/v1/auth/mfa/totp/enable", "Enable TOTP {code, password} → {backup_codes}", True, "revokes other sessions"),
    ("post", "/api/v1/auth/mfa/totp/disable", "Disable TOTP {code, password}", True, ""),
    ("post", "/api/v1/auth/logout", "Revoke the current session", True, ""),
    ("post", "/api/v1/auth/password", "Change password {password, new_password}", True, "revokes other sessions"),
    ("post", "/api/v1/auth/recover", "Send a recovery email {email} (always 202)", False, ""),
    ("post", "/api/v1/auth/recover/confirm", "Reset password {token, password}", False, ""),
    ("get", "/api/v1/sessions", "List sessions", True, ""),
    ("delete", "/api/v1/sessions/{id}", "Revoke a session (closes its Gateway connections)", True, ""),
    ("post", "/api/v1/users/@me/delete", "Delete account and personal data {password}", True, "152-FZ / GDPR"),
    ("get", "/api/v1/users/@me", "Current user", True, ""),
    ("patch", "/api/v1/users/@me", "Update profile {display_name, pronouns, bio, avatar, banner, banner_color (-1 clears), theme_colors, avatar_decoration, profile_effect, nameplate, name_style{font,effect,colors}, primary_guild_id}", True, "catalog ids: spec/profile-catalog.md"),
    ("get", "/api/v1/users/{uid}/profile", "Full profile ?guild_id → UserProfile (bio, banner, theme, server profile, mutual guilds, viewer's note, links)", True, "self, shared guild or DM; else 404"),
    ("put", "/api/v1/users/{uid}/note", "Private note {note} (empty deletes)", True, "self, shared guild or DM"),
    ("put", "/api/v1/users/@me/links", "Replace profile links [{label, url}] (≤ 6)", True, ""),
    ("post", "/api/v1/users/@me/links/verify", "Verify links by a rel=me backlink to {public_url}/u/{username}", True, "SSRF-safe fetch"),
    ("get", "/api/v1/users/{uid}", "Public profile", True, ""),
    ("get", "/api/v1/users/@me/channels", "DMs and group DMs", True, ""),
    ("post", "/api/v1/users/@me/channels", "Open DM {recipient_id} or group DM {recipient_ids, name, e2ee}", True, "friends or a shared guild (local users); never with a block"),
    ("get", "/api/v1/users/@me/read-states", "Read states", True, ""),
    ("get", "/api/v1/users/@me/relationships", "Friends, blocks and pending requests", True, ""),
    ("post", "/api/v1/users/@me/relationships", "Send a friend request {username} (accepts an incoming one)", True, "30/hour; blocks are not revealed"),
    ("put", "/api/v1/users/@me/relationships/{uid}", "Accept a request {type: 1} or block {type: 2}", True, ""),
    ("delete", "/api/v1/users/@me/relationships/{uid}", "Remove friend, cancel/decline request, unblock", True, ""),
    ("get", "/api/v1/users/@me/guilds", "Guilds of the user", True, ""),
    ("delete", "/api/v1/users/@me/guilds/{gid}", "Leave a guild", True, ""),
    ("put", "/api/v1/channels/{cid}/recipients/{uid}", "Add to group DM", True, "recipient"),
    ("delete", "/api/v1/channels/{cid}/recipients/{uid}", "Remove from group DM", True, "owner or self"),
    ("post", "/api/v1/guilds", "Create a guild {name}", True, ""),
    ("get", "/api/v1/guilds/{gid}", "Guild with visible channels, roles, emojis", True, "member"),
    ("patch", "/api/v1/guilds/{gid}", "Update {name, icon, tag, tag_badge}", True, "MANAGE_GUILD"),
    ("delete", "/api/v1/guilds/{gid}", "Delete guild", True, "owner"),
    ("post", "/api/v1/guilds/{gid}/transfer", "Transfer ownership {user_id}", True, "owner"),
    ("get", "/api/v1/guilds/{gid}/channels", "Visible channels", True, "member"),
    ("post", "/api/v1/guilds/{gid}/channels", "Create channel {name, type, parent_id, topic, overwrites}", True, "MANAGE_CHANNELS"),
    ("patch", "/api/v1/guilds/{gid}/channels", "Reorder [{id, position, parent_id}]", True, "MANAGE_CHANNELS"),
    ("get", "/api/v1/guilds/{gid}/roles", "Roles", True, "member"),
    ("post", "/api/v1/guilds/{gid}/roles", "Create role {name, permissions, color, hoist, mentionable}", True, "MANAGE_ROLES; only held bits"),
    ("patch", "/api/v1/guilds/{gid}/roles", "Reorder [{id, position}]", True, "MANAGE_ROLES; hierarchy"),
    ("patch", "/api/v1/guilds/{gid}/roles/{rid}", "Update role", True, "MANAGE_ROLES; hierarchy"),
    ("delete", "/api/v1/guilds/{gid}/roles/{rid}", "Delete role", True, "MANAGE_ROLES; hierarchy"),
    ("get", "/api/v1/guilds/{gid}/members", "Members page ?offset&limit", True, "member"),
    ("get", "/api/v1/guilds/{gid}/members/{uid}", "Member (uid or @me)", True, "member"),
    ("patch", "/api/v1/guilds/{gid}/members/{uid}", "Update {nick, timeout_until, role_ids, reason}; server profile of @me {avatar, banner, bio, pronouns, theme_colors}", True, "CHANGE/MANAGE_NICKNAMES, MODERATE_MEMBERS, MANAGE_ROLES; profile: self only"),
    ("delete", "/api/v1/guilds/{gid}/members/{uid}", "Kick ?reason", True, "KICK_MEMBERS; hierarchy"),
    ("put", "/api/v1/guilds/{gid}/members/{uid}/roles/{rid}", "Add role", True, "MANAGE_ROLES; hierarchy"),
    ("delete", "/api/v1/guilds/{gid}/members/{uid}/roles/{rid}", "Remove role", True, "MANAGE_ROLES; hierarchy"),
    ("get", "/api/v1/guilds/{gid}/bans", "Bans", True, "BAN_MEMBERS"),
    ("put", "/api/v1/guilds/{gid}/bans/{uid}", "Ban {reason, delete_message_seconds}", True, "BAN_MEMBERS; hierarchy"),
    ("delete", "/api/v1/guilds/{gid}/bans/{uid}", "Unban", True, "BAN_MEMBERS"),
    ("get", "/api/v1/guilds/{gid}/audit-log", "Audit log ?before&limit&action", True, "VIEW_AUDIT_LOG"),
    ("get", "/api/v1/guilds/{gid}/automod/rules", "Auto-moderation rules", True, "MANAGE_GUILD"),
    ("post", "/api/v1/guilds/{gid}/automod/rules", "Create rule {name, kind: keyword|mention_spam|link, action: block|timeout|alert, …}", True, "MANAGE_GUILD"),
    ("patch", "/api/v1/guilds/{gid}/automod/rules/{rid}", "Update rule", True, "MANAGE_GUILD"),
    ("delete", "/api/v1/guilds/{gid}/automod/rules/{rid}", "Delete rule", True, "MANAGE_GUILD"),
    ("get", "/api/v1/guilds/{gid}/emojis", "Custom emojis", True, "member"),
    ("post", "/api/v1/guilds/{gid}/emojis", "Create emoji {name, key, animated}", True, "MANAGE_EMOJIS"),
    ("delete", "/api/v1/guilds/{gid}/emojis/{eid}", "Delete emoji", True, "MANAGE_EMOJIS"),
    ("get", "/api/v1/guilds/{gid}/stickers", "Stickers", True, "member"),
    ("post", "/api/v1/guilds/{gid}/stickers", "Create sticker {name, key, format}", True, "MANAGE_EMOJIS"),
    ("delete", "/api/v1/guilds/{gid}/stickers/{sid}", "Delete sticker", True, "MANAGE_EMOJIS"),
    ("post", "/api/v1/guilds/{gid}/uploads/{kind}", "Signed upload URL for icon|emoji|sticker|member-avatar|member-banner", True, "MANAGE_GUILD / MANAGE_EMOJIS; member-*: membership"),
    ("get", "/api/v1/guilds/{gid}/messages/search", "Search ?q&author_id&channel_id&has&offset&limit", True, "VIEW_CHANNEL+READ_MESSAGE_HISTORY per channel"),
    ("get", "/api/v1/guilds/{gid}/webhooks", "Webhooks", True, "MANAGE_WEBHOOKS"),
    ("post", "/api/v1/guilds/{gid}/bots", "Add a bot {application_id, permissions}", True, "MANAGE_GUILD"),
    ("get", "/api/v1/guilds/{gid}/commands", "Slash commands available in the guild", True, "member"),
    ("get", "/api/v1/channels/{cid}", "Channel", True, "VIEW_CHANNEL"),
    ("patch", "/api/v1/channels/{cid}", "Update {name, topic, position, parent_id, nsfw, slowmode_seconds}", True, "MANAGE_CHANNELS"),
    ("delete", "/api/v1/channels/{cid}", "Delete channel / close DM", True, "MANAGE_CHANNELS"),
    ("put", "/api/v1/channels/{cid}/overwrites/{target}", "Set overwrite {type, allow, deny}", True, "MANAGE_ROLES; hierarchy; only held bits"),
    ("delete", "/api/v1/channels/{cid}/overwrites/{target}", "Delete overwrite", True, "MANAGE_ROLES; hierarchy"),
    ("post", "/api/v1/channels/{cid}/threads", "Create thread {name, message_id} or forum post {name, content}", True, "CREATE_THREADS"),
    ("post", "/api/v1/channels/{cid}/threads/{tid}/archive", "Archive thread", True, "owner or MANAGE_THREADS"),
    ("post", "/api/v1/channels/{cid}/forum-tags", "Create forum tag", True, "MANAGE_CHANNELS"),
    ("delete", "/api/v1/channels/{cid}/forum-tags/{tag}", "Delete forum tag", True, "MANAGE_CHANNELS"),
    ("patch", "/api/v1/channels/{cid}/followers", "Follow an announcement channel {webhook_channel_id}", True, "MANAGE_WEBHOOKS in target"),
    ("get", "/api/v1/channels/{cid}/invites", "Invites", True, "MANAGE_CHANNELS"),
    ("post", "/api/v1/channels/{cid}/invites", "Create invite {max_uses, max_age}", True, "CREATE_INVITE"),
    ("get", "/api/v1/invites/{code}", "Invite preview", False, "rate-limited per IP"),
    ("post", "/api/v1/invites/{code}", "Accept invite → Guild", True, "not banned"),
    ("delete", "/api/v1/invites/{code}", "Delete invite", True, "inviter or MANAGE_GUILD"),
    ("get", "/api/v1/channels/{cid}/messages", "History ?before|after|around&limit (oldest → newest)", True, "READ_MESSAGE_HISTORY"),
    ("post", "/api/v1/channels/{cid}/messages", "Send {content, nonce, reply_to_id, attachment_ids, sticker_id, e2ee_payload}", True, "SEND_MESSAGES; idempotent by nonce; slowmode; automod"),
    ("get", "/api/v1/channels/{cid}/messages/{mid}", "Message", True, "READ_MESSAGE_HISTORY"),
    ("patch", "/api/v1/channels/{cid}/messages/{mid}", "Edit {content | e2ee_payload | suppress_embeds}", True, "author (+SEND_MESSAGES) or MANAGE_MESSAGES for suppress"),
    ("delete", "/api/v1/channels/{cid}/messages/{mid}", "Delete", True, "author or MANAGE_MESSAGES"),
    ("post", "/api/v1/channels/{cid}/messages/bulk-delete", "Bulk delete {ids}", True, "MANAGE_MESSAGES"),
    ("post", "/api/v1/channels/{cid}/messages/{mid}/ack", "Mark read", True, "VIEW_CHANNEL"),
    ("post", "/api/v1/channels/{cid}/messages/{mid}/crosspost", "Publish announcement to followers", True, "MANAGE_MESSAGES"),
    ("get", "/api/v1/channels/{cid}/messages/search", "Search in a channel / DM", True, "READ_MESSAGE_HISTORY"),
    ("get", "/api/v1/channels/{cid}/messages/{mid}/reactions/{emoji}", "Users who reacted", True, "READ_MESSAGE_HISTORY"),
    ("delete", "/api/v1/channels/{cid}/messages/{mid}/reactions/{emoji}", "Remove all reactions of an emoji", True, "MANAGE_MESSAGES"),
    ("put", "/api/v1/channels/{cid}/messages/{mid}/reactions/{emoji}/@me", "React", True, "ADD_REACTIONS"),
    ("delete", "/api/v1/channels/{cid}/messages/{mid}/reactions/{emoji}/@me", "Remove own reaction", True, ""),
    ("delete", "/api/v1/channels/{cid}/messages/{mid}/reactions/{emoji}/{uid}", "Remove someone's reaction", True, "MANAGE_MESSAGES"),
    ("get", "/api/v1/channels/{cid}/pins", "Pinned messages", True, "READ_MESSAGE_HISTORY"),
    ("put", "/api/v1/channels/{cid}/pins/{mid}", "Pin (max 50)", True, "MANAGE_MESSAGES (any recipient in DMs)"),
    ("delete", "/api/v1/channels/{cid}/pins/{mid}", "Unpin", True, "MANAGE_MESSAGES"),
    ("post", "/api/v1/channels/{cid}/typing", "Typing indicator", True, "SEND_MESSAGES"),
    ("post", "/api/v1/channels/{cid}/attachments", "Presigned upload {filename, content_type, size} → {id, upload_url, url}", True, "ATTACH_FILES"),
    ("post", "/api/v1/uploads/{kind}", "Signed upload URL for avatar|banner {filename, content_type, size} → {key, upload_url, url}", True, "png/jpeg/webp/gif ≤ 8 MiB"),
    ("put", "/api/v1/files/{key}", "Upload bytes to local storage (signed URL from an uploads endpoint)", False, "NEOSHARD_FILES_DIR; signature binds key, expiry, size, type"),
    ("get", "/files/{key}", "Serve a stored object", False, "images inline, everything else as a download; sandbox CSP"),
    ("get", "/api/v1/channels/{cid}/webhooks", "Channel webhooks", True, "MANAGE_WEBHOOKS"),
    ("post", "/api/v1/channels/{cid}/webhooks", "Create webhook {name} → {id, token, url}", True, "MANAGE_WEBHOOKS"),
    ("delete", "/api/v1/webhooks/{wid}", "Delete webhook", True, "MANAGE_WEBHOOKS"),
    ("post", "/api/v1/webhooks/{wid}/{token}", "Execute webhook {content, username}", False, "token in path; cannot ping @everyone"),
    ("get", "/api/v1/applications", "My bot applications", True, ""),
    ("post", "/api/v1/applications", "Create application + bot user → {id, bot_user_id, token}", True, "not bots"),
    ("delete", "/api/v1/applications/{aid}", "Delete application", True, "owner"),
    ("post", "/api/v1/applications/{aid}/reset-token", "New bot token (old one revoked)", True, "owner"),
    ("get", "/api/v1/applications/{aid}/commands", "Global commands", True, "owner or bot"),
    ("put", "/api/v1/applications/{aid}/commands", "Overwrite global commands [{name, description, options}]", True, "owner or bot"),
    ("put", "/api/v1/applications/{aid}/guilds/{gid}/commands", "Overwrite guild commands", True, "owner or bot"),
    ("post", "/api/v1/interactions", "Invoke a slash command {application_id, channel_id, command, options}", True, "USE_COMMANDS"),
    ("post", "/api/v1/interactions/{iid}/{token}/callback", "Bot reply {content, ephemeral} (once, 15 min)", False, "interaction token"),
    ("post", "/api/v1/channels/{cid}/voice", "LiveKit token + TURN credentials → {url, token, room, grants, turn}", True, "CONNECT (SPEAK/VIDEO/STREAM → publish grants)"),
    ("get", "/api/v1/turn", "TURN credentials (coturn REST format)", True, ""),
    ("post", "/api/v1/mls/key-packages", "Upload MLS KeyPackages {device_id, key_packages[b64]}", True, ""),
    ("post", "/api/v1/users/{uid}/mls/key-package", "Claim one KeyPackage per device", True, "rate-limited"),
    ("post", "/api/v1/channels/{cid}/mls", "Relay MLS commit/welcome/proposal {kind, payload, to}", True, "E2EE channel recipient"),
    ("post", "/api/v1/push/subscriptions", "Web Push subscription {endpoint, keys}", True, ""),
    ("delete", "/api/v1/push/subscriptions/{id}", "Remove push subscription", True, ""),
    ("post", "/api/v1/federation/resolve/{address}", "Resolve user@instance into a local (shadow) user", True, "federation enabled"),
    ("get", "/federation/v1/users/{username}", "Federation: public profile", False, ""),
    ("post", "/federation/v1/inbox", "Federation: signed event (spec/federation.md)", False, "Ed25519 signature"),
]

print("""# Generated by spec/tools/gen_openapi.py — do not edit by hand.
# SPDX-License-Identifier: CC-BY-4.0
openapi: 3.1.0
info:
  title: NeoShard REST API
  version: "1.0.0"
  license: { name: CC-BY-4.0, url: https://creativecommons.org/licenses/by/4.0/ }
  description: |
    JSON = canonical proto3 JSON of spec/proto with snake_case names: 64-bit ids are
    strings, enums are names, defaults may be omitted. Errors are
    {"code","message"} (+ "missing_permissions"). 429 responses carry Retry-After.
    Realtime events: Gateway (spec/gateway.md, spec/asyncapi.yaml).
servers:
  - url: https://neoshard.ru
security:
  - bearer: []
components:
  securitySchemes:
    bearer: { type: http, scheme: bearer, description: "User token ns_…, bot token nsb_… (also accepted as 'Bot <token>')" }
  schemas:
    Error:
      type: object
      required: [code, message]
      properties:
        code: { type: string, enum: [bad_request, unauthorized, invalid_credentials, missing_permissions, forbidden, not_found, conflict, rate_limited, unavailable, internal] }
        message: { type: string }
        missing_permissions: { type: string, description: decimal bitmask }
    ProtoJson:
      description: A message from spec/proto/neoshard/v1/models.proto in proto3 JSON.
      type: object
  responses:
    Error: { description: Error, content: { application/json: { schema: { $ref: "#/components/schemas/Error" } } } }
paths:""")
by_path = {}
for m, p, s, auth, note in ROUTES:
    by_path.setdefault(p, []).append((m, s, auth, note))
for p, ops in by_path.items():
    print(f"  {p}:")
    for m, s, auth, note in ops:
        print(f"    {m}:")
        summary = s.replace('"', "'")
        print(f'      summary: "{summary}"')
        if note:
            print(f"      description: \"{note}\"")
        if not auth:
            print("      security: []")
        params = [seg[1:-1] for seg in p.split("/") if seg.startswith("{")]
        if params:
            print("      parameters:")
            for name in params:
                print(f"        - {{ name: {name}, in: path, required: true, schema: {{ type: string }} }}")
        if m in ("post", "put", "patch"):
            print("      requestBody: { required: false, content: { application/json: { schema: { type: object } } } }")
        print("      responses:")
        print("        \"200\": { description: OK, content: { application/json: { schema: { $ref: \"#/components/schemas/ProtoJson\" } } } }")
        print("        default: { $ref: \"#/components/responses/Error\" }")
