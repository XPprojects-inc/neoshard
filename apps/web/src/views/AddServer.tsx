// «Добавить сервер»: find a public server by name, join by invite link or code,
// or create a new one. Invite links (…/invite/CODE) open this on the invite tab.

import { ChannelType } from "@neoshard/core";
import { Button, Modal, Switch, TextField, toast } from "@neoshard/ui";
import { createEffect, createResource, createSignal, For, onCleanup, Show } from "solid-js";

import { canShare, copy, select, share } from "../actions";
import { inviteCode, inviteLink } from "../instance";
import { api, asset, sid, state, type Id } from "../state";

type Tab = "find" | "invite" | "create";

interface PublicGuild {
  id: Id;
  name: string;
  icon: string;
  description: string;
  member_count: number;
  joined: boolean;
}

const members = (n: number) => {
  const m10 = n % 10, m100 = n % 100;
  const word = m10 === 1 && m100 !== 11 ? "участник" : m10 >= 2 && m10 <= 4 && (m100 < 12 || m100 > 14) ? "участника" : "участников";
  return `${n} ${word}`;
};

function GuildIcon(props: { name: string; icon: string }) {
  return (
    <span class="add-icon" aria-hidden="true">
      <Show when={asset(props.icon)} fallback={props.name.slice(0, 2)}>
        <img src={asset(props.icon)} alt="" />
      </Show>
    </span>
  );
}

export function AddServerModal(props: { open: boolean; onClose: () => void; code?: string }) {
  const [tab, setTab] = createSignal<Tab>("find");
  const [busy, setBusy] = createSignal(false);
  const done = (gid: Id) => {
    props.onClose();
    select(gid);
  };

  // ---- find by name
  const [query, setQuery] = createSignal("");
  const [debounced, setDebounced] = createSignal("");
  let timer = 0;
  createEffect(() => {
    const q = query();
    clearTimeout(timer);
    timer = window.setTimeout(() => setDebounced(q.trim()), 250);
  });
  onCleanup(() => clearTimeout(timer));
  const [found] = createResource(
    () => (props.open && tab() === "find" ? { q: debounced() } : undefined),
    async ({ q }) => {
      const r = await api<PublicGuild[]>("GET", `/discovery/guilds?q=${encodeURIComponent(q)}`);
      return r.status === 200 ? r.body : [];
    },
  );
  const joinPublic = async (g: PublicGuild) => {
    if (g.joined) return done(g.id);
    setBusy(true);
    const r = await api<{ id: string }>("POST", `/discovery/guilds/${g.id}/join`, {});
    setBusy(false);
    if (r.status === 200) {
      toast(`Вы вступили в «${g.name}»`);
      done(g.id);
    } else toast(r.status === 403 ? "Вы заблокированы на этом сервере" : "Не удалось вступить", "error");
  };

  // ---- invite link or code
  const [code, setCode] = createSignal("");
  createEffect(() => {
    if (props.open && props.code) {
      setTab("invite");
      setCode(props.code);
    }
  });
  const [preview] = createResource(
    () => (props.open && tab() === "invite" && inviteCode(code()).length >= 4 ? inviteCode(code()) : undefined),
    async (c) => {
      const r = await api<{ guild: { id: string; name: string }; member_count: number }>("GET", `/invites/${encodeURIComponent(c)}`);
      return r.status === 200 ? r.body : null;
    },
  );
  const joinInvite = async () => {
    const c = inviteCode(code());
    setBusy(true);
    const r = await api<{ id: string; name: string }>("POST", `/invites/${encodeURIComponent(c)}`, {});
    setBusy(false);
    if (r.status === 200) {
      toast(`Вы вступили в «${r.body.name}»`);
      setCode("");
      done(r.body.id);
    } else toast(r.status === 403 ? "Вы заблокированы на этом сервере" : "Приглашение недействительно или истекло", "error");
  };

  // ---- create
  const [name, setName] = createSignal("");
  const [isPublic, setPublic] = createSignal(false);
  const create = async () => {
    setBusy(true);
    const r = await api<{ id: string }>("POST", "/guilds", { name: name().trim() });
    if (r.status === 201 && isPublic()) await api("PUT", `/guilds/${r.body.id}/discovery`, { enabled: true, description: "" });
    setBusy(false);
    if (r.status === 201) {
      toast("Сервер создан");
      setName("");
      done(r.body.id);
    } else toast("Не удалось создать сервер", "error");
  };

  const tabs: [Tab, string][] = [
    ["find", "Найти"],
    ["invite", "По ссылке"],
    ["create", "Создать"],
  ];
  return (
    <Modal open={props.open} title="Добавить сервер" onClose={props.onClose}>
      <div class="add-tabs" role="tablist" aria-label="Способ">
        <For each={tabs}>
          {([id, label]) => (
            <button role="tab" aria-selected={tab() === id} onClick={() => setTab(id)}>
              {label}
            </button>
          )}
        </For>
      </div>

      <Show when={tab() === "find"}>
        <TextField label="Название сервера" value={query()} onInput={setQuery} autofocus />
        <div class="add-list" role="list" aria-busy={found.loading}>
          <For
            each={found() ?? []}
            fallback={
              <p class="edited">
                {found.loading ? "Ищем…" : debounced() ? "Ничего не нашлось. Сервер может быть закрытым — тогда нужна ссылка-приглашение." : "Публичных серверов пока нет."}
              </p>
            }
          >
            {(g) => (
              <div class="add-row" role="listitem">
                <GuildIcon name={g.name} icon={g.icon} />
                <span class="add-meta">
                  <b>{g.name}</b>
                  <small>{g.description || members(g.member_count)}</small>
                  <Show when={g.description}>
                    <small>{members(g.member_count)}</small>
                  </Show>
                </span>
                <Button size="sm" variant={g.joined ? "secondary" : "primary"} loading={busy()} onClick={() => void joinPublic(g)}>
                  {g.joined ? "Открыть" : "Вступить"}
                </Button>
              </div>
            )}
          </For>
        </div>
      </Show>

      <Show when={tab() === "invite"}>
        <TextField label="Ссылка или код приглашения" value={code()} onInput={setCode} placeholder="https://…/invite/AbC123" autofocus />
        <Show when={preview()}>
          {(p) => (
            <div class="add-row">
              <GuildIcon name={p().guild.name} icon="" />
              <span class="add-meta">
                <b>{p().guild.name}</b>
                <small>{members(p().member_count)}</small>
              </span>
            </div>
          )}
        </Show>
        <Show when={preview() === null}>
          <p class="edited">Приглашение не найдено или истекло.</p>
        </Show>
        <Button disabled={!inviteCode(code())} loading={busy()} onClick={joinInvite}>Присоединиться</Button>
      </Show>

      <Show when={tab() === "create"}>
        <TextField label="Название" value={name()} onInput={setName} autofocus />
        <Switch checked={isPublic()} onChange={setPublic} label="Публичный сервер" description="Его можно будет найти по названию и вступить без приглашения. Это можно изменить в настройках сервера." />
        <Button disabled={!name().trim()} loading={busy()} onClick={create}>Создать</Button>
      </Show>
    </Modal>
  );
}

/** «Пригласить друзей»: a link that works outside the app, copy and (on phones) share. */
export function InviteModal(props: { open: boolean; onClose: () => void }) {
  const [link, setLink] = createSignal("");
  const [error, setError] = createSignal("");
  // The open channel, or the server's first text channel when none is open (server menu).
  const channelId = () => {
    const g = state.guilds[state.selection.guild];
    const open = state.selection.channel;
    if (open && (!g || g.channels.some((c) => sid(c.id) === open && c.type !== ChannelType.VOICE))) return open;
    const first = g?.channels.filter((c) => c.type === ChannelType.TEXT).sort((a, b) => a.position - b.position)[0];
    return first ? sid(first.id) : open;
  };
  createEffect(() => {
    const cid = channelId();
    if (!props.open || !cid) return;
    setLink("");
    setError("");
    void api<{ code: string }>("POST", `/channels/${cid}/invites`, {}).then((r) => {
      if (r.status === 201) setLink(inviteLink(r.body.code));
      else setError("У вас нет права создавать приглашения на этом сервере.");
    });
  });
  const guildName = () => state.guilds[state.selection.guild]?.name ?? "NeoShard";
  return (
    <Modal open={props.open} title="Пригласить друзей" onClose={props.onClose}>
      <Show when={!error()} fallback={<p class="edited">{error()}</p>}>
        <label class="ns-field">
          <span class="ns-label">Ссылка (действует 7 дней)</span>
          {/* readonly + select on focus: long-press copy works even where the clipboard API does not */}
          <input class="ns-input invite-link" readonly value={link() || "Создаём ссылку…"} onFocus={(e) => e.currentTarget.select()} />
        </label>
        <div class="invite-actions">
          <Button disabled={!link()} onClick={() => void copy(link(), "Ссылка скопирована")}>Копировать</Button>
          <Show when={canShare()}>
            <Button variant="secondary" disabled={!link()} onClick={() => void share(link(), `Приглашение в «${guildName()}»`)}>Поделиться</Button>
          </Show>
        </div>
      </Show>
    </Modal>
  );
}
