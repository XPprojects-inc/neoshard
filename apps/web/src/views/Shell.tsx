import { ChannelType, ConnectionState, Permission, Status, hasPermission, type Channel, type Member } from "@neoshard/core";
import { Button, createContextMenu, createReorder, IconButton, Menu, Modal, Popover, TextField, toast, Tooltip, type MenuItem } from "@neoshard/ui";
import { createEffect, createMemo, createResource, createSignal, For, lazy, onCleanup, onMount, Show, Suspense } from "solid-js";

import { activeCustom, copy, ensureUsers, loadGuildMembers, openDm, roleColor, select, setMyPresence, STATUS_LABEL, statusName } from "../actions";
import { ArrowLeft, ChevronDown, ChevronRight, Copy, Hash, MessageSquarePlus, Headphones, HeadphoneOff, Lock, LogOut, Megaphone, MessageSquare, Mic, MicOff, Pencil, Phone, Pin, Plus, Settings, Smile, UserPlus, Users, Volume2 } from "../icons";
import { Markdown } from "../markdown";
import { notify } from "../notifications";
import { forwardToPlugins, startInstalledPlugins } from "../plugins/host";
import { prefs, setPref, toggleCategory } from "../prefs";
import { TagChip, UserAvatar, UserName } from "../profile/components";
import { CustomStatusModal } from "../profile/CustomStatus";
import { currentGuild, openProfile } from "../profile/data";
import { ProfileLayer } from "../profile/ProfileLayer";
import { api, asset, core, observeUpdates, setGuildOrder, setState, sid, state, type GuildView, type Id } from "../state";
import { snowflakeTime } from "../time";
import { Lightbox } from "./Attachments";
import { FriendsPage, pendingIncoming, relationshipItems } from "./Friends";
import { AddServerModal, InviteModal } from "./AddServer";
import { IncomingCall } from "./Call";
import { Messages } from "./Messages";
import { installRipples, isMobile, mobileTab, setMobileTab, type MobileTab } from "./mobile";
import { ServerSettings } from "./ServerSettings";
import { SearchBox, searchOpen, SearchPanel } from "./Search";
import { SettingsModal, type SettingsTab } from "./Settings";

const Voice = lazy(() => import("./Voice"));

export { statusName };

const unread = (cid: Id, last: bigint) => {
  const rs = state.readStates[cid];
  return last > 0n && (!rs || BigInt(rs.last) < last);
};

function guildUnread(g: GuildView) {
  return g.channels.some((c) => c.type !== ChannelType.VOICE && c.type !== ChannelType.CATEGORY && unread(sid(c.id), c.lastMessageId));
}

function guildMentions(g: GuildView) {
  return g.channels.reduce((n, c) => n + (state.readStates[sid(c.id)]?.mentions ?? 0), 0);
}

const initials = (name: string) =>
  name
    .split(/\s+/)
    .map((w) => w[0])
    .join("")
    .slice(0, 3);

/** Presence of any user; our own comes from this device. */
export function presenceOf(id: Id) {
  return id === sid(state.me?.id ?? 0n) ? state.myPresence : state.presence[id];
}

// Settings can be opened from several places (user bar, menus, profile).
const [settingsOpen, setSettingsOpen] = createSignal<SettingsTab>();
export const openSettings = (tab: SettingsTab = "account") => setSettingsOpen(tab);

const ctx = createContextMenu();

export function userMenuItems(userId: Id, name: string): MenuItem[] {
  const me = userId === sid(state.me?.id ?? 0n);
  return [
    { label: "Профиль", onSelect: () => openProfile({ userId, guildId: currentGuild(), full: true }) },
    ...(me
      ? [{ label: "Редактировать профиль", onSelect: () => openSettings("profiles") }]
      : [
          { label: "Написать", onSelect: () => void openDm(userId) },
          { label: "Зашифрованное ЛС", onSelect: () => void openDm(userId, true) },
        ]),
    ...(me ? [] : [{ separator: true }, ...relationshipItems(userId, name)]),
    ...moderationItems(userId, name),
    { separator: true },
    { label: "Копировать имя", onSelect: () => void copy(name, "Имя скопировано") },
    { label: "Копировать ID пользователя", onSelect: () => void copy(userId, "ID скопирован") },
  ];
}

/** Moderation actions on a member of the current guild, by the viewer's permissions. */
function moderationItems(userId: Id, name: string): MenuItem[] {
  const gid = currentGuild();
  const g = gid ? state.guilds[gid] : undefined;
  if (!g || userId === sid(state.me?.id ?? 0n) || userId === g.ownerId) return [];
  const p = g.ownerId === sid(state.me?.id ?? 0n) ? Permission.ADMINISTRATOR : permsForGuild(g);
  const run = async (method: string, path: string, body: unknown, ok: string) => {
    const r = await api<{ message?: string }>(method, path, body);
    if (r.status >= 200 && r.status < 300) toast(ok);
    else toast(r.body?.message ?? (r.status === 403 ? "Роль участника не ниже вашей" : "Не получилось"), "error");
  };
  const items: MenuItem[] = [];
  if (hasPermission(p, Permission.MODERATE_MEMBERS)) {
    items.push(
      { label: "Тайм-аут на 10 минут", onSelect: () => void run("PATCH", `/guilds/${g.id}/members/${userId}`, { timeout_until: Date.now() + 600_000 }, `${name}: тайм-аут 10 минут`) },
      { label: "Тайм-аут на час", onSelect: () => void run("PATCH", `/guilds/${g.id}/members/${userId}`, { timeout_until: Date.now() + 3_600_000 }, `${name}: тайм-аут на час`) },
      { label: "Снять тайм-аут", onSelect: () => void run("PATCH", `/guilds/${g.id}/members/${userId}`, { timeout_until: 0 }, "Тайм-аут снят") },
    );
  }
  if (hasPermission(p, Permission.KICK_MEMBERS))
    items.push({ label: `Выгнать ${name}`, danger: true, onSelect: () => confirm(`Выгнать ${name}?`) && void run("DELETE", `/guilds/${g.id}/members/${userId}`, undefined, `${name} исключён`) });
  if (hasPermission(p, Permission.BAN_MEMBERS))
    items.push({ label: `Забанить ${name}`, danger: true, onSelect: () => confirm(`Забанить ${name}?`) && void run("PUT", `/guilds/${g.id}/bans/${userId}`, { reason: "" }, `${name} забанен`) });
  return items.length ? [{ separator: true }, ...items] : [];
}

export const openUserMenu = (e: MouseEvent | KeyboardEvent, userId: Id, name: string) => ctx.open(e, userMenuItems(userId, name), `Действия: ${name}`);
export const openContextMenu = ctx.open;

// ---------------------------------------------------------------- guild rail

function GuildIcon(props: { g: GuildView }) {
  return (
    <Show when={asset(props.g.icon)} fallback={<span>{initials(props.g.name)}</span>}>
      <img src={asset(props.g.icon)} alt="" />
    </Show>
  );
}

function GuildRail(props: { onCreate: () => void }) {
  let rail!: HTMLDivElement;
  const bind = createReorder<Id>({
    container: () => rail,
    items: () => state.guildOrder,
    key: (id) => id,
    onReorder: (next) => setGuildOrder(next),
  });
  const dmUnread = () => Object.values(state.dms).some((c) => unread(sid(c.id), c.lastMessageId));
  return (
    <nav class="rail" aria-label="Серверы">
      <div class="guild" classList={{ unread: dmUnread() }} aria-current={state.selection.guild === "@me"}>
        <span class="pill" />
        <Tooltip label="Личные сообщения и друзья" placement="right">
          <button class="guild-btn guild-home" aria-label={`Личные сообщения${pendingIncoming() ? `, заявок в друзья: ${pendingIncoming()}` : ""}`} onClick={() => select("@me")}>
            <MessageSquare size={22} />
          </button>
        </Tooltip>
        <Show when={pendingIncoming() > 0}>
          <span class="badge" aria-hidden="true">{pendingIncoming()}</span>
        </Show>
      </div>
      <div class="rail-sep" />
      <div ref={rail} style={{ display: "contents" }} role="tree" aria-label="Список серверов (Alt+↑/↓ — переставить)">
        <For each={state.guildOrder}>
          {(id) => {
            const g = () => state.guilds[id];
            return (
              <Show when={g()}>
                <div class="guild" classList={{ unread: guildUnread(g()) }} aria-current={state.selection.guild === id} role="treeitem" ref={(el) => onCleanup(bind(el, id))}>
                  <span class="pill" />
                  <Tooltip label={g().name} placement="right">
                    <button
                      class="guild-btn"
                      aria-label={`${g().name}${guildMentions(g()) ? `, упоминаний: ${guildMentions(g())}` : ""}`}
                      onClick={() => {
                        const first = g().channels.find((c) => c.type === ChannelType.TEXT);
                        select(id, first ? sid(first.id) : undefined);
                      }}
                      onContextMenu={(e) =>
                        ctx.open(e, [
                          { label: "Отметить как прочитанное", onSelect: () => markGuildRead(g()) },
                          { label: "Пригласить людей", onSelect: () => select(id, sid(g().channels.find((c) => c.type === ChannelType.TEXT)?.id ?? 0n)) },
                          { separator: true },
                          { label: "Копировать ID сервера", onSelect: () => void copy(id, "ID скопирован") },
                        ])
                      }
                    >
                      <GuildIcon g={g()} />
                    </button>
                  </Tooltip>
                  <Show when={guildMentions(g()) > 0}>
                    <span class="badge" aria-hidden="true">{guildMentions(g())}</span>
                  </Show>
                </div>
              </Show>
            );
          }}
        </For>
      </div>
      <div class="guild guild-add">
        <Tooltip label="Добавить сервер" placement="right">
          <button class="guild-btn" aria-label="Создать или присоединиться к серверу" onClick={props.onCreate}>
            <Plus size={22} />
          </button>
        </Tooltip>
      </div>
    </nav>
  );
}

function markGuildRead(g: GuildView) {
  for (const c of g.channels) {
    if (c.lastMessageId > 0n && unread(sid(c.id), c.lastMessageId)) core().send({ case: "ackChannel", value: { channelId: c.id, messageId: c.lastMessageId } });
  }
}

// ---------------------------------------------------------------- channel sidebar

function channelIcon(t: ChannelType, size = 18) {
  switch (t) {
    case ChannelType.VOICE:
      return <Volume2 size={size} aria-hidden="true" />;
    case ChannelType.ANNOUNCEMENT:
      return <Megaphone size={size} aria-hidden="true" />;
    case ChannelType.FORUM:
      return <MessageSquare size={size} aria-hidden="true" />;
    default:
      return <Hash size={size} aria-hidden="true" />;
  }
}

function dmName(c: Channel) {
  if (c.name) return c.name;
  return c.recipientIds
    .filter((u) => u !== state.me?.id)
    .map((u) => state.users[sid(u)]?.displayName || state.users[sid(u)]?.username || "собеседник")
    .join(", ");
}

function ServerMenu(props: { g: GuildView; onInvite: () => void; onSettings: () => void }) {
  const [anchor, setAnchor] = createSignal<HTMLElement>();
  const canManage = () => {
    if (props.g.ownerId === sid(state.me?.id ?? 0n)) return true;
    const p = permsForGuild(props.g);
    return [Permission.MANAGE_GUILD, Permission.MANAGE_ROLES, Permission.MANAGE_CHANNELS, Permission.KICK_MEMBERS, Permission.BAN_MEMBERS, Permission.VIEW_AUDIT_LOG, Permission.MANAGE_EMOJIS].some((b) => hasPermission(p, b));
  };
  const items = (): MenuItem[] => [
    { label: "Пригласить людей", icon: <UserPlus size={16} />, onSelect: props.onInvite },
    ...(canManage() ? [{ label: "Настройки сервера", icon: <Settings size={16} />, onSelect: props.onSettings }] : []),
    { label: "Изменить профиль на сервере", icon: <Pencil size={16} />, onSelect: () => openSettings("profiles") },
    { label: "Отметить как прочитанное", onSelect: () => markGuildRead(props.g) },
    { separator: true },
    { label: "Копировать ID сервера", icon: <Copy size={16} />, onSelect: () => void copy(props.g.id, "ID скопирован") },
    ...(props.g.ownerId !== sid(state.me?.id ?? 0n)
      ? [
          { separator: true },
          {
            label: "Покинуть сервер",
            icon: <LogOut size={16} />,
            danger: true,
            onSelect: async () => {
              if (!confirm(`Покинуть «${props.g.name}»?`)) return;
              const r = await api("DELETE", `/users/@me/guilds/${props.g.id}`);
              if (r.status !== 204) toast("Не удалось покинуть сервер", "error");
            },
          },
        ]
      : []),
  ];
  return (
    <>
      <button class="server-head" aria-haspopup="menu" aria-expanded={!!anchor()} onClick={(e) => setAnchor(anchor() ? undefined : e.currentTarget)}>
        <span class="server-name">
          {props.g.name}
          <Show when={props.g.tag}>
            <TagChip tag={props.g.tag} badge={props.g.tagBadge} />
          </Show>
        </span>
        <ChevronDown size={18} />
      </button>
      <Menu open={!!anchor()} anchor={anchor()} placement="bottom-start" onClose={() => setAnchor(undefined)} label={`Меню сервера ${props.g.name}`} items={items()} />
    </>
  );
}

/** Guild-level permissions of the current user (from the guild's roles). */
function permsForGuild(g: GuildView): bigint {
  const ids = new Set((g.self?.roleIds ?? []).map((r) => sid(r)));
  return g.roles.filter((r) => ids.has(sid(r.id)) || sid(r.id) === g.id).reduce((p, r) => p | r.permissions, 0n);
}

function Sidebar(props: { onInvite: () => void; onVoice: (cid: Id) => void; onServerSettings: () => void; onGroupDm: () => void }) {
  const guild = () => (state.selection.guild === "@me" ? undefined : state.guilds[state.selection.guild]);
  const groups = createMemo(() => {
    const g = guild();
    if (!g) return [];
    const visible = g.channels.filter((c) => c.type !== ChannelType.THREAD);
    const cats = visible.filter((c) => c.type === ChannelType.CATEGORY);
    const loose = visible.filter((c) => c.type !== ChannelType.CATEGORY && c.parentId === 0n);
    return [{ cat: undefined, items: loose }, ...cats.map((cat) => ({ cat, items: visible.filter((c) => c.parentId === cat.id && c.type !== ChannelType.CATEGORY) }))];
  });
  const dms = createMemo(() => Object.values(state.dms).sort((a, b) => Number(b.lastMessageId - a.lastMessageId)));
  // DM partners are known by id only until their profile is loaded.
  createEffect(() => ensureUsers(dms().flatMap((c) => c.recipientIds.map((u) => sid(u)))));
  return (
    <aside class="sidebar" aria-label="Каналы">
      <div class="sidebar-head">
        <Show when={guild()} fallback={<span class="dm-head">Личные сообщения</span>}>
          <ServerMenu g={guild()!} onInvite={props.onInvite} onSettings={props.onServerSettings} />
        </Show>
      </div>
      <nav class="channels">
        <Show
          when={guild()}
          fallback={
            <>
              <a href="#/@me" class="channel friends-link" aria-current={!state.selection.channel ? "page" : undefined} onClick={(e) => (e.preventDefault(), select("@me"))}>
                <Users size={20} aria-hidden="true" />
                <span class="name">Друзья</span>
                <Show when={pendingIncoming() > 0}>
                  <span class="mentions">{pendingIncoming()}</span>
                </Show>
              </a>
              <div class="category-row static dm-section">
                Личные сообщения
                <Tooltip label="Создать групповой чат">
                  <button class="ns-icon-btn mini" aria-label="Создать групповой чат" onClick={props.onGroupDm}>
                    <Plus size={14} />
                  </button>
                </Tooltip>
              </div>
              <For each={dms()} fallback={<p class="empty">Здесь появятся ваши личные беседы. Нажмите на участника сервера, чтобы написать ему.</p>}>
                {(c) => {
                  const cid = sid(c.id);
                  const peerId = () => c.recipientIds.find((u) => u !== state.me?.id);
                  const peer = () => (peerId() !== undefined ? state.users[sid(peerId()!)] : undefined);
                  const pres = () => (peerId() !== undefined ? presenceOf(sid(peerId()!)) : undefined);
                  const custom = () => activeCustom(pres());
                  return (
                    <a
                      href={`#/@me/${cid}`}
                      class={`channel dm ${peer()?.nameplate ? `np np-${peer()!.nameplate}` : ""}`}
                      aria-current={state.selection.channel === cid ? "page" : undefined}
                      classList={{ unread: unread(cid, c.lastMessageId) }}
                      onClick={(e) => (e.preventDefault(), select("@me", cid))}
                      onContextMenu={(e) => peerId() !== undefined && openUserMenu(e, sid(peerId()!), dmName(c))}
                    >
                      <UserAvatar user={peer()} name={dmName(c)} size={32} status={c.recipientIds.length <= 2 ? statusName(pres()?.status) : undefined} />
                      <span class="dm-text">
                        <span class="name">{dmName(c)}</span>
                        <Show when={custom()}>
                          <small>
                            {custom()!.emoji} {custom()!.text}
                          </small>
                        </Show>
                      </span>
                      <Show when={c.e2ee}>
                        <span class="e2ee-badge" title="Сквозное шифрование">
                          <Lock size={14} aria-label="зашифровано" />
                        </span>
                      </Show>
                    </a>
                  );
                }}
              </For>
            </>
          }
        >
          <For each={groups()}>
            {(grp) => {
              const collapsed = () => !!grp.cat && !!prefs.collapsed[sid(grp.cat.id)];
              return (
                <>
                  <Show when={grp.cat}>
                    <button class="category-row" aria-expanded={!collapsed()} onClick={() => toggleCategory(sid(grp.cat!.id))}>
                      {collapsed() ? <ChevronRight size={12} /> : <ChevronDown size={12} />}
                      <span>{grp.cat!.name}</span>
                    </button>
                  </Show>
                  <For each={grp.items.filter((c) => !collapsed() || state.selection.channel === sid(c.id) || unread(sid(c.id), c.lastMessageId))}>
                    {(c) => {
                      const cid = sid(c.id);
                      const voice = () => guild()!.voice.filter((v) => v.channelId === c.id);
                      return (
                        <>
                          <a
                            href={`#/${state.selection.guild}/${cid}`}
                            class="channel"
                            aria-current={state.selection.channel === cid ? "page" : undefined}
                            classList={{ unread: c.type !== ChannelType.VOICE && unread(cid, c.lastMessageId) }}
                            onClick={(e) => {
                              e.preventDefault();
                              if (c.type === ChannelType.VOICE) props.onVoice(cid);
                              else select(state.selection.guild, cid);
                            }}
                            onContextMenu={(e) =>
                              ctx.open(e, [
                                { label: "Отметить как прочитанное", disabled: !unread(cid, c.lastMessageId), onSelect: () => core().send({ case: "ackChannel", value: { channelId: c.id, messageId: c.lastMessageId } }) },
                                { label: "Пригласить в канал", onSelect: props.onInvite },
                                { separator: true },
                                { label: "Копировать ссылку", onSelect: () => void copy(`${location.origin}/#/${state.selection.guild}/${cid}`, "Ссылка скопирована") },
                                { label: "Копировать ID канала", onSelect: () => void copy(cid, "ID скопирован") },
                              ])
                            }
                          >
                            {channelIcon(c.type)}
                            <span class="name">{c.name}</span>
                            <Show when={(state.readStates[cid]?.mentions ?? 0) > 0}>
                              <span class="mentions" aria-label={`упоминаний: ${state.readStates[cid].mentions}`}>{state.readStates[cid].mentions}</span>
                            </Show>
                            <span class="channel-hover">
                              <Tooltip label="Пригласить">
                                <button class="ns-icon-btn mini" aria-label="Пригласить в канал" onClick={(e) => (e.preventDefault(), e.stopPropagation(), props.onInvite())}>
                                  <UserPlus size={14} />
                                </button>
                              </Tooltip>
                            </span>
                          </a>
                          <Show when={voice().length}>
                            <div class="voice-users">
                              <For each={voice()}>
                                {(v) => (
                                  <button class="voice-user" onClick={(e) => openProfile({ userId: sid(v.userId), guildId: currentGuild(), anchor: e.currentTarget })}>
                                    <UserAvatar user={state.users[sid(v.userId)]} size={22} noDeco />
                                    <span>{state.users[sid(v.userId)]?.displayName || state.users[sid(v.userId)]?.username}</span>
                                    <Show when={v.selfMute}>
                                      <MicOff size={14} aria-label="микрофон выключен" />
                                    </Show>
                                  </button>
                                )}
                              </For>
                            </div>
                          </Show>
                        </>
                      );
                    }}
                  </For>
                </>
              );
            }}
          </For>
        </Show>
      </nav>
      <UserBar />
    </aside>
  );
}

function UserBar() {
  const [statusOpen, setStatusOpen] = createSignal<HTMLElement>();
  const [customOpen, setCustomOpen] = createSignal(false);
  const [muted, setMuted] = createSignal(false);
  const [deaf, setDeaf] = createSignal(false);
  const me = () => state.me;
  const custom = () => activeCustom(state.myPresence);
  const statuses: Array<[Status, string]> = [
    [Status.ONLINE, STATUS_LABEL.online],
    [Status.IDLE, STATUS_LABEL.idle],
    [Status.DND, STATUS_LABEL.dnd],
    [Status.INVISIBLE, STATUS_LABEL.invisible],
  ];
  return (
    <div class="userbar">
      <button class="userbar-me" aria-label="Статус и профиль" aria-haspopup="dialog" onClick={(e) => setStatusOpen(statusOpen() ? undefined : e.currentTarget)}>
        <UserAvatar user={me()} size={32} status={state.myPresence.status === Status.INVISIBLE ? "offline" : statusName(state.myPresence.status)} />
        <span class="who">
          <b>{me()?.displayName || me()?.username}</b>
          <small>{custom() ? `${custom()!.emoji} ${custom()!.text}`.trim() : state.myPresence.status === Status.INVISIBLE ? STATUS_LABEL.invisible : `@${me()?.username ?? ""}`}</small>
        </span>
      </button>
      <Tooltip label={muted() ? "Включить микрофон" : "Выключить микрофон"}>
        <IconButton label={muted() ? "Включить микрофон" : "Выключить микрофон"} aria-pressed={muted()} onClick={() => setMuted(!muted())}>
          {muted() ? <MicOff size={18} /> : <Mic size={18} />}
        </IconButton>
      </Tooltip>
      <Tooltip label={deaf() ? "Включить звук" : "Выключить звук"}>
        <IconButton label={deaf() ? "Включить звук" : "Выключить звук"} aria-pressed={deaf()} onClick={() => setDeaf(!deaf())}>
          {deaf() ? <HeadphoneOff size={18} /> : <Headphones size={18} />}
        </IconButton>
      </Tooltip>
      <Tooltip label="Настройки">
        <IconButton label="Настройки" onClick={() => openSettings("account")}>
          <Settings size={18} />
        </IconButton>
      </Tooltip>
      <Popover open={!!statusOpen()} anchor={statusOpen()} placement="top-start" onClose={() => setStatusOpen(undefined)} label="Статус" class="status-pop">
        <div class="status-card">
          <button class="status-card-head" onClick={() => (setStatusOpen(undefined), openProfile({ userId: sid(me()?.id ?? 0n), guildId: currentGuild(), full: true }))}>
            <UserAvatar user={me()} size={48} />
            <span>
              <UserName user={me()} />
              <small class="edited">@{me()?.username}</small>
            </span>
          </button>
          <button class="status-item" onClick={() => (setStatusOpen(undefined), setCustomOpen(true))}>
            <Smile size={16} />
            <span>{custom() ? `${custom()!.emoji} ${custom()!.text}`.trim() : "Задать свой статус"}</span>
          </button>
          <Show when={custom()}>
            <button class="status-item subtle" onClick={() => setMyPresence({ custom: "", emoji: "", expires: 0 })}>
              Убрать свой статус
            </button>
          </Show>
          <div class="ns-menu-sep" />
          <For each={statuses}>
            {([s, label]) => (
              <button class="status-item" aria-pressed={state.myPresence.status === s} onClick={() => (setMyPresence({ status: s }), setStatusOpen(undefined))}>
                <span class={`ns-status-dot ns-status--${s === Status.INVISIBLE ? "offline" : statusName(s)}`} aria-hidden="true" />
                <span>{label}</span>
              </button>
            )}
          </For>
          <div class="ns-menu-sep" />
          <button class="status-item" onClick={() => (setStatusOpen(undefined), openSettings("profiles"))}>
            <Pencil size={16} />
            <span>Редактировать профиль</span>
          </button>
          <button class="status-item" onClick={() => (setStatusOpen(undefined), void copy(sid(me()?.id ?? 0n), "ID скопирован"))}>
            <Copy size={16} />
            <span>Копировать ID</span>
          </button>
        </div>
      </Popover>
      <CustomStatusModal open={customOpen()} onClose={() => setCustomOpen(false)} />
    </div>
  );
}

// ---------------------------------------------------------------- members

function Members() {
  const gid = () => state.selection.guild;
  createEffect(() => {
    const g = gid();
    // Lazy member list over the gateway: first window of 100.
    if (g !== "@me" && !state.members[g]) core().send({ case: "members", value: { guildId: BigInt(g), start: 0, end: 100 } });
  });
  const list = () => (gid() === "@me" ? [] : (state.members[gid()] ?? []).filter(Boolean));
  const status = (m: Member) => statusName(presenceOf(sid(m.user?.id ?? 0n))?.status);
  // Hoisted roles get their own group (highest first), then online, then offline.
  const sections = createMemo(() => {
    const g = state.guilds[gid()];
    const hoisted = (g?.roles ?? []).filter((r) => r.hoist && sid(r.id) !== gid()).sort((a, b) => b.position - a.position);
    const left = new Set(list());
    const out: Array<{ title: string; members: Member[] }> = [];
    for (const r of hoisted) {
      const ms = [...left].filter((m) => status(m) !== "offline" && m.roleIds.some((x) => x === r.id));
      ms.forEach((m) => left.delete(m));
      if (ms.length) out.push({ title: r.name, members: ms });
    }
    const online = [...left].filter((m) => status(m) !== "offline");
    const offline = [...left].filter((m) => status(m) === "offline");
    if (online.length) out.push({ title: "В сети", members: online });
    if (offline.length) out.push({ title: "Не в сети", members: offline });
    return out;
  });
  const row = (m: Member) => {
    const id = sid(m.user?.id ?? 0n);
    // The live user record (UserUpdate) wins over the copy in the member list.
    const u = () => state.users[id] ?? m.user;
    const name = () => m.nick || u()?.displayName || u()?.username || "?";
    const custom = () => activeCustom(presenceOf(id));
    return (
      <button
        class={`member ${u()?.nameplate ? `np np-${u()!.nameplate}` : ""}`}
        classList={{ offline: status(m) === "offline" }}
        onClick={(e) => openProfile({ userId: id, guildId: gid(), anchor: e.currentTarget })}
        onContextMenu={(e) => openUserMenu(e, id, name())}
      >
        <UserAvatar user={u()} memberAvatar={m.avatar} name={name()} size={32} status={status(m)} />
        <span class="member-name">
          <span class="member-line">
            <UserName user={u()} text={name()} color={roleColor(gid(), m.roleIds)} />
            <Show when={u()?.bot}>
              <span class="tag-chip">БОТ</span>
            </Show>
          </span>
          <Show when={custom()}>
            <small>
              {custom()!.emoji} {custom()!.text}
            </small>
          </Show>
        </span>
      </button>
    );
  };
  return (
    <aside class="members" aria-label="Участники">
      <For each={sections()}>
        {(s) => (
          <>
            <h3>
              {s.title} — {s.members.length}
            </h3>
            <For each={s.members}>{row}</For>
          </>
        )}
      </For>
    </aside>
  );
}

// ---------------------------------------------------------------- pins

function PinsButton(props: { channelId: Id }) {
  const [anchor, setAnchor] = createSignal<HTMLElement>();
  const [pins] = createResource(
    () => (anchor() ? props.channelId : undefined),
    async (cid) => (await api<Array<{ id: string; content: string; author?: { id: string; username: string; display_name?: string } }>>("GET", `/channels/${cid}/pins`)).body ?? [],
  );
  return (
    <>
      <Tooltip label="Закреплённые сообщения" placement="bottom">
        <IconButton label="Закреплённые сообщения" aria-expanded={!!anchor()} onClick={(e) => setAnchor(anchor() ? undefined : e.currentTarget)}>
          <Pin size={18} />
        </IconButton>
      </Tooltip>
      <Popover open={!!anchor()} anchor={anchor()} placement="bottom-end" onClose={() => setAnchor(undefined)} label="Закреплённые сообщения" class="pins-pop">
        <div class="pins">
          <h3>Закреплённые сообщения</h3>
          <Show when={!pins.loading} fallback={<p class="edited">Загрузка…</p>}>
            <For each={pins()} fallback={<p class="edited">В этом канале пока ничего не закрепили.</p>}>
              {(p) => (
                <div class="pin">
                  <b>{p.author?.display_name || p.author?.username}</b>
                  <small class="edited"> · {new Date(snowflakeTime(BigInt(p.id))).toLocaleDateString("ru-RU")}</small>
                  <div class="pin-body">
                    <Markdown source={p.content} resolver={{ user: (id) => state.users[id]?.username, role: () => undefined, isMe: () => false }} />
                  </div>
                </div>
              )}
            </For>
          </Show>
        </div>
      </Popover>
    </>
  );
}

// ---------------------------------------------------------------- modals

/** New group DM (up to 9 other people you share a server with). */
function GroupDmModal(props: { open: boolean; onClose: () => void }) {
  const [picked, setPicked] = createSignal<Id[]>([]);
  const [name, setName] = createSignal("");
  const [filter, setFilter] = createSignal("");
  const [e2ee, setE2ee] = createSignal(false);
  const me = () => sid(state.me?.id ?? 0n);
  const people = () =>
    Object.values(state.users)
      .filter((u) => sid(u.id) !== me() && !u.bot && !u.username.startsWith("deleted_"))
      .filter((u) => !filter() || `${u.username} ${u.displayName}`.toLowerCase().includes(filter().toLowerCase()))
      .slice(0, 100);
  createEffect(() => props.open && void loadGuildMembers());
  const toggle = (id: Id) => setPicked(picked().includes(id) ? picked().filter((x) => x !== id) : picked().length < 9 ? [...picked(), id] : picked());
  const create = async () => {
    if (e2ee() && !core().e2ee) return toast("Включите шифрование ЛС в настройках этого устройства", "error");
    const r = await api<{ id: string; message?: string }>("POST", "/users/@me/channels", { recipient_ids: picked(), name: name().trim(), e2ee: e2ee() });
    if (r.status === 200 || r.status === 201) {
      select("@me", r.body.id);
      setPicked([]);
      setName("");
      props.onClose();
    } else toast(r.body?.message ?? "Не удалось создать беседу (писать можно тем, с кем есть общий сервер)", "error");
  };
  return (
    <Modal open={props.open} title="Групповой чат" onClose={props.onClose}>
      <p class="pe-hint" style={{ margin: 0 }}>Выберите до 9 человек. Выбрано: {picked().length}</p>
      <TextField label="Найти" value={filter()} onInput={setFilter} />
      <div class="gdm-list" role="group" aria-label="Люди">
        <For each={people()} fallback={<p class="edited">Здесь появятся люди с ваших серверов.</p>}>
          {(u) => (
            <label class="gdm-row">
              <input type="checkbox" checked={picked().includes(sid(u.id))} onChange={() => toggle(sid(u.id))} />
              <UserAvatar user={u} size={28} noDeco />
              <span>
                {u.displayName || u.username} <small class="edited">@{u.username}</small>
              </span>
            </label>
          )}
        </For>
      </div>
      <Show when={picked().length > 1}>
        <TextField label="Название (необязательно)" value={name()} onInput={setName} />
      </Show>
      <label class="gdm-row">
        <input type="checkbox" checked={e2ee()} onChange={(e) => setE2ee(e.currentTarget.checked)} />
        <span>Сквозное шифрование</span>
      </label>
      <Button disabled={!picked().length} onClick={create}>
        {picked().length > 1 ? "Создать групповой чат" : "Открыть беседу"}
      </Button>
    </Modal>
  );
}

// ---------------------------------------------------------------- shell

export default function Shell() {
  onMount(() => {
    startInstalledPlugins();
    observeUpdates((u) => {
      forwardToPlugins(u);
      notify(u);
    });
  });
  const [createOpen, setCreateOpen] = createSignal(false);
  // Opened from an invite link: https://server/invite/CODE → /app/?invite=CODE.
  const [linkCode, setLinkCode] = createSignal<string>();
  onMount(() => {
    const code = new URLSearchParams(location.search).get("invite");
    if (!code) return;
    setLinkCode(code);
    setCreateOpen(true);
    history.replaceState(null, "", location.pathname);
  });
  const [inviteOpen, setInviteOpen] = createSignal(false);
  const [serverSettings, setServerSettings] = createSignal(false);
  const [groupDm, setGroupDm] = createSignal(false);
  const [voiceChannel, setVoiceChannel] = createSignal<Id>();
  // The guild the call belongs to ("@me" for DMs), fixed when joining: browsing elsewhere keeps the call.
  const [voiceGuild, setVoiceGuild] = createSignal<Id | "@me">("@me");
  const joinVoice = (cid: Id, gid: Id | "@me") => {
    setVoiceGuild(gid);
    setVoiceChannel(cid);
  };
  const channel = () => {
    const cid = state.selection.channel;
    if (!cid) return undefined;
    if (state.selection.guild === "@me") return state.dms[cid];
    return state.guilds[state.selection.guild]?.channels.find((c) => sid(c.id) === cid);
  };
  const perms = () => state.perms[state.selection.channel ?? ""] ?? 0n;
  const showMembers = () => state.selection.guild !== "@me" && prefs.membersVisible;
  const rightPanel = () => (isMobile() ? (mobileMembers() ? "members" : undefined) : searchOpen() ? "search" : showMembers() ? "members" : undefined);
  // Phones: one screen at a time — lists, a chat, friends or "you".
  const [mobileMembers, setMobileMembers] = createSignal(false);
  const screen = () => (!isMobile() ? "desktop" : channel() || voiceChannel() ? "chat" : mobileTab());
  const back = () => {
    setVoiceChannel(undefined);
    setMobileMembers(false);
    setState("selection", "channel", undefined);
  };
  let shellEl!: HTMLDivElement;
  onMount(() => installRipples(shellEl));
  const banner = () => {
    const s = state.connection.state;
    if (s === ConnectionState.READY || s === ConnectionState.IDENTIFYING || s === ConnectionState.CONNECTING) return "";
    if (s === ConnectionState.RESUMING) return "Восстанавливаем соединение…";
    if (s === ConnectionState.WAITING_RECONNECT) return state.connection.retryInMs > 0 ? `Нет соединения — повтор через ${Math.ceil(state.connection.retryInMs / 1000)} с` : "Нет соединения — переподключаемся";
    if (s === ConnectionState.AUTH_FAILED) return "Сессия истекла — войдите снова";
    if (s === ConnectionState.DISCONNECTED && state.connection.reason) {
      return state.connection.reason === "too many sessions"
        ? "Открыто слишком много вкладок и устройств с этим аккаунтом. Закройте лишние и обновите страницу."
        : `Сервер закрыл соединение: ${state.connection.reason}`;
    }
    return "";
  };
  return (
    <div ref={shellEl} class="shell" classList={{ "no-members": !rightPanel(), "wide-right": rightPanel() === "search", m3: isMobile() }} data-screen={screen()}>
      <div class="nav-drawer">
        <GuildRail onCreate={() => setCreateOpen(true)} />
        <Sidebar onInvite={() => setInviteOpen(true)} onVoice={(cid) => joinVoice(cid, state.selection.guild)} onServerSettings={() => setServerSettings(true)} onGroupDm={() => setGroupDm(true)} />
      </div>
      <main class="main">
        <Show when={screen() === "friends"}>
          <FriendsPage />
        </Show>
        <Show when={screen() === "you"}>
          <MobileYou onSettings={(t) => openSettings(t)} />
        </Show>
        <Show when={banner()}>
          <div class="conn-banner" role="status">
            {banner()}
            <Show when={state.connection.state === ConnectionState.DISCONNECTED}>
              {" "}
              <button class="conn-retry" onClick={() => location.reload()}>Обновить</button>
            </Show>
          </div>
        </Show>
        <Show
          when={voiceChannel()}
          fallback={
            <Show when={screen() === "chat" || screen() === "desktop" ? channel() : undefined} keyed fallback={screen() === "desktop" ? (state.selection.guild === "@me" ? <FriendsPage /> : <Welcome />) : null}>
              {(ch) => {
                const c = () => channel() ?? ch;
                const peer = () => (c().guildId === 0n && c().recipientIds.length <= 2 ? c().recipientIds.find((u) => u !== state.me?.id) : undefined);
                return (
                  <>
                    <header class="topbar">
                      <Show when={isMobile()}>
                        <IconButton label="Назад" class="m3-back" onClick={back}>
                          <ArrowLeft size={22} />
                        </IconButton>
                      </Show>
                      <Show when={peer() !== undefined} fallback={channelIcon(c().type, 20)}>
                        <UserAvatar user={state.users[sid(peer()!)]} size={24} noDeco />
                      </Show>
                      <h1>
                        <Show when={peer() !== undefined} fallback={c().name || "Личные сообщения"}>
                          <button class="topbar-user" onClick={(e) => openProfile({ userId: sid(peer()!), anchor: e.currentTarget })}>
                            {state.users[sid(peer()!)]?.displayName || state.users[sid(peer()!)]?.username}
                          </button>
                        </Show>
                      </h1>
                      <Show when={c().e2ee}>
                        <span class="e2ee-badge" title="Сквозное шифрование">
                          <Lock size={14} aria-label="зашифровано" />
                        </span>
                      </Show>
                      <Show when={c().topic}>
                        <span class="topbar-sep" aria-hidden="true" />
                        <span class="topic" title={c().topic}>{c().topic}</span>
                      </Show>
                      <span class="topbar-spacer" />
                      <Show when={!c().e2ee}>
                        <SearchBox />
                      </Show>
                      <Show when={c().guildId === 0n}>
                        <Tooltip label={state.dmVoice[sid(c().id)]?.length ? "Присоединиться к звонку" : "Позвонить"} placement="bottom">
                          <IconButton label={state.dmVoice[sid(c().id)]?.length ? "Присоединиться к звонку" : "Позвонить"} class="call-start" classList={{ "call-live": !!state.dmVoice[sid(c().id)]?.length }} onClick={() => joinVoice(sid(c().id), "@me")}>
                            <Phone size={18} />
                          </IconButton>
                        </Tooltip>
                      </Show>
                      <Show when={c().guildId !== 0n}>
                        <PinsButton channelId={sid(c().id)} />
                        <Tooltip label={prefs.membersVisible ? "Скрыть участников" : "Показать участников"} placement="bottom">
                          <IconButton label="Список участников" aria-pressed={isMobile() ? mobileMembers() : prefs.membersVisible} class="toggle" onClick={() => (isMobile() ? setMobileMembers(!mobileMembers()) : setPref("membersVisible", !prefs.membersVisible))}>
                            <Users size={18} />
                          </IconButton>
                        </Tooltip>
                      </Show>
                    </header>
                    <Messages channel={c()} canSend={hasPermission(perms(), Permission.SEND_MESSAGES) || c().guildId === 0n} canManage={hasPermission(perms(), Permission.MANAGE_MESSAGES)} canAttach={hasPermission(perms(), Permission.ATTACH_FILES) || c().guildId === 0n} />
                  </>
                );
              }}
            </Show>
          }
        >
          <Suspense fallback={<div class="empty">Подключаем голос…</div>}>
            <Voice channelId={voiceChannel()!} guildId={voiceGuild()} onLeave={() => setVoiceChannel(undefined)} />
          </Suspense>
        </Show>
      </main>
      <Show when={rightPanel() === "members"}>
        <Show when={isMobile()}>
          <div class="m3-scrim" onClick={() => setMobileMembers(false)} />
        </Show>
        <Members />
      </Show>
      <Show when={isMobile() && screen() !== "chat"}>
        <MobileNav />
        <Show when={screen() === "chats"}>
          <button class="m3-fab" aria-label="Новая беседа" onClick={() => setGroupDm(true)}>
            <MessageSquarePlus size={24} />
          </button>
        </Show>
      </Show>
      <Show when={rightPanel() === "search"}>
        <SearchPanel />
      </Show>
      <IncomingCall
        activeCall={voiceChannel()}
        onAccept={(cid) => {
          select("@me", cid);
          joinVoice(cid, "@me");
        }}
      />
      <AddServerModal open={createOpen()} code={linkCode()} onClose={() => (setCreateOpen(false), setLinkCode(undefined))} />
      <InviteModal open={inviteOpen()} onClose={() => setInviteOpen(false)} />
      <Show when={serverSettings() && state.selection.guild !== "@me"}>
        <ServerSettings guildId={state.selection.guild} onClose={() => setServerSettings(false)} />
      </Show>
      <SettingsModal open={!!settingsOpen()} initial={settingsOpen()} onClose={() => setSettingsOpen(undefined)} />
      <GroupDmModal open={groupDm()} onClose={() => setGroupDm(false)} />
      <ProfileLayer />
      <Lightbox />
      {ctx.view()}
    </div>
  );
}

/** Material navigation bar (phones). */
function MobileNav() {
  const items: Array<[MobileTab, string, () => any]> = [
    ["chats", "Чаты", () => <MessageSquare size={22} />],
    ["friends", "Друзья", () => <Users size={22} />],
    ["you", "Вы", () => <UserAvatar user={state.me} size={24} noDeco />],
  ];
  return (
    <nav class="m3-navbar" aria-label="Разделы">
      <For each={items}>
        {([id, label, icon]) => (
          <button class="m3-nav-item" aria-current={mobileTab() === id ? "page" : undefined} onClick={() => setMobileTab(id)}>
            <span class="m3-nav-indicator">
              {icon()}
              <Show when={id === "friends" && pendingIncoming() > 0}>
                <span class="m3-nav-badge">{pendingIncoming()}</span>
              </Show>
            </span>
            <span class="m3-nav-label">{label}</span>
          </button>
        )}
      </For>
    </nav>
  );
}

/** "You" tab on phones: profile, status and settings entry points. */
function MobileYou(props: { onSettings: (t: SettingsTab) => void }) {
  const me = () => state.me;
  const custom = () => activeCustom(state.myPresence);
  const rows: Array<[SettingsTab, string]> = [
    ["account", "Учётная запись"],
    ["profiles", "Профили"],
    ["security", "Безопасность"],
    ["privacy", "Шифрование"],
    ["appearance", "Внешний вид"],
    ["voice", "Голос и видео"],
    ["notifications", "Уведомления"],
  ];
  return (
    <section class="m3-you" aria-label="Вы">
      <button class="m3-you-card m3-tap" onClick={() => openProfile({ userId: sid(me()?.id ?? 0n), full: true })}>
        <UserAvatar user={me()} size={72} status={statusName(state.myPresence.status)} />
        <span>
          <UserName user={me()} />
          <small class="edited">@{me()?.username}</small>
          <Show when={custom()}>
            <small>
              {custom()!.emoji} {custom()!.text}
            </small>
          </Show>
        </span>
      </button>
      <div class="m3-list">
        <For each={rows}>
          {([t, label]) => (
            <button class="m3-list-item" onClick={() => props.onSettings(t)}>
              <span>{label}</span>
              <ChevronRight size={18} />
            </button>
          )}
        </For>
      </div>
    </section>
  );
}

function Welcome() {
  return (
    <div class="welcome">
      <div class="welcome-art" aria-hidden="true">
        <svg viewBox="0 0 120 120" width="120" height="120">
          <path d="M60 8l38 32-16 72H38L22 40z" fill="color-mix(in srgb, var(--ns-accent) 25%, transparent)" stroke="var(--ns-accent)" stroke-width="3" stroke-linejoin="round" />
          <path d="M60 8L50 60l10 52M22 40l28 20 48-20" fill="none" stroke="var(--ns-accent)" stroke-width="2" opacity=".6" />
        </svg>
      </div>
      <h2>{state.selection.guild === "@me" ? "Выберите беседу" : "Выберите канал"}</h2>
      <p class="edited">{state.selection.guild === "@me" ? "Личные сообщения появятся слева. Чтобы написать кому-то, нажмите на участника сервера." : "Каналы этого сервера — слева."}</p>
    </div>
  );
}
