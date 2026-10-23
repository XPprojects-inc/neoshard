// Friends home: online / all / pending / blocked lists and "add friend" by username.

import { RelationshipKind } from "@neoshard/core";
import { Button, IconButton, toast, Tooltip, type MenuItem } from "@neoshard/ui";
import { createMemo, createSignal, For, Show } from "solid-js";

import { activeCustom, openDm, STATUS_LABEL, statusName } from "../actions";
import { Check, Ellipsis, MessageSquare, Users, X } from "../icons";
import { UserAvatar, UserName } from "../profile/components";
import { openProfile } from "../profile/data";
import { api, sid, state, type Id } from "../state";
import { openContextMenu } from "./Shell";

type Tab = "online" | "all" | "pending" | "blocked" | "add";

const errorText: Record<number, string> = {
  404: "Такого пользователя нет. Проверьте имя пользователя — это не отображаемое имя.",
  409: "Вы уже друзья или заявка уже отправлена.",
  429: "Слишком много заявок. Попробуйте позже.",
};

/** Friend request by username; returns true when it went through. */
export async function addFriend(username: string): Promise<boolean> {
  const r = await api<{ message?: string }>("POST", "/users/@me/relationships", { username: username.trim() });
  if (r.status === 201) {
    toast(`Заявка отправлена: @${username.trim().replace(/^@/, "")}`);
    return true;
  }
  if (r.status === 200) {
    toast("Теперь вы друзья");
    return true;
  }
  toast(errorText[r.status] ?? "Не получилось отправить заявку", "error");
  return false;
}

export const acceptFriend = (id: Id) => api("PUT", `/users/@me/relationships/${id}`, { type: RelationshipKind.FRIEND });
export const removeRelationship = (id: Id) => api("DELETE", `/users/@me/relationships/${id}`);
export const blockUser = (id: Id) => api("PUT", `/users/@me/relationships/${id}`, { type: RelationshipKind.BLOCKED });

/** Menu items about the relationship with a user (for profile cards and menus). */
export function relationshipItems(id: Id, name: string): MenuItem[] {
  const k = state.relationships[id]?.kind;
  const items: MenuItem[] = [];
  if (k === undefined || k === RelationshipKind.NONE) items.push({ label: "Добавить в друзья", onSelect: () => void addFriend(state.users[id]?.username ?? "") });
  if (k === RelationshipKind.INCOMING) items.push({ label: "Принять заявку в друзья", onSelect: () => void acceptFriend(id) });
  if (k === RelationshipKind.OUTGOING) items.push({ label: "Отменить заявку", onSelect: () => void removeRelationship(id) });
  if (k === RelationshipKind.FRIEND) items.push({ label: "Удалить из друзей", danger: true, onSelect: () => confirm(`Удалить ${name} из друзей?`) && void removeRelationship(id) });
  if (k === RelationshipKind.BLOCKED) items.push({ label: "Разблокировать", onSelect: () => void removeRelationship(id) });
  else items.push({ label: "Заблокировать", danger: true, onSelect: () => confirm(`Заблокировать ${name}? Вы не будете получать от него сообщения и заявки.`) && void blockUser(id) });
  return items;
}

export const pendingIncoming = () => Object.values(state.relationships).filter((r) => r.kind === RelationshipKind.INCOMING).length;

export function FriendsPage() {
  const [tab, setTab] = createSignal<Tab>("online");
  const [filter, setFilter] = createSignal("");
  const rows = createMemo(() =>
    Object.entries(state.relationships)
      .map(([id, r]) => ({ id, ...r, user: state.users[id] }))
      .filter((r) => r.user)
      .sort((a, b) => (a.user!.displayName || a.user!.username).localeCompare(b.user!.displayName || b.user!.username, "ru")),
  );
  const status = (id: Id) => statusName(state.presence[id]?.status);
  const list = () => {
    const f = filter().toLowerCase();
    const byTab = rows().filter((r) => {
      switch (tab()) {
        case "online":
          return r.kind === RelationshipKind.FRIEND && status(r.id) !== "offline";
        case "all":
          return r.kind === RelationshipKind.FRIEND;
        case "pending":
          return r.kind === RelationshipKind.INCOMING || r.kind === RelationshipKind.OUTGOING;
        case "blocked":
          return r.kind === RelationshipKind.BLOCKED;
        default:
          return false;
      }
    });
    return f ? byTab.filter((r) => `${r.user!.username} ${r.user!.displayName}`.toLowerCase().includes(f)) : byTab;
  };
  const title = () => ({ online: "В сети", all: "Все друзья", pending: "Ожидание", blocked: "Заблокированные", add: "" })[tab()];
  return (
    <section class="friends" aria-label="Друзья">
      <header class="topbar friends-bar">
        <Users size={20} aria-hidden="true" />
        <h1>Друзья</h1>
        <span class="topbar-sep" aria-hidden="true" />
        <nav class="friends-tabs" role="tablist" aria-label="Списки друзей">
          <For each={[["online", "В сети"], ["all", "Все"], ["pending", "Ожидание"], ["blocked", "Заблокированные"]] as Array<[Tab, string]>}>
            {([id, label]) => (
              <button role="tab" aria-selected={tab() === id} onClick={() => setTab(id)}>
                {label}
                <Show when={id === "pending" && pendingIncoming() > 0}>
                  <span class="friends-badge" aria-label={`входящих заявок: ${pendingIncoming()}`}>{pendingIncoming()}</span>
                </Show>
              </button>
            )}
          </For>
          <button role="tab" class="friends-add-tab" aria-selected={tab() === "add"} onClick={() => setTab("add")}>
            Добавить в друзья
          </button>
        </nav>
      </header>
      <Show when={tab() !== "add"} fallback={<AddFriend />}>
        <div class="friends-body">
          <div class="friends-search">
            <input placeholder="Поиск" aria-label="Поиск по друзьям" value={filter()} onInput={(e) => setFilter(e.currentTarget.value)} />
          </div>
          <h2 class="friends-count">
            {title()} — {list().length}
          </h2>
          <For each={list()} fallback={<EmptyFriends tab={tab()} onAdd={() => setTab("add")} />}>
            {(r) => {
              const name = () => r.user!.displayName || r.user!.username;
              const custom = () => activeCustom(state.presence[r.id]);
              return (
                <div
                  class="friend-row"
                  tabindex="0"
                  onClick={(e) => r.kind === RelationshipKind.FRIEND && openProfile({ userId: r.id, anchor: e.currentTarget })}
                  onContextMenu={(e) => openContextMenu(e, relationshipItems(r.id, name()), `Действия: ${name()}`)}
                >
                  <UserAvatar user={r.user} size={36} status={r.kind === RelationshipKind.FRIEND ? status(r.id) : undefined} />
                  <span class="friend-text">
                    <span class="friend-name">
                      <UserName user={r.user} />
                      <small class="edited"> @{r.user!.username}</small>
                    </span>
                    <small class="edited">
                      {r.kind === RelationshipKind.INCOMING
                        ? "Входящая заявка в друзья"
                        : r.kind === RelationshipKind.OUTGOING
                          ? "Исходящая заявка в друзья"
                          : r.kind === RelationshipKind.BLOCKED
                            ? "Заблокирован"
                            : custom()
                              ? `${custom()!.emoji} ${custom()!.text}`.trim()
                              : STATUS_LABEL[status(r.id)]}
                    </small>
                  </span>
                  <span class="friend-actions" onClick={(e) => e.stopPropagation()}>
                    <Show when={r.kind === RelationshipKind.FRIEND}>
                      <Tooltip label="Написать">
                        <IconButton label={`Написать ${name()}`} class="round" onClick={() => void openDm(r.id)}>
                          <MessageSquare size={18} />
                        </IconButton>
                      </Tooltip>
                    </Show>
                    <Show when={r.kind === RelationshipKind.INCOMING}>
                      <Tooltip label="Принять">
                        <IconButton label={`Принять заявку от ${name()}`} class="round accept" onClick={() => void acceptFriend(r.id)}>
                          <Check size={18} />
                        </IconButton>
                      </Tooltip>
                    </Show>
                    <Show when={r.kind !== RelationshipKind.FRIEND}>
                      <Tooltip label={r.kind === RelationshipKind.BLOCKED ? "Разблокировать" : r.kind === RelationshipKind.INCOMING ? "Отклонить" : "Отменить"}>
                        <IconButton label={r.kind === RelationshipKind.BLOCKED ? `Разблокировать ${name()}` : `Отклонить ${name()}`} class="round decline" onClick={() => void removeRelationship(r.id)}>
                          <X size={18} />
                        </IconButton>
                      </Tooltip>
                    </Show>
                    <Show when={r.kind === RelationshipKind.FRIEND}>
                      <Tooltip label="Ещё">
                        <IconButton label="Ещё" class="round" onClick={(e) => openContextMenu(e, relationshipItems(r.id, name()), `Действия: ${name()}`)}>
                          <Ellipsis size={18} />
                        </IconButton>
                      </Tooltip>
                    </Show>
                  </span>
                </div>
              );
            }}
          </For>
        </div>
      </Show>
    </section>
  );
}

function AddFriend() {
  const [name, setName] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const send = async () => {
    if (!name().trim()) return;
    setBusy(true);
    if (await addFriend(name())) setName("");
    setBusy(false);
  };
  return (
    <div class="friends-body">
      <h2 class="add-title">Добавить в друзья</h2>
      <p class="edited">Добавляйте друзей по имени пользователя — общий сервер не нужен.</p>
      <form
        class="add-friend"
        onSubmit={(e) => {
          e.preventDefault();
          void send();
        }}
      >
        <input placeholder="Имя пользователя, например alice" aria-label="Имя пользователя" autocomplete="off" value={name()} onInput={(e) => setName(e.currentTarget.value)} />
        <Button type="submit" size="sm" loading={busy()} disabled={!name().trim()}>
          Отправить заявку
        </Button>
      </form>
      <div class="add-art" aria-hidden="true">
        <svg viewBox="0 0 220 140" width="220" height="140">
          <circle cx="80" cy="70" r="34" fill="var(--ns-surface-2)" />
          <circle cx="140" cy="70" r="34" fill="color-mix(in srgb, var(--ns-accent) 30%, var(--ns-surface-2))" />
          <path d="M110 48v44M88 70h44" stroke="var(--ns-accent)" stroke-width="6" stroke-linecap="round" />
        </svg>
      </div>
    </div>
  );
}

function EmptyFriends(props: { tab: Tab; onAdd: () => void }) {
  const text = () =>
    ({
      online: "Сейчас никого из друзей нет в сети.",
      all: "Пока здесь пусто. Добавьте друзей по имени пользователя.",
      pending: "Нет заявок в друзья.",
      blocked: "Вы никого не блокировали.",
      add: "",
    })[props.tab];
  return (
    <div class="friends-empty">
      <p class="edited">{text()}</p>
      <Show when={props.tab === "all" || props.tab === "online"}>
        <Button size="sm" onClick={props.onAdd}>Добавить в друзья</Button>
      </Show>
    </div>
  );
}

