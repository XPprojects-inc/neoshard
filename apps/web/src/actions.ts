// UI actions shared by several screens. They only send commands to the core or
// call the REST API through it; decisions stay on the server and in the core.

import { fromJson, Status, UserSchema, type User } from "@neoshard/core";
import { toast, type StatusName } from "@neoshard/ui";

import type { MentionResolver } from "./markdown";
import { api, asset, core, setState, sid, state, type Id } from "./state";

export const statusName = (s?: Status): StatusName =>
  s === Status.ONLINE ? "online" : s === Status.IDLE ? "idle" : s === Status.DND ? "dnd" : "offline";

export const STATUS_LABEL: Record<StatusName | "invisible", string> = {
  online: "В сети",
  idle: "Неактивен",
  dnd: "Не беспокоить",
  offline: "Не в сети",
  invisible: "Невидимка",
};

export function select(guild: Id | "@me", channel?: Id): void {
  setState("selection", { guild, channel });
  if (channel) core().send({ case: "selectChannel", value: { channelId: BigInt(channel) } });
}

/** Open (or create) a DM; `e2ee` asks for an MLS-encrypted one. Returns the channel id. */
export async function openDm(userId: bigint | Id, e2ee = false): Promise<Id | undefined> {
  if (e2ee && !core().e2ee) {
    toast("Включите шифрование личных сообщений в настройках этого устройства", "error");
    return undefined;
  }
  const r = await api<{ id: string }>("POST", "/users/@me/channels", { recipient_id: String(userId), e2ee });
  if (r.status === 200 || r.status === 201) {
    select("@me", r.body.id);
    return r.body.id;
  }
  toast(r.status === 403 ? "Этот пользователь не принимает личные сообщения" : "Не удалось открыть беседу", "error");
  return undefined;
}

/** Our status on this device, plus the custom status (text, emoji, expiry). */
export function setMyPresence(p: Partial<{ status: Status; custom: string; emoji: string; expires: number }>): void {
  const next = { ...state.myPresence, ...p };
  setState("myPresence", next);
  core().send({
    case: "setPresence",
    value: { status: next.status, customStatus: next.custom, customEmoji: next.emoji, customExpiresAt: BigInt(next.expires) },
  });
}

/** A custom status that has not expired yet. */
export function activeCustom(p: { custom: string; emoji: string; expires: number } | undefined): { text: string; emoji: string } | undefined {
  if (!p || (!p.custom && !p.emoji)) return undefined;
  if (p.expires && p.expires <= Date.now()) return undefined;
  return { text: p.custom, emoji: p.emoji };
}

export const mentionResolver: MentionResolver = {
  user: (id) => state.users[id]?.displayName || state.users[id]?.username,
  role: (id) => {
    const g = state.guilds[state.selection.guild];
    const r = g?.roles.find((x) => sid(x.id) === id);
    return r ? { name: r.name, color: r.color } : undefined;
  },
  isMe: (id) => sid(state.me?.id ?? 0n) === id,
  emoji: (name) => {
    const here = state.guilds[state.selection.guild]?.emojis.find((e) => e.name === name);
    const any = here ?? Object.values(state.guilds).flatMap((g) => g.emojis).find((e) => e.name === name);
    return any ? asset(any.key) : undefined;
  },
};

const requested = new Set<string>();

/** Fill the user cache for ids we only know by id (DM partners, audit log). */
export function ensureUsers(ids: Iterable<string>): void {
  for (const id of ids) {
    if (!id || id === "0" || state.users[id] || requested.has(id)) continue;
    requested.add(id);
    void api<unknown>("GET", `/users/${id}`).then((r) => {
      if (r.status !== 200 || !r.body) return;
      try {
        setState("users", id, fromJson(UserSchema, r.body as never, { ignoreUnknownFields: true }) as User);
      } catch {
        // malformed profile: keep showing the id
      }
    });
  }
}

/** Members of all my servers into the user cache (for pickers like the group DM dialog). */
export async function loadGuildMembers(): Promise<void> {
  for (const gid of state.guildOrder) {
    const r = await api<unknown[]>("GET", `/guilds/${gid}/members?limit=1000`);
    if (r.status !== 200) continue;
    for (const m of r.body ?? []) {
      const u = (m as { user?: { id?: string } }).user;
      if (u?.id && !state.users[u.id]) {
        try {
          setState("users", u.id, fromJson(UserSchema, u as never, { ignoreUnknownFields: true }) as User);
        } catch {
          // skip
        }
      }
    }
  }
}

/** execCommand fallback: Android WebView and some mobile browsers refuse the Clipboard API. */
function copyViaSelection(text: string): boolean {
  const ta = document.createElement("textarea");
  ta.value = text;
  ta.setAttribute("readonly", "");
  ta.style.cssText = "position:fixed;top:0;left:0;width:1px;height:1px;opacity:0;";
  document.body.append(ta);
  ta.focus();
  ta.setSelectionRange(0, text.length);
  let ok = false;
  try {
    ok = document.execCommand("copy");
  } catch {
    ok = false;
  }
  ta.remove();
  return ok;
}

export async function copy(text: string, what = "Скопировано"): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
    toast(what);
    return;
  } catch {
    // fall through
  }
  if (copyViaSelection(text)) toast(what);
  else toast("Не удалось скопировать — выделите ссылку и скопируйте вручную", "error");
}

/** The system share sheet (phones), when there is one. */
export const canShare = () => typeof navigator.share === "function";

export async function share(url: string, title: string): Promise<void> {
  try {
    await navigator.share({ title, url });
  } catch (e) {
    // Closing the sheet is not an error.
    if ((e as DOMException)?.name !== "AbortError") await copy(url, "Ссылка скопирована");
  }
}

/** Highest coloured role of a member in a guild (for name colours). */
export function roleColor(guildId: Id | undefined, roleIds: readonly (bigint | Id)[] | undefined): string | undefined {
  if (!guildId || !roleIds?.length) return undefined;
  const g = state.guilds[guildId];
  if (!g) return undefined;
  const ids = new Set(roleIds.map((r) => sid(r)));
  const top = g.roles.filter((r) => ids.has(sid(r.id)) && r.color !== 0).sort((a, b) => b.position - a.position)[0];
  return top ? `#${top.color.toString(16).padStart(6, "0")}` : undefined;
}
