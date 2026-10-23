// Profile data for the UI: fetching full profiles, merging the per-server
// profile over the user profile, uploads, and the "open profile" signal.
// No business rules here — the server validates everything.

import type { Anchor } from "@neoshard/ui";
import type { User } from "@neoshard/core";
import { createSignal } from "solid-js";

import { api, asset, sid, state, type Id } from "../state";

/** GET /users/{id}/profile as JSON (proto field names, 64-bit ids as strings). */
export interface ProfileJson {
  user?: { id: string; username: string; display_name?: string; avatar?: string; bot?: boolean };
  pronouns?: string;
  bio?: string;
  banner?: string;
  banner_color?: number;
  theme_colors?: number[];
  profile_effect?: string;
  created_at?: string;
  links?: Array<{ label: string; url: string; verified?: boolean }>;
  member?: {
    guild_id: string;
    nick?: string;
    avatar?: string;
    banner?: string;
    bio?: string;
    pronouns?: string;
    theme_colors?: number[];
    role_ids?: string[];
    joined_at?: string;
  };
  mutual_guild_ids?: string[];
  note?: string;
}

/** What a profile card shows (already merged with the server profile). */
export interface ProfileView {
  userId: Id;
  username: string;
  displayName: string;
  bot: boolean;
  avatar?: string;
  banner?: string;
  bannerColor?: number;
  theme?: [number, number];
  effect: string;
  decoration: string;
  nameStyle?: { font: string; effect: string; colors: number[] };
  tag?: { tag: string; badge: string; guildId: Id };
  pronouns: string;
  bio: string;
  createdAt: number;
  links: Array<{ label: string; url: string; verified: boolean }>;
  guildId?: Id;
  roleIds: Id[];
  joinedAt: number;
  mutualGuildIds: Id[];
  note: string;
  /** True when the card shows a per-server profile. */
  serverProfile: boolean;
}

export function viewFromJson(p: ProfileJson, u: User | undefined): ProfileView {
  const m = p.member;
  const theme = (m?.theme_colors?.length === 2 ? m.theme_colors : p.theme_colors?.length === 2 ? p.theme_colors : undefined) as [number, number] | undefined;
  return {
    userId: p.user?.id ?? sid(u?.id ?? 0n),
    username: p.user?.username ?? u?.username ?? "",
    displayName: m?.nick || p.user?.display_name || u?.displayName || p.user?.username || u?.username || "",
    bot: p.user?.bot ?? u?.bot ?? false,
    avatar: asset(m?.avatar || p.user?.avatar || u?.avatar),
    banner: asset(m?.banner || p.banner),
    bannerColor: p.banner_color ?? undefined,
    theme,
    effect: p.profile_effect ?? "",
    decoration: u?.avatarDecoration ?? "",
    nameStyle: u?.nameStyle ? { font: u.nameStyle.font, effect: u.nameStyle.effect, colors: [...u.nameStyle.colors] } : undefined,
    tag: u?.guildTag?.tag ? { tag: u.guildTag.tag, badge: u.guildTag.badge, guildId: sid(u.guildTag.guildId) } : undefined,
    pronouns: m?.pronouns || p.pronouns || "",
    bio: m?.bio || p.bio || "",
    createdAt: Number(p.created_at ?? 0),
    links: (p.links ?? []).map((l) => ({ label: l.label, url: l.url, verified: !!l.verified })),
    guildId: m?.guild_id,
    roleIds: m?.role_ids ?? [],
    joinedAt: Number(m?.joined_at ?? 0),
    mutualGuildIds: p.mutual_guild_ids ?? [],
    note: p.note ?? "",
    serverProfile: !!(m && (m.avatar || m.banner || m.bio || m.pronouns || m.theme_colors?.length || m.nick)),
  };
}

/** Minimal view from the cached user only (before the full profile arrives). */
export function viewFromUser(u: User | undefined, id: Id): ProfileView {
  return viewFromJson({ user: u ? { id: sid(u.id), username: u.username, display_name: u.displayName, avatar: u.avatar, bot: u.bot } : { id, username: "" } }, u);
}

const cache = new Map<string, { at: number; json: ProfileJson }>();
const TTL = 60_000;

export async function fetchProfile(userId: Id, guildId?: Id, fresh = false): Promise<ProfileJson | undefined> {
  const key = `${userId}:${guildId ?? ""}`;
  const hit = cache.get(key);
  if (!fresh && hit && Date.now() - hit.at < TTL) return hit.json;
  const r = await api<ProfileJson>("GET", `/users/${userId}/profile${guildId ? `?guild_id=${guildId}` : ""}`);
  if (r.status !== 200) return undefined;
  cache.set(key, { at: Date.now(), json: r.body });
  return r.body;
}

export function invalidateProfile(userId: Id): void {
  for (const k of cache.keys()) if (k.startsWith(`${userId}:`)) cache.delete(k);
}

/** Upload an image through a signed PUT; returns the object key to save in the profile. */
export async function uploadImage(endpoint: string, file: File): Promise<string> {
  if (!["image/png", "image/jpeg", "image/webp", "image/gif"].includes(file.type)) throw new Error("Нужна картинка PNG, JPEG, WebP или GIF");
  if (file.size > 8 * 1024 * 1024) throw new Error("Картинка больше 8 МБ");
  const r = await api<{ key: string; upload_url: string; message?: string }>("POST", endpoint, { filename: file.name, content_type: file.type, size: file.size });
  if (r.status !== 200) throw new Error(r.status === 503 ? "На сервере не настроено хранилище файлов" : r.body?.message || "Не удалось начать загрузку");
  const put = await fetch(r.body.upload_url, { method: "PUT", body: file, headers: { "content-type": file.type } });
  if (!put.ok) throw new Error(`Загрузка не удалась (${put.status})`);
  return r.body.key;
}

// ---------------------------------------------------------------- open/close

export interface ProfileTarget {
  userId: Id;
  guildId?: Id;
  anchor?: Anchor;
  full?: boolean;
}

const [target, setTarget] = createSignal<ProfileTarget>();
export const profileTarget = target;

/** Show the profile popout next to `anchor` (or the full profile when `full`). */
export function openProfile(t: ProfileTarget): void {
  setTarget(t);
}

export function closeProfile(): void {
  setTarget(undefined);
}

/** Server context for profiles opened from the current screen. */
export function currentGuild(): Id | undefined {
  return state.selection.guild === "@me" ? undefined : state.selection.guild;
}
