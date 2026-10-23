// Permission bit names mirror `neoshard_proto::permissions` (spec/permissions.md).
// The core computes effective permissions; the UI only tests bits it was given.

export const Permission = {
  VIEW_CHANNEL: 1n << 0n,
  SEND_MESSAGES: 1n << 1n,
  READ_MESSAGE_HISTORY: 1n << 2n,
  ADD_REACTIONS: 1n << 3n,
  ATTACH_FILES: 1n << 4n,
  EMBED_LINKS: 1n << 5n,
  MENTION_EVERYONE: 1n << 6n,
  MANAGE_MESSAGES: 1n << 7n,
  CREATE_THREADS: 1n << 8n,
  SEND_IN_THREADS: 1n << 9n,
  USE_EXTERNAL_EMOJI: 1n << 10n,
  USE_COMMANDS: 1n << 11n,
  MANAGE_THREADS: 1n << 12n,
  CONNECT: 1n << 16n,
  SPEAK: 1n << 17n,
  VIDEO: 1n << 18n,
  STREAM: 1n << 19n,
  MUTE_MEMBERS: 1n << 20n,
  DEAFEN_MEMBERS: 1n << 21n,
  MOVE_MEMBERS: 1n << 22n,
  PRIORITY_SPEAKER: 1n << 23n,
  CREATE_INVITE: 1n << 32n,
  CHANGE_NICKNAME: 1n << 33n,
  MANAGE_NICKNAMES: 1n << 34n,
  KICK_MEMBERS: 1n << 35n,
  BAN_MEMBERS: 1n << 36n,
  MODERATE_MEMBERS: 1n << 37n,
  MANAGE_CHANNELS: 1n << 40n,
  MANAGE_ROLES: 1n << 41n,
  MANAGE_GUILD: 1n << 42n,
  MANAGE_WEBHOOKS: 1n << 43n,
  MANAGE_EMOJIS: 1n << 44n,
  VIEW_AUDIT_LOG: 1n << 45n,
  ADMINISTRATOR: 1n << 63n,
} as const;

export function hasPermission(perms: bigint, bit: bigint): boolean {
  return (perms & Permission.ADMINISTRATOR) !== 0n || (perms & bit) === bit;
}
