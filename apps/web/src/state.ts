// UI state = a projection of core updates into a Solid store. No business
// logic: permissions, retries, caching and ordering decisions are made in
// neoshard-core; this file only mirrors what the core reports.

import { batch } from "solid-js";
import { createStore, produce, reconcile } from "solid-js/store";
import type {
  Channel,
  Guild,
  Member,
  Message,
  NeoShardCore,
  ReadState,
  Role,
  Update,
  User,
  VoiceState,
} from "@neoshard/core";
import { AuthPhase, ConnectionState, RelationshipKind, Status } from "@neoshard/core";

export type Id = string;
export const sid = (v: bigint | number | string): Id => String(v);

export interface MessageView extends Omit<Message, "$typeName"> {
  key: string;
  pending?: boolean;
  failed?: string;
  retryable?: boolean;
}

export interface GuildView {
  id: Id;
  name: string;
  icon: string;
  ownerId: Id;
  channels: Channel[];
  roles: Role[];
  memberCount: number;
  self?: Member;
  voice: VoiceState[];
  tag: string;
  tagBadge: string;
  emojis: Guild["emojis"];
  stickers: Guild["stickers"];
}

export interface PresenceView {
  status: Status;
  custom: string;
  emoji: string;
  /** unix millis, 0 = never */
  expires: number;
}

export interface AppState {
  auth: { phase: AuthPhase; error: string };
  connection: { state: ConnectionState; retryInMs: number; reason: string };
  me?: User;
  guildOrder: Id[];
  guilds: Record<Id, GuildView>;
  dms: Record<Id, Channel>;
  /** Who is in a call in each DM / group DM (by channel id). */
  dmVoice: Record<Id, VoiceState[]>;
  messages: Record<Id, MessageView[]>;
  reachedStart: Record<Id, boolean>;
  readStates: Record<Id, { last: Id; mentions: number }>;
  presence: Record<Id, PresenceView>;
  /** Our own status as last set on this device (others see it via presence). */
  myPresence: PresenceView;
  /** Prefix for object keys (avatars, banners) from Ready. */
  filesBase: string;
  /** Friends, blocks and pending requests by user id. */
  relationships: Record<Id, { kind: RelationshipKind; since: number }>;
  users: Record<Id, User>;
  typing: Record<Id, Record<Id, number>>;
  perms: Record<Id, bigint>;
  members: Record<Id, Member[]>;
  memberTotal: Record<Id, number>;
  selection: { guild: Id | "@me"; channel?: Id };
  results: Record<string, { ok: boolean; error: string }>;
}

const initial: AppState = {
  auth: { phase: AuthPhase.LOGGED_OUT, error: "" },
  connection: { state: ConnectionState.DISCONNECTED, retryInMs: 0, reason: "" },
  guildOrder: [],
  guilds: {},
  dms: {},
  dmVoice: {},
  messages: {},
  reachedStart: {},
  readStates: {},
  presence: {},
  myPresence: { status: Status.ONLINE, custom: "", emoji: "", expires: 0 },
  filesBase: "",
  relationships: {},
  users: {},
  typing: {},
  perms: {},
  members: {},
  memberTotal: {},
  selection: { guild: "@me" },
  results: {},
};

export const [state, setState] = createStore<AppState>(structuredClone(initial));

const GUILD_ORDER_KEY = "neoshard.guildOrder";

function guildView(g: Guild): GuildView {
  return {
    id: sid(g.id),
    name: g.name,
    icon: g.icon,
    ownerId: sid(g.ownerId),
    channels: [...g.channels].sort((a, b) => a.position - b.position || Number(a.id - b.id)),
    roles: g.roles,
    memberCount: g.memberCount,
    self: g.selfMember,
    voice: g.voiceStates,
    tag: g.tag,
    tagBadge: g.tagBadge,
    emojis: g.emojis,
    stickers: g.stickers,
  };
}

function presenceView(p: { status: Status; customStatus: string; customEmoji: string; customExpiresAt: bigint }): PresenceView {
  return { status: p.status, custom: p.customStatus, emoji: p.customEmoji, expires: Number(p.customExpiresAt) };
}

/** Resolve an object key (or an absolute URL from a federated instance) to a URL. */
export function asset(key: string | undefined): string | undefined {
  if (!key) return undefined;
  if (/^https?:\/\//.test(key)) return key;
  return state.filesBase ? `${state.filesBase}/${key}` : undefined;
}

function view(m: Message, extra: Partial<MessageView> = {}): MessageView {
  return { ...m, key: m.id ? sid(m.id) : `n:${m.nonce}`, ...extra };
}

/** Confirmed messages sorted by id, then pending ones (id 0) in send order. */
export function upsertMessage(list: MessageView[], m: MessageView): void {
  const i = list.findIndex((x) => (m.id !== 0n && x.id === m.id) || (m.nonce !== "" && x.nonce === m.nonce && x.id === 0n));
  if (i >= 0) {
    const key = list[i].key;
    list.splice(i, 1);
    m = { ...m, key };
  }
  if (m.id === 0n) {
    list.push(m);
    return;
  }
  // Binary search among confirmed messages only (they form a sorted prefix).
  let hi = list.findIndex((x) => x.id === 0n);
  if (hi < 0) hi = list.length;
  let lo = 0;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (list[mid].id < m.id) lo = mid + 1;
    else hi = mid;
  }
  list.splice(lo, 0, m);
}

function rememberUser(s: AppState, u?: User) {
  if (u) s.users[sid(u.id)] = u;
}

function applyEvent(s: AppState, e: NonNullable<Extract<Update["kind"], { case: "event" }>["value"]>) {
  const k = e.kind;
  switch (k.case) {
    case "guildCreate": {
      const g = guildView(k.value);
      s.guilds[g.id] = g;
      if (!s.guildOrder.includes(g.id)) s.guildOrder.push(g.id);
      break;
    }
    case "guildUpdate": {
      const g = s.guilds[sid(k.value.id)];
      if (g) Object.assign(g, { name: k.value.name, icon: k.value.icon, ownerId: sid(k.value.ownerId), tag: k.value.tag, tagBadge: k.value.tagBadge, emojis: k.value.emojis, stickers: k.value.stickers });
      // Members showing this server's tag see the new text without a per-user event.
      for (const u of Object.values(s.users)) {
        if (u.guildTag && u.guildTag.guildId === k.value.id) {
          if (k.value.tag) Object.assign(u.guildTag, { tag: k.value.tag, badge: k.value.tagBadge });
          else u.guildTag = undefined;
        }
      }
      break;
    }
    case "guildDelete": {
      const id = sid(k.value.id);
      delete s.guilds[id];
      s.guildOrder = s.guildOrder.filter((x) => x !== id);
      if (s.selection.guild === id) s.selection = { guild: "@me" };
      break;
    }
    case "channelCreate":
    case "channelUpdate": {
      const c = k.value;
      if (c.guildId === 0n) {
        s.dms[sid(c.id)] = c;
        break;
      }
      const g = s.guilds[sid(c.guildId)];
      if (!g) break;
      const i = g.channels.findIndex((x) => x.id === c.id);
      if (i >= 0) g.channels[i] = c;
      else g.channels.push(c);
      g.channels.sort((a, b) => a.position - b.position || Number(a.id - b.id));
      break;
    }
    case "channelDelete": {
      const cid = sid(k.value.id);
      if (k.value.guildId === 0n) delete s.dms[cid];
      else {
        const g = s.guilds[sid(k.value.guildId)];
        if (g) g.channels = g.channels.filter((c) => c.id !== k.value.id);
      }
      delete s.messages[cid];
      if (s.selection.channel === cid) s.selection.channel = undefined;
      break;
    }
    case "roleCreate":
    case "roleUpdate": {
      const g = s.guilds[sid(k.value.guildId)];
      if (!g) break;
      const i = g.roles.findIndex((r) => r.id === k.value.id);
      if (i >= 0) g.roles[i] = k.value;
      else g.roles.push(k.value);
      g.roles.sort((a, b) => b.position - a.position);
      break;
    }
    case "roleDelete": {
      const g = s.guilds[sid(k.value.guildId)];
      if (g) g.roles = g.roles.filter((r) => r.id !== k.value.id);
      break;
    }
    case "memberAdd":
      rememberUser(s, k.value.user);
      if (s.guilds[sid(k.value.guildId)]) s.guilds[sid(k.value.guildId)].memberCount++;
      break;
    case "memberUpdate": {
      rememberUser(s, k.value.user);
      const g = s.guilds[sid(k.value.guildId)];
      if (g && k.value.user?.id === s.me?.id) g.self = k.value;
      const list = s.members[sid(k.value.guildId)];
      if (list) {
        const i = list.findIndex((m) => m.user?.id === k.value.user?.id);
        if (i >= 0) list[i] = k.value;
      }
      break;
    }
    case "memberRemove": {
      const g = s.guilds[sid(k.value.guildId)];
      if (g) g.memberCount = Math.max(0, g.memberCount - 1);
      const list = s.members[sid(k.value.guildId)];
      if (list) s.members[sid(k.value.guildId)] = list.filter((m) => m.user?.id !== k.value.userId);
      break;
    }
    case "messageCreate":
    case "messageUpdate": {
      const m = k.value;
      rememberUser(s, m.author);
      const cid = sid(m.channelId);
      if (k.case === "messageUpdate" && !s.messages[cid]) break;
      s.messages[cid] ??= [];
      upsertMessage(s.messages[cid], view(m));
      if (k.case === "messageCreate") {
        // Whoever sent a message is no longer typing.
        if (s.typing[cid] && m.author) delete s.typing[cid][sid(m.author.id)];
        bumpLastMessage(s, m);
      }
      break;
    }
    case "messageDelete": {
      const cid = sid(k.value.channelId);
      if (s.messages[cid]) s.messages[cid] = s.messages[cid].filter((m) => m.id !== k.value.id);
      break;
    }
    case "messageDeleteBulk": {
      const cid = sid(k.value.channelId);
      const ids = new Set(k.value.ids);
      if (s.messages[cid]) s.messages[cid] = s.messages[cid].filter((m) => !ids.has(m.id));
      break;
    }
    case "reactionAdd":
    case "reactionRemove": {
      const r = k.value;
      const m = s.messages[sid(r.channelId)]?.find((x) => x.id === r.messageId);
      if (!m) break;
      const mine = r.userId === s.me?.id;
      const i = m.reactions.findIndex((x) => x.emoji === r.emoji);
      if (k.case === "reactionAdd") {
        if (i >= 0) {
          m.reactions[i].count++;
          if (mine) m.reactions[i].me = true;
        } else m.reactions.push({ $typeName: "neoshard.v1.ReactionCount", emoji: r.emoji, count: 1, me: mine });
      } else if (i >= 0) {
        m.reactions[i].count--;
        if (mine) m.reactions[i].me = false;
        if (m.reactions[i].count <= 0) m.reactions.splice(i, 1);
      }
      break;
    }
    case "presenceUpdate":
      s.presence[sid(k.value.userId)] = presenceView(k.value);
      break;
    case "typingStart": {
      const cid = sid(k.value.channelId);
      s.typing[cid] ??= {};
      s.typing[cid][sid(k.value.userId)] = Date.now() + 10_000;
      break;
    }
    case "readStateUpdate":
      s.readStates[sid(k.value.channelId)] = { last: sid(k.value.lastReadMessageId), mentions: k.value.mentionCount };
      break;
    case "userUpdate":
      rememberUser(s, k.value);
      if (s.me?.id === k.value.id) s.me = k.value;
      break;
    case "relationshipAdd": {
      const r = k.value;
      if (!r.user) break;
      rememberUser(s, r.user);
      s.relationships[sid(r.user.id)] = { kind: r.kind, since: Number(r.since) };
      break;
    }
    case "relationshipRemove":
      delete s.relationships[sid(k.value.userId)];
      break;
    case "voiceStateUpdate": {
      if (k.value.guildId === 0n) {
        // DM calls: a user is in at most one call, so drop them everywhere first.
        for (const cid of Object.keys(s.dmVoice)) {
          s.dmVoice[cid] = s.dmVoice[cid].filter((v) => v.userId !== k.value.userId);
          if (!s.dmVoice[cid].length) delete s.dmVoice[cid];
        }
        if (k.value.channelId !== 0n) (s.dmVoice[sid(k.value.channelId)] ??= []).push(k.value);
        break;
      }
      const g = s.guilds[sid(k.value.guildId)];
      if (!g) break;
      g.voice = g.voice.filter((v) => v.userId !== k.value.userId);
      if (k.value.channelId !== 0n) g.voice.push(k.value);
      break;
    }
    case "memberListChunk": {
      const gid = sid(k.value.guildId);
      s.memberTotal[gid] = k.value.total;
      const list = (s.members[gid] ??= []);
      k.value.members.forEach((m, i) => {
        list[k.value.start + i] = m;
        rememberUser(s, m.user);
      });
      for (const p of k.value.presences) s.presence[sid(p.userId)] = presenceView(p);
      break;
    }
    default:
      break;
  }
}

function bumpLastMessage(s: AppState, m: Message) {
  const cid = m.channelId;
  if (m.guildId === 0n) {
    const c = s.dms[sid(cid)];
    if (c && c.lastMessageId < m.id) c.lastMessageId = m.id;
    return;
  }
  const c = s.guilds[sid(m.guildId)]?.channels.find((x) => x.id === cid);
  if (c && c.lastMessageId < m.id) c.lastMessageId = m.id;
  if (m.author?.id === s.me?.id) s.readStates[sid(cid)] = { last: sid(m.id), mentions: 0 };
}

export function applyUpdates(updates: Update[]): void {
  batch(() =>
    setState(
      produce((s) => {
        for (const u of updates) {
          const k = u.kind;
          switch (k.case) {
            case "auth":
              s.auth = { phase: k.value.phase, error: k.value.error };
              if (k.value.phase === AuthPhase.LOGGED_OUT) {
                // s.auth is a store proxy here: clone only the plain initial state.
                Object.assign(s, structuredClone(initial), { auth: { phase: k.value.phase, error: k.value.error } });
                s.me = undefined;
              }
              break;
            case "connection":
              s.connection = { state: k.value.state, retryInMs: k.value.retryInMs, reason: k.value.reason };
              break;
            case "ready": {
              const r = k.value;
              s.me = r.user;
              s.filesBase = r.filesBase;
              s.relationships = {};
              for (const rel of r.relationships) {
                if (!rel.user) continue;
                rememberUser(s, rel.user);
                s.relationships[sid(rel.user.id)] = { kind: rel.kind, since: Number(rel.since) };
              }
              rememberUser(s, r.user);
              const guilds: Record<Id, GuildView> = {};
              for (const g of r.guilds) guilds[sid(g.id)] = guildView(g);
              s.guilds = guilds;
              s.guildOrder = savedOrder(Object.keys(guilds));
              s.dms = Object.fromEntries(r.privateChannels.map((c) => [sid(c.id), c]));
              s.readStates = Object.fromEntries(r.readStates.map((rs: ReadState) => [sid(rs.channelId), { last: sid(rs.lastReadMessageId), mentions: rs.mentionCount }]));
              for (const p of r.presences) {
                // Our own entry carries the real status (also Invisible) and the stored custom status.
                if (p.userId === r.user?.id) s.myPresence = presenceView(p);
                else s.presence[sid(p.userId)] = presenceView(p);
              }
              break;
            }
            case "event":
              applyEvent(s, k.value);
              break;
            case "pendingMessage": {
              const m = k.value.message;
              if (!m) break;
              const cid = sid(m.channelId);
              s.messages[cid] ??= [];
              upsertMessage(s.messages[cid], view(m, { pending: true }));
              break;
            }
            case "pendingSent": {
              const m = k.value.message;
              if (!m) break;
              const cid = sid(m.channelId);
              s.messages[cid] ??= [];
              const list = s.messages[cid];
              const i = list.findIndex((x) => x.nonce === k.value.nonce && x.id === 0n);
              const dup = list.findIndex((x) => x.id === m.id);
              if (i >= 0 && dup >= 0) list.splice(i, 1);
              else if (i >= 0) list[i] = view(m, { key: list[i].key });
              else upsertMessage(list, view(m));
              break;
            }
            case "pendingFailed": {
              const list = s.messages[sid(k.value.channelId)];
              const m = list?.find((x) => x.nonce === k.value.nonce && x.id === 0n);
              if (m) Object.assign(m, { pending: false, failed: k.value.reason, retryable: k.value.retryable });
              break;
            }
            case "history": {
              const cid = sid(k.value.channelId);
              s.messages[cid] ??= [];
              for (const m of k.value.messages) {
                rememberUser(s, m.author);
                upsertMessage(s.messages[cid], view(m));
              }
              if (k.value.reachedStart) s.reachedStart[cid] = true;
              break;
            }
            case "permissions":
              s.perms[sid(k.value.channelId)] = k.value.permissions;
              break;
            case "result":
              s.results[sid(k.value.requestId)] = { ok: k.value.ok, error: k.value.error };
              break;
            default:
              break;
          }
        }
      }),
    ),
  );
}

// ---------------------------------------------------------------- guild order (local preference)

function savedOrder(ids: Id[]): Id[] {
  let saved: Id[] = [];
  try {
    saved = JSON.parse(localStorage.getItem(GUILD_ORDER_KEY) ?? "[]");
  } catch {
    saved = [];
  }
  const known = saved.filter((x) => ids.includes(x));
  return [...known, ...ids.filter((x) => !known.includes(x))];
}

export function setGuildOrder(order: Id[]): void {
  setState("guildOrder", reconcile(order));
  try {
    localStorage.setItem(GUILD_ORDER_KEY, JSON.stringify(order));
  } catch {
    // private mode
  }
}

// ---------------------------------------------------------------- core wiring

let coreRef: NeoShardCore | undefined;
export const core = (): NeoShardCore => {
  if (!coreRef) throw new Error("core not loaded");
  return coreRef;
};

const pendingApi = new Map<bigint, (r: { status: number; body: unknown }) => void>();

export function attachCore(c: NeoShardCore): void {
  coreRef = c;
  c.subscribe((updates) => {
    for (const u of updates) {
      if (u.kind.case === "api") {
        const cb = pendingApi.get(u.kind.value.requestId);
        if (cb) {
          pendingApi.delete(u.kind.value.requestId);
          let body: unknown = null;
          try {
            body = u.kind.value.bodyJson ? JSON.parse(u.kind.value.bodyJson) : null;
          } catch {
            body = null;
          }
          cb({ status: u.kind.value.status, body });
        }
      }
    }
    applyUpdates(updates);
    onBatch?.(updates);
  });
}

let onBatch: ((u: Update[]) => void) | undefined;
/** Extra observer of update batches (plugins). */
export function observeUpdates(f: (u: Update[]) => void): void {
  onBatch = f;
}

/** Authenticated REST call through the core. Resolves with status and parsed JSON
 * (status 0 = no response within 30 s). */
export function api<T = unknown>(method: string, path: string, body?: unknown): Promise<{ status: number; body: T }> {
  return new Promise((resolve) => {
    const id = core().send({ case: "api", value: { method, path, bodyJson: body === undefined ? "" : JSON.stringify(body) } });
    const timer = setTimeout(() => {
      if (pendingApi.delete(id)) resolve({ status: 0, body: null as T });
    }, 30_000);
    pendingApi.set(id, (r) => {
      clearTimeout(timer);
      resolve(r as { status: number; body: T });
    });
  });
}

export function resetForTests(): void {
  setState(reconcile(structuredClone(initial)));
}
