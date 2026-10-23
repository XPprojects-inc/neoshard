// Full-screen server settings. Everything goes through the REST API; the
// server checks permissions (the UI only hides what the user cannot do).

import { ChannelType, hasPermission, Permission, type Channel, type Role } from "@neoshard/core";
import { Button, ColorField, Modal, Switch, TextArea, TextField, toast } from "@neoshard/ui";
import { createEffect, createMemo, createResource, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { Portal } from "solid-js/web";

import { copy, ensureUsers } from "../actions";
import { inviteLink } from "../instance";
import { ChevronDown, Hash, ImagePlus, Megaphone, MessageSquare, Trash2, Volume2, X } from "../icons";
import { TagChip } from "../profile/components";
import { hex, TagBadge, TAG_BADGES } from "../profile/cosmetics";
import { uploadImage } from "../profile/data";
import { api, asset, sid, state, type GuildView, type Id } from "../state";
import { snowflakeTime } from "../time";

type Tab = "overview" | "roles" | "channels" | "members" | "emojis" | "integrations" | "invites" | "bans" | "audit" | "automod";

/** Permission groups shown in role and channel editors (labels are our own wording). */
export const PERMISSION_GROUPS: Array<{ title: string; items: Array<[keyof typeof Permission, string, string?]> }> = [
  {
    title: "Общие",
    items: [
      ["VIEW_CHANNEL", "Просматривать каналы"],
      ["MANAGE_CHANNELS", "Управлять каналами"],
      ["MANAGE_ROLES", "Управлять ролями", "Только ролями ниже своей высшей роли"],
      ["MANAGE_EMOJIS", "Управлять эмодзи и стикерами"],
      ["VIEW_AUDIT_LOG", "Просматривать журнал аудита"],
      ["MANAGE_WEBHOOKS", "Управлять вебхуками"],
      ["MANAGE_GUILD", "Управлять сервером", "Название, значок, тег, автомодерация"],
    ],
  },
  {
    title: "Участники",
    items: [
      ["CREATE_INVITE", "Создавать приглашения"],
      ["CHANGE_NICKNAME", "Менять свой никнейм"],
      ["MANAGE_NICKNAMES", "Менять чужие никнеймы"],
      ["KICK_MEMBERS", "Выгонять участников"],
      ["BAN_MEMBERS", "Банить участников"],
      ["MODERATE_MEMBERS", "Отправлять участников подумать", "Временный запрет писать и говорить"],
    ],
  },
  {
    title: "Текстовые каналы",
    items: [
      ["SEND_MESSAGES", "Отправлять сообщения"],
      ["SEND_IN_THREADS", "Писать в ветках"],
      ["CREATE_THREADS", "Создавать ветки"],
      ["EMBED_LINKS", "Встраивать ссылки"],
      ["ATTACH_FILES", "Прикреплять файлы"],
      ["ADD_REACTIONS", "Добавлять реакции"],
      ["USE_EXTERNAL_EMOJI", "Эмодзи с других серверов"],
      ["MENTION_EVERYONE", "Упоминать @everyone и все роли"],
      ["MANAGE_MESSAGES", "Управлять сообщениями", "Удалять чужие, закреплять"],
      ["MANAGE_THREADS", "Управлять ветками"],
      ["READ_MESSAGE_HISTORY", "Читать историю сообщений"],
      ["USE_COMMANDS", "Использовать команды приложений"],
    ],
  },
  {
    title: "Голосовые каналы",
    items: [
      ["CONNECT", "Подключаться"],
      ["SPEAK", "Говорить"],
      ["VIDEO", "Включать камеру"],
      ["STREAM", "Демонстрировать экран"],
      ["PRIORITY_SPEAKER", "Приоритетный режим"],
      ["MUTE_MEMBERS", "Отключать микрофон участникам"],
      ["DEAFEN_MEMBERS", "Отключать звук участникам"],
      ["MOVE_MEMBERS", "Перемещать участников"],
    ],
  },
  {
    title: "Особые",
    items: [["ADMINISTRATOR", "Администратор", "Все права и доступ ко всем каналам. Выдавайте осторожно."]],
  },
];

const ROLE_COLORS = [0x2ec4a6, 0x3ddc84, 0x6cb6ff, 0x9d7bff, 0xff6fa8, 0xff6b6b, 0xffb03a, 0xf2c94c, 0x95a5b4, 0x1f8a9e];

function myGuildPerms(g: GuildView): bigint {
  if (g.ownerId === sid(state.me?.id ?? 0n)) return Permission.ADMINISTRATOR;
  const ids = new Set((g.self?.roleIds ?? []).map((r) => sid(r)));
  return g.roles.filter((r) => ids.has(sid(r.id)) || sid(r.id) === g.id).reduce((p, r) => p | r.permissions, 0n);
}

export function ServerSettings(props: { guildId: Id; onClose: () => void }) {
  const g = () => state.guilds[props.guildId];
  const perms = () => (g() ? myGuildPerms(g()!) : 0n);
  const can = (bit: bigint) => hasPermission(perms(), bit);
  const nav = (): Array<{ group: string; items: Array<[Tab, string, boolean]> }> => [
    {
      group: g()?.name ?? "",
      items: [
        ["overview", "Обзор", can(Permission.MANAGE_GUILD)],
        ["roles", "Роли", can(Permission.MANAGE_ROLES)],
        ["channels", "Каналы", can(Permission.MANAGE_CHANNELS)],
        ["emojis", "Эмодзи и стикеры", can(Permission.MANAGE_EMOJIS)],
        ["integrations", "Интеграции", can(Permission.MANAGE_WEBHOOKS)],
        ["automod", "Автомодерация", can(Permission.MANAGE_GUILD)],
      ],
    },
    {
      group: "Управление сообществом",
      items: [
        ["members", "Участники", can(Permission.MANAGE_ROLES) || can(Permission.KICK_MEMBERS) || can(Permission.BAN_MEMBERS) || can(Permission.MODERATE_MEMBERS)],
        ["invites", "Приглашения", can(Permission.MANAGE_CHANNELS)],
        ["bans", "Баны", can(Permission.BAN_MEMBERS)],
        ["audit", "Журнал аудита", can(Permission.VIEW_AUDIT_LOG)],
      ],
    },
  ];
  const first = () => nav().flatMap((n) => n.items).find(([, , ok]) => ok)?.[0] ?? "overview";
  const [tab, setTab] = createSignal<Tab>(first());
  let root!: HTMLDivElement;
  const prev = document.activeElement as HTMLElement | null;
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape" && !document.querySelector(".ns-backdrop, .ns-popover")) props.onClose();
  };
  onMount(() => {
    document.addEventListener("keydown", onKey);
    requestAnimationFrame(() => root.querySelector<HTMLElement>('[aria-current="page"]')?.focus());
  });
  onCleanup(() => {
    document.removeEventListener("keydown", onKey);
    prev?.focus?.();
  });
  return (
    <Portal>
      <div ref={root} class="settings-layer" role="dialog" aria-modal="true" aria-label={`Настройки сервера ${g()?.name ?? ""}`}>
        <nav class="settings-nav" aria-label="Разделы настроек сервера">
          <div class="settings-nav-inner">
            <For each={nav()}>
              {(grp) => (
                <>
                  <h2>{grp.group}</h2>
                  <For each={grp.items.filter(([, , ok]) => ok)}>
                    {([id, label]) => (
                      <button class="settings-nav-item" aria-current={tab() === id ? "page" : undefined} onClick={() => setTab(id)}>
                        {label}
                      </button>
                    )}
                  </For>
                  <div class="settings-sep" />
                </>
              )}
            </For>
            <Show when={g()?.ownerId === sid(state.me?.id ?? 0n)}>
              <DeleteGuild g={g()!} onDone={props.onClose} />
            </Show>
          </div>
        </nav>
        <main class="settings-content">
          <div class="settings-content-inner">
            <Show when={g()} fallback={<p>Сервер недоступен.</p>}>
              <Show when={tab() === "overview"}>
                <Overview g={g()!} />
              </Show>
              <Show when={tab() === "roles"}>
                <Roles g={g()!} myPerms={perms()} />
              </Show>
              <Show when={tab() === "channels"}>
                <Channels g={g()!} />
              </Show>
              <Show when={tab() === "members"}>
                <MembersAdmin g={g()!} can={can} />
              </Show>
              <Show when={tab() === "emojis"}>
                <Emojis g={g()!} />
              </Show>
              <Show when={tab() === "integrations"}>
                <Integrations g={g()!} />
              </Show>
              <Show when={tab() === "invites"}>
                <Invites g={g()!} />
              </Show>
              <Show when={tab() === "bans"}>
                <Bans g={g()!} />
              </Show>
              <Show when={tab() === "audit"}>
                <Audit g={g()!} />
              </Show>
              <Show when={tab() === "automod"}>
                <Automod g={g()!} />
              </Show>
            </Show>
          </div>
          <div class="settings-close">
            <button class="settings-close-btn" aria-label="Закрыть настройки сервера (Esc)" onClick={props.onClose}>
              <X size={18} />
            </button>
            <small aria-hidden="true">ESC</small>
          </div>
        </main>
      </div>
    </Portal>
  );
}

function Page(props: { title: string; hint?: string; children: JSX.Element }) {
  return (
    <div class="settings-page">
      <h1>{props.title}</h1>
      <Show when={props.hint}>
        <p class="pe-hint" style={{ "margin-bottom": "16px" }}>{props.hint}</p>
      </Show>
      <div class="settings-stack wide">{props.children}</div>
    </div>
  );
}

const errText = (r: { status: number; body: unknown }, fallback: string) => (r.body as { message?: string } | null)?.message ?? (r.status === 403 ? "Недостаточно прав" : fallback);

// ---------------------------------------------------------------- overview

function Overview(props: { g: GuildView }) {
  const [name, setName] = createSignal(props.g.name);
  const [tag, setTag] = createSignal(props.g.tag);
  const [badge, setBadge] = createSignal(props.g.tagBadge || "shard");
  const [busy, setBusy] = createSignal(false);
  let file!: HTMLInputElement;
  const save = async (body: Record<string, unknown>, ok = "Сохранено") => {
    setBusy(true);
    const r = await api("PATCH", `/guilds/${props.g.id}`, body);
    setBusy(false);
    if (r.status === 200) toast(ok);
    else toast(errText(r, "Не удалось сохранить"), "error");
  };
  return (
    <Page title="Обзор сервера">
      <div class="srv-overview">
        <button class="srv-icon-pick" aria-label="Сменить значок сервера" onClick={() => file.click()}>
          <Show when={asset(props.g.icon)} fallback={<span>{props.g.name.slice(0, 2)}</span>}>
            <img src={asset(props.g.icon)} alt="" />
          </Show>
          <span class="srv-icon-hover">
            <ImagePlus size={20} />
          </span>
        </button>
        <input
          ref={file}
          type="file"
          accept="image/png,image/jpeg,image/webp,image/gif"
          hidden
          onChange={async (e) => {
            const f = e.currentTarget.files?.[0];
            e.currentTarget.value = "";
            if (!f) return;
            try {
              const key = await uploadImage(`/guilds/${props.g.id}/uploads/icon`, f);
              await save({ icon: key }, "Значок обновлён");
            } catch (err) {
              toast((err as Error).message, "error");
            }
          }}
        />
        <div style={{ flex: 1, display: "grid", gap: "12px" }}>
          <TextField label="Название сервера" value={name()} onInput={setName} />
          <div>
            <Button loading={busy()} disabled={!name().trim() || name() === props.g.name} onClick={() => save({ name: name().trim() })}>Сохранить название</Button>
            <Show when={props.g.icon}>
              <Button variant="ghost" onClick={() => save({ icon: "" }, "Значок удалён")}>Удалить значок</Button>
            </Show>
          </div>
        </div>
      </div>
      <h2 class="settings-h2">Тег сервера</h2>
      <p class="pe-hint">Участники могут показывать тег рядом со своим именем (Настройки → Профили → Тег сервера).</p>
      <TextField label="Тег (1–4 буквы или цифры)" value={tag()} onInput={(v) => setTag(v.slice(0, 4))} />
      <div class="pe-chips" role="radiogroup" aria-label="Значок тега">
        <For each={TAG_BADGES}>
          {(b) => (
            <button role="radio" class="pe-chip" aria-checked={badge() === b.id} aria-label={b.label} title={b.label} onClick={() => setBadge(b.id)}>
              <TagBadge id={b.id} size={14} />
            </button>
          )}
        </For>
      </div>
      <Show when={tag().trim()}>
        <p class="pe-hint">
          Так будет выглядеть: <b>{state.me?.displayName || state.me?.username}</b> <TagChip tag={tag().trim()} badge={badge()} />
        </p>
      </Show>
      <div>
        <Button loading={busy()} onClick={() => save({ tag: tag().trim(), tag_badge: badge() }, tag().trim() ? "Тег сохранён" : "Тег убран")}>Сохранить тег</Button>
      </div>
      <Discovery g={props.g} />
      <h2 class="settings-h2">Идентификатор</h2>
      <div>
        <Button variant="secondary" size="sm" onClick={() => void copy(props.g.id, "ID сервера скопирован")}>Копировать ID сервера</Button>
      </div>
    </Page>
  );
}

/** Public server: found by name in «Добавить сервер → Найти» and joined without an invite. */
function Discovery(props: { g: GuildView }) {
  const [cfg, { mutate }] = createResource(async () => {
    const r = await api<{ enabled: boolean; description: string }>("GET", `/guilds/${props.g.id}/discovery`);
    return r.status === 200 ? r.body : { enabled: false, description: "" };
  });
  const [description, setDescription] = createSignal("");
  createEffect(() => setDescription(cfg()?.description ?? ""));
  const save = async (enabled: boolean, text = description()) => {
    const r = await api<{ enabled: boolean; description: string }>("PUT", `/guilds/${props.g.id}/discovery`, { enabled, description: text });
    if (r.status === 200) {
      mutate(r.body);
      toast(enabled ? "Сервер можно найти по названию" : "Сервер скрыт из поиска");
    } else toast(errText(r, "Не удалось сохранить"), "error");
  };
  return (
    <>
      <h2 class="settings-h2">Публичный сервер</h2>
      <Switch
        checked={!!cfg()?.enabled}
        disabled={cfg.loading}
        onChange={(v) => void save(v)}
        label="Можно найти по названию"
        description="Любой пользователь этого сервера NeoShard найдёт его в «Добавить сервер → Найти» и сможет вступить без приглашения. Баны продолжают действовать."
      />
      <Show when={cfg()?.enabled}>
        <TextArea label="Описание для поиска" value={description()} onInput={(v) => void setDescription(v)} max={300} rows={3} />
        <div>
          <Button size="sm" disabled={description() === (cfg()?.description ?? "")} onClick={() => void save(true)}>Сохранить описание</Button>
        </div>
      </Show>
    </>
  );
}

function DeleteGuild(props: { g: GuildView; onDone: () => void }) {
  const [open, setOpen] = createSignal(false);
  const [confirmName, setConfirmName] = createSignal("");
  return (
    <>
      <button class="settings-nav-item danger" onClick={() => setOpen(true)}>
        <Trash2 size={16} /> Удалить сервер
      </button>
      <Modal open={open()} title={`Удалить «${props.g.name}»?`} onClose={() => setOpen(false)}>
        <p style={{ margin: 0 }}>Все каналы и сообщения будут удалены. Это нельзя отменить.</p>
        <TextField label="Введите название сервера" value={confirmName()} onInput={setConfirmName} />
        <Button
          variant="danger"
          disabled={confirmName() !== props.g.name}
          onClick={async () => {
            const r = await api("DELETE", `/guilds/${props.g.id}`);
            if (r.status === 204) {
              setOpen(false);
              props.onDone();
            } else toast(errText(r, "Не удалось удалить сервер"), "error");
          }}
        >
          Удалить сервер
        </Button>
      </Modal>
    </>
  );
}

// ---------------------------------------------------------------- roles

function PermissionList(props: { value: bigint; onToggle: (bit: bigint, on: boolean) => void; editable: (bit: bigint) => boolean }) {
  return (
    <For each={PERMISSION_GROUPS}>
      {(grp) => (
        <section class="perm-group">
          <h3>{grp.title}</h3>
          <For each={grp.items}>
            {([key, label, hint]) => {
              const bit = Permission[key];
              return <Switch label={label} description={hint} checked={(props.value & bit) === bit} disabled={!props.editable(bit)} onChange={(on) => props.onToggle(bit, on)} />;
            }}
          </For>
        </section>
      )}
    </For>
  );
}

function Roles(props: { g: GuildView; myPerms: bigint }) {
  const roles = () => [...props.g.roles].sort((a, b) => b.position - a.position);
  const [sel, setSel] = createSignal<Id | undefined>(roles().find((r) => sid(r.id) !== props.g.id)?.id.toString() ?? props.g.id);
  const role = () => props.g.roles.find((r) => sid(r.id) === sel());
  const isOwner = () => props.g.ownerId === sid(state.me?.id ?? 0n);
  const create = async () => {
    const r = await api<{ id: string }>("POST", `/guilds/${props.g.id}/roles`, { name: "новая роль", color: ROLE_COLORS[props.g.roles.length % ROLE_COLORS.length], permissions: "0" });
    if (r.status === 201) setSel(r.body.id);
    else toast(errText(r, "Не удалось создать роль"), "error");
  };
  const move = async (r: Role, dir: 1 | -1) => {
    const list = roles().filter((x) => sid(x.id) !== props.g.id);
    const i = list.findIndex((x) => x.id === r.id);
    const j = i - dir;
    if (j < 0 || j >= list.length) return;
    [list[i], list[j]] = [list[j], list[i]];
    const body = list.map((x, k) => ({ id: sid(x.id), position: list.length - k }));
    const res = await api("PATCH", `/guilds/${props.g.id}/roles`, body);
    if (res.status !== 200 && res.status !== 204) toast(errText(res, "Не удалось переставить роли"), "error");
  };
  return (
    <Page title="Роли" hint="Роль выше в списке главнее. Изменять можно только роли ниже вашей высшей роли и только теми правами, которые есть у вас.">
      <div class="roles-layout">
        <div class="roles-list" role="listbox" aria-label="Роли">
          <Button size="sm" onClick={create}>Создать роль</Button>
          <For each={roles()}>
            {(r) => (
              <div class="role-row" classList={{ selected: sel() === sid(r.id) }}>
                <button class="role-pick" role="option" aria-selected={sel() === sid(r.id)} onClick={() => setSel(sid(r.id))}>
                  <i style={{ background: r.color ? hex(r.color) : "var(--ns-text-faint)" }} />
                  {sid(r.id) === props.g.id ? "@everyone" : r.name}
                </button>
                <Show when={sid(r.id) !== props.g.id}>
                  <button class="ns-icon-btn mini" aria-label={`Поднять ${r.name}`} onClick={() => move(r, 1)}>↑</button>
                  <button class="ns-icon-btn mini" aria-label={`Опустить ${r.name}`} onClick={() => move(r, -1)}>↓</button>
                </Show>
              </div>
            )}
          </For>
        </div>
        <Show when={role()} keyed>
          {(r) => <RoleEditor g={props.g} role={r} myPerms={props.myPerms} isOwner={isOwner()} onDeleted={() => setSel(props.g.id)} />}
        </Show>
      </div>
    </Page>
  );
}

function RoleEditor(props: { g: GuildView; role: Role; myPerms: bigint; isOwner: boolean; onDeleted: () => void }) {
  const everyone = () => sid(props.role.id) === props.g.id;
  const [name, setName] = createSignal(props.role.name);
  const [color, setColor] = createSignal(props.role.color);
  const [hoist, setHoist] = createSignal(props.role.hoist);
  const [mentionable, setMentionable] = createSignal(props.role.mentionable);
  const [bits, setBits] = createSignal(props.role.permissions);
  const dirty = () => name() !== props.role.name || color() !== props.role.color || hoist() !== props.role.hoist || mentionable() !== props.role.mentionable || bits() !== props.role.permissions;
  const [busy, setBusy] = createSignal(false);
  const editable = (bit: bigint) => props.isOwner || hasPermission(props.myPerms, bit);
  const save = async () => {
    setBusy(true);
    const body: Record<string, unknown> = { permissions: bits().toString() };
    if (!everyone()) Object.assign(body, { name: name().trim(), color: color(), hoist: hoist(), mentionable: mentionable() });
    const r = await api("PATCH", `/guilds/${props.g.id}/roles/${sid(props.role.id)}`, body);
    setBusy(false);
    if (r.status === 200) toast("Роль сохранена");
    else toast(errText(r, "Не удалось сохранить роль"), "error");
  };
  return (
    <div class="role-editor">
      <Show when={!everyone()} fallback={<p class="pe-hint">@everyone — права всех участников сервера по умолчанию.</p>}>
        <TextField label="Название роли" value={name()} onInput={setName} />
        <div class="ns-field">
          <span class="ns-label">Цвет роли</span>
          <div class="pe-row">
            <For each={ROLE_COLORS}>{(c) => <button class="swatch" style={{ background: hex(c) }} aria-label={`Цвет ${hex(c)}`} aria-pressed={color() === c} onClick={() => setColor(c)} />}</For>
            <ColorField label="Свой" value={color() || 0x95a5b4} onInput={setColor} />
            <Button size="sm" variant="ghost" onClick={() => setColor(0)}>Без цвета</Button>
          </div>
        </div>
        <Switch label="Показывать участников с ролью отдельно" description="Отдельная группа в списке участников" checked={hoist()} onChange={setHoist} />
        <Switch label="Разрешить упоминать роль" checked={mentionable()} onChange={setMentionable} />
      </Show>
      <h2 class="settings-h2">Права</h2>
      <PermissionList value={bits()} editable={editable} onToggle={(bit, on) => setBits(on ? bits() | bit : bits() & ~bit)} />
      <div class="pe-row" style={{ "margin-top": "12px" }}>
        <Show when={!everyone()}>
          <Button
            variant="danger"
            size="sm"
            onClick={async () => {
              if (!confirm(`Удалить роль «${props.role.name}»?`)) return;
              const r = await api("DELETE", `/guilds/${props.g.id}/roles/${sid(props.role.id)}`);
              if (r.status === 204) props.onDeleted();
              else toast(errText(r, "Не удалось удалить роль"), "error");
            }}
          >
            Удалить роль
          </Button>
        </Show>
      </div>
      <Show when={dirty()}>
        <div class="pe-savebar" role="region" aria-label="Несохранённые изменения">
          <span>Есть несохранённые изменения</span>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              setName(props.role.name);
              setColor(props.role.color);
              setHoist(props.role.hoist);
              setMentionable(props.role.mentionable);
              setBits(props.role.permissions);
            }}
          >
            Сбросить
          </Button>
          <Button size="sm" loading={busy()} onClick={save}>Сохранить изменения</Button>
        </div>
      </Show>
    </div>
  );
}

// ---------------------------------------------------------------- channels

const CHANNEL_TYPES: Array<[ChannelType, string, () => JSX.Element]> = [
  [ChannelType.TEXT, "Текстовый", () => <Hash size={16} />],
  [ChannelType.VOICE, "Голосовой", () => <Volume2 size={16} />],
  [ChannelType.ANNOUNCEMENT, "Объявления", () => <Megaphone size={16} />],
  [ChannelType.FORUM, "Форум", () => <MessageSquare size={16} />],
  [ChannelType.CATEGORY, "Категория", () => <ChevronDown size={16} />],
];

const typeIcon = (t: ChannelType) => (CHANNEL_TYPES.find(([x]) => x === t)?.[2] ?? (() => <Hash size={16} />))();

function Channels(props: { g: GuildView }) {
  const ordered = createMemo(() => {
    const chans = props.g.channels.filter((c) => c.type !== ChannelType.THREAD);
    const cats = chans.filter((c) => c.type === ChannelType.CATEGORY);
    const out: Array<{ c: Channel; depth: number }> = [];
    for (const c of chans.filter((x) => x.type !== ChannelType.CATEGORY && x.parentId === 0n)) out.push({ c, depth: 0 });
    for (const cat of cats) {
      out.push({ c: cat, depth: 0 });
      for (const c of chans.filter((x) => x.parentId === cat.id && x.type !== ChannelType.CATEGORY)) out.push({ c, depth: 1 });
    }
    return out;
  });
  const [sel, setSel] = createSignal<Id | undefined>(ordered()[0] ? sid(ordered()[0].c.id) : undefined);
  const channel = () => props.g.channels.find((c) => sid(c.id) === sel());
  const [creating, setCreating] = createSignal(false);
  const move = async (c: Channel, dir: 1 | -1) => {
    const siblings = props.g.channels.filter((x) => x.parentId === c.parentId && (x.type === ChannelType.CATEGORY) === (c.type === ChannelType.CATEGORY) && x.type !== ChannelType.THREAD);
    const i = siblings.findIndex((x) => x.id === c.id);
    const j = i - dir;
    if (j < 0 || j >= siblings.length) return;
    [siblings[i], siblings[j]] = [siblings[j], siblings[i]];
    const r = await api("PATCH", `/guilds/${props.g.id}/channels`, siblings.map((x, k) => ({ id: sid(x.id), position: k })));
    if (r.status !== 200 && r.status !== 204) toast(errText(r, "Не удалось переставить каналы"), "error");
  };
  return (
    <Page title="Каналы" hint="Категории и каналы, их порядок, описание, медленный режим и права для отдельных ролей.">
      <div class="roles-layout">
        <div class="roles-list">
          <Button size="sm" onClick={() => setCreating(true)}>Создать канал</Button>
          <For each={ordered()}>
            {({ c, depth }) => (
              <div class="role-row" classList={{ selected: sel() === sid(c.id), category: c.type === ChannelType.CATEGORY }} style={{ "padding-left": `${depth * 16}px` }}>
                <button class="role-pick" aria-pressed={sel() === sid(c.id)} onClick={() => setSel(sid(c.id))}>
                  {typeIcon(c.type)}
                  {c.name}
                </button>
                <button class="ns-icon-btn mini" aria-label={`Выше: ${c.name}`} onClick={() => move(c, 1)}>↑</button>
                <button class="ns-icon-btn mini" aria-label={`Ниже: ${c.name}`} onClick={() => move(c, -1)}>↓</button>
              </div>
            )}
          </For>
        </div>
        <Show when={channel()} keyed>
          {(c) => <ChannelEditor g={props.g} c={c} onDeleted={() => setSel(undefined)} />}
        </Show>
      </div>
      <CreateChannel g={props.g} open={creating()} onClose={() => setCreating(false)} onCreated={(id) => setSel(id)} />
    </Page>
  );
}

function CreateChannel(props: { g: GuildView; open: boolean; onClose: () => void; onCreated: (id: Id) => void }) {
  const [name, setName] = createSignal("");
  const [type, setType] = createSignal<ChannelType>(ChannelType.TEXT);
  const [parent, setParent] = createSignal("");
  const [priv, setPriv] = createSignal(false);
  const cats = () => props.g.channels.filter((c) => c.type === ChannelType.CATEGORY);
  const create = async () => {
    const body: Record<string, unknown> = { name: name().trim(), type: type() };
    if (parent() && type() !== ChannelType.CATEGORY) body.parent_id = parent();
    // Private: @everyone cannot see it; roles are granted access in the channel editor.
    if (priv()) body.overwrites = [{ id: props.g.id, type: 0, allow: "0", deny: Permission.VIEW_CHANNEL.toString() }];
    const r = await api<{ id: string }>("POST", `/guilds/${props.g.id}/channels`, body);
    if (r.status === 201) {
      props.onCreated(r.body.id);
      props.onClose();
      setName("");
    } else toast(errText(r, "Не удалось создать канал"), "error");
  };
  return (
    <Modal open={props.open} title="Создать канал" onClose={props.onClose}>
      <div class="pe-chips" role="radiogroup" aria-label="Тип канала">
        <For each={CHANNEL_TYPES}>
          {([t, label, icon]) => (
            <button role="radio" class="pe-chip" aria-checked={type() === t} onClick={() => setType(t)}>
              {icon()} {label}
            </button>
          )}
        </For>
      </div>
      <TextField label="Название" value={name()} onInput={(v) => setName(type() === ChannelType.CATEGORY ? v : v.toLowerCase().replace(/\s+/g, "-"))} />
      <Show when={type() !== ChannelType.CATEGORY}>
        <label class="ns-field">
          <span class="ns-label">Категория</span>
          <select class="ns-select" value={parent()} onChange={(e) => setParent(e.currentTarget.value)}>
            <option value="">Без категории</option>
            <For each={cats()}>{(c) => <option value={sid(c.id)}>{c.name}</option>}</For>
          </select>
        </label>
      </Show>
      <Switch label="Закрытый канал" description="Видят только роли, которым вы откроете доступ" checked={priv()} onChange={setPriv} />
      <Button disabled={!name().trim()} onClick={create}>Создать</Button>
    </Modal>
  );
}

function ChannelEditor(props: { g: GuildView; c: Channel; onDeleted: () => void }) {
  const [name, setName] = createSignal(props.c.name);
  const [topic, setTopic] = createSignal(props.c.topic);
  const [slow, setSlow] = createSignal(props.c.slowmodeSeconds);
  const [nsfw, setNsfw] = createSignal(props.c.nsfw);
  const [target, setTarget] = createSignal<Id>(props.g.id);
  const isCat = () => props.c.type === ChannelType.CATEGORY;
  const save = async () => {
    const r = await api("PATCH", `/channels/${sid(props.c.id)}`, { name: name().trim(), topic: topic(), slowmode_seconds: slow(), nsfw: nsfw() });
    if (r.status === 200) toast("Канал сохранён");
    else toast(errText(r, "Не удалось сохранить канал"), "error");
  };
  const ow = () => props.c.overwrites.find((o) => sid(o.id) === target());
  const setBit = async (bit: bigint, mode: "allow" | "deny" | "inherit") => {
    const cur = ow();
    let allow = cur?.allow ?? 0n;
    let deny = cur?.deny ?? 0n;
    allow &= ~bit;
    deny &= ~bit;
    if (mode === "allow") allow |= bit;
    if (mode === "deny") deny |= bit;
    const path = `/channels/${sid(props.c.id)}/overwrites/${target()}`;
    const r = allow === 0n && deny === 0n ? await api("DELETE", path) : await api("PUT", path, { type: 0, allow: allow.toString(), deny: deny.toString() });
    if (r.status !== 204 && r.status !== 200) toast(errText(r, "Не удалось изменить права"), "error");
  };
  const SLOW: Array<[number, string]> = [[0, "выкл."], [5, "5 с"], [10, "10 с"], [30, "30 с"], [60, "1 мин"], [300, "5 мин"], [900, "15 мин"], [3600, "1 ч"]];
  const channelGroups = () => PERMISSION_GROUPS.filter((g) => g.title !== "Участники" && g.title !== "Особые" && (props.c.type === ChannelType.VOICE ? g.title !== "Текстовые каналы" : g.title !== "Голосовые каналы" || isCat()));
  return (
    <div class="role-editor">
      <TextField label="Название" value={name()} onInput={setName} />
      <Show when={!isCat()}>
        <TextArea label="Описание канала" value={topic()} onInput={setTopic} max={1024} rows={3} />
        <label class="ns-field">
          <span class="ns-label">Медленный режим</span>
          <select class="ns-select" value={slow()} onChange={(e) => setSlow(Number(e.currentTarget.value))}>
            <For each={SLOW}>{([v, l]) => <option value={v}>{l}</option>}</For>
          </select>
        </label>
        <Switch label="Канал 18+" checked={nsfw()} onChange={setNsfw} />
      </Show>
      <div class="pe-row">
        <Button onClick={save}>Сохранить</Button>
        <Button
          variant="danger"
          onClick={async () => {
            if (!confirm(`Удалить «${props.c.name}»?`)) return;
            const r = await api("DELETE", `/channels/${sid(props.c.id)}`);
            if (r.status === 204) props.onDeleted();
            else toast(errText(r, "Не удалось удалить"), "error");
          }}
        >
          Удалить
        </Button>
      </div>
      <h2 class="settings-h2">Права в канале</h2>
      <p class="pe-hint">Переопределения для роли: ✓ — разрешить, ✕ — запретить, / — как в роли.</p>
      <label class="ns-field">
        <span class="ns-label">Роль</span>
        <select class="ns-select" value={target()} onChange={(e) => setTarget(e.currentTarget.value)}>
          <For each={[...props.g.roles].sort((a, b) => b.position - a.position)}>{(r) => <option value={sid(r.id)}>{sid(r.id) === props.g.id ? "@everyone" : r.name}</option>}</For>
        </select>
      </label>
      <For each={channelGroups()}>
        {(grp) => (
          <section class="perm-group">
            <h3>{grp.title}</h3>
            <For each={grp.items}>
              {([key, label]) => {
                const bit = Permission[key];
                const mode = () => ((ow()?.allow ?? 0n) & bit ? "allow" : (ow()?.deny ?? 0n) & bit ? "deny" : "inherit");
                return (
                  <div class="tri-row">
                    <span>{label}</span>
                    <div class="tri" role="radiogroup" aria-label={label}>
                      <button role="radio" class="tri-deny" aria-checked={mode() === "deny"} aria-label="Запретить" onClick={() => setBit(bit, "deny")}>✕</button>
                      <button role="radio" aria-checked={mode() === "inherit"} aria-label="Как в роли" onClick={() => setBit(bit, "inherit")}>/</button>
                      <button role="radio" class="tri-allow" aria-checked={mode() === "allow"} aria-label="Разрешить" onClick={() => setBit(bit, "allow")}>✓</button>
                    </div>
                  </div>
                );
              }}
            </For>
          </section>
        )}
      </For>
    </div>
  );
}

// ---------------------------------------------------------------- members

interface MemberJson {
  user?: { id: string; username: string; display_name?: string; avatar?: string; bot?: boolean };
  nick?: string;
  role_ids?: string[];
  joined_at?: string;
  timeout_until?: string;
}

function MembersAdmin(props: { g: GuildView; can: (bit: bigint) => boolean }) {
  const [members, { refetch }] = createResource(async () => (await api<MemberJson[]>("GET", `/guilds/${props.g.id}/members?limit=1000`)).body ?? []);
  const [filter, setFilter] = createSignal("");
  const [rolesFor, setRolesFor] = createSignal<MemberJson>();
  const list = () => (members() ?? []).filter((m) => !filter() || `${m.user?.username} ${m.user?.display_name ?? ""} ${m.nick ?? ""}`.toLowerCase().includes(filter().toLowerCase()));
  const act = async (method: string, path: string, body: unknown, ok: string) => {
    const r = await api(method, path, body);
    if (r.status >= 200 && r.status < 300) {
      toast(ok);
      void refetch();
    } else toast(errText(r, "Не получилось"), "error");
  };
  const timedOut = (m: MemberJson) => Number(m.timeout_until ?? 0) > Date.now();
  return (
    <Page title="Участники" hint="Роли, тайм-ауты, исключение и бан. Действовать можно только на участников с ролью ниже вашей.">
      <TextField label="Поиск участника" value={filter()} onInput={setFilter} />
      <div class="admin-table" role="table" aria-label="Участники">
        <For each={list()}>
          {(m) => (
            <div class="admin-row" role="row">
              <span role="cell" class="admin-who">
                <b>{m.nick || m.user?.display_name || m.user?.username}</b>
                <small class="edited">@{m.user?.username}</small>
              </span>
              <span role="cell" class="admin-roles">
                <For each={(m.role_ids ?? []).map((id) => props.g.roles.find((r) => sid(r.id) === id)).filter(Boolean)}>
                  {(r) => (
                    <span class="role-chip" style={{ "--rc": r!.color ? hex(r!.color) : undefined }}>
                      <i />
                      {r!.name}
                    </span>
                  )}
                </For>
                <Show when={timedOut(m)}>
                  <span class="tag-chip">тайм-аут</span>
                </Show>
              </span>
              <span role="cell" class="admin-actions">
                <Show when={props.can(Permission.MANAGE_ROLES)}>
                  <Button size="sm" variant="secondary" onClick={() => setRolesFor(m)}>Роли</Button>
                </Show>
                <Show when={props.can(Permission.MODERATE_MEMBERS) && m.user?.id !== sid(state.me?.id ?? 0n)}>
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() =>
                      act("PATCH", `/guilds/${props.g.id}/members/${m.user!.id}`, { timeout_until: timedOut(m) ? 0 : Date.now() + 10 * 60_000, reason: "из настроек сервера" }, timedOut(m) ? "Тайм-аут снят" : "Тайм-аут на 10 минут")
                    }
                  >
                    {timedOut(m) ? "Снять тайм-аут" : "Тайм-аут 10 мин"}
                  </Button>
                </Show>
                <Show when={props.can(Permission.KICK_MEMBERS) && m.user?.id !== sid(state.me?.id ?? 0n)}>
                  <Button size="sm" variant="ghost" onClick={() => confirm(`Выгнать ${m.user?.username}?`) && act("DELETE", `/guilds/${props.g.id}/members/${m.user!.id}`, undefined, "Участник исключён")}>
                    Выгнать
                  </Button>
                </Show>
                <Show when={props.can(Permission.BAN_MEMBERS) && m.user?.id !== sid(state.me?.id ?? 0n)}>
                  <Button size="sm" variant="danger" onClick={() => confirm(`Забанить ${m.user?.username}?`) && act("PUT", `/guilds/${props.g.id}/bans/${m.user!.id}`, { reason: "" }, "Участник забанен")}>
                    Бан
                  </Button>
                </Show>
              </span>
            </div>
          )}
        </For>
      </div>
      <Modal open={!!rolesFor()} title={`Роли: ${rolesFor()?.user?.username ?? ""}`} onClose={() => setRolesFor(undefined)}>
        <For each={[...props.g.roles].filter((r) => sid(r.id) !== props.g.id).sort((a, b) => b.position - a.position)}>
          {(r) => (
            <Switch
              label={r.name}
              checked={(rolesFor()?.role_ids ?? []).includes(sid(r.id))}
              onChange={async (on) => {
                const m = rolesFor()!;
                const res = await api(on ? "PUT" : "DELETE", `/guilds/${props.g.id}/members/${m.user!.id}/roles/${sid(r.id)}`, on ? {} : undefined);
                if (res.status === 204 || res.status === 200) {
                  setRolesFor({ ...m, role_ids: on ? [...(m.role_ids ?? []), sid(r.id)] : (m.role_ids ?? []).filter((x) => x !== sid(r.id)) });
                  void refetch();
                } else toast(errText(res, "Нельзя выдать эту роль"), "error");
              }}
            />
          )}
        </For>
      </Modal>
    </Page>
  );
}

// ---------------------------------------------------------------- emojis

/** Emoji names are ASCII [A-Za-z0-9_]{2,32}; Cyrillic is transliterated. */
export function emojiName(raw: string): string {
  const tr: Record<string, string> = { а: "a", б: "b", в: "v", г: "g", д: "d", е: "e", ё: "e", ж: "zh", з: "z", и: "i", й: "y", к: "k", л: "l", м: "m", н: "n", о: "o", п: "p", р: "r", с: "s", т: "t", у: "u", ф: "f", х: "h", ц: "c", ч: "ch", ш: "sh", щ: "sch", ъ: "", ы: "y", ь: "", э: "e", ю: "yu", я: "ya" };
  const t = [...raw.toLowerCase()].map((c) => tr[c] ?? c).join("");
  const n = t.replace(/[^a-z0-9_]+/g, "_").replace(/^_+|_+$/g, "").slice(0, 32);
  return n.length >= 2 ? n : `emoji_${n}`.slice(0, 32);
}

function Emojis(props: { g: GuildView }) {
  const [list, { refetch }] = createResource(async () => (await api<Array<{ id: string; name: string; animated?: boolean; key?: string }>>("GET", `/guilds/${props.g.id}/emojis`)).body ?? []);
  const [name, setName] = createSignal("");
  let file!: HTMLInputElement;
  const add = async (f: File) => {
    const n = emojiName(name().trim() || f.name.replace(/\.[^.]+$/, ""));
    try {
      const key = await uploadImage(`/guilds/${props.g.id}/uploads/emoji`, f);
      const r = await api("POST", `/guilds/${props.g.id}/emojis`, { name: n, key, animated: f.type === "image/gif" });
      if (r.status === 201 || r.status === 200) {
        toast(`Эмодзи :${n}: добавлен`);
        setName("");
        void refetch();
      } else toast(errText(r, "Не удалось добавить эмодзи"), "error");
    } catch (e) {
      toast((e as Error).message, "error");
    }
  };
  return (
    <>
    <Page title="Эмодзи" hint="Свои эмодзи сервера: PNG, WebP или GIF до 256 КБ, лучше квадратные 128×128. Имя — латиница, цифры и _, от 2 до 32 символов; в сообщении пишется как :имя:.">
      <div class="pe-row">
        <TextField label="Название (необязательно)" value={name()} onInput={setName} />
        <input ref={file} type="file" accept="image/png,image/webp,image/gif,image/jpeg" hidden onChange={(e) => (e.currentTarget.files?.[0] && add(e.currentTarget.files[0]), (e.currentTarget.value = ""))} />
        <Button onClick={() => file.click()}>
          <ImagePlus size={16} /> Загрузить эмодзи
        </Button>
      </div>
      <div class="emoji-admin">
        <For each={list() ?? []} fallback={<p class="edited">Пока нет своих эмодзи.</p>}>
          {(e) => (
            <div class="emoji-admin-row">
              <Show when={asset(e.key)} fallback={<span class="emoji-admin-img">?</span>}>
                <img class="emoji-admin-pic" src={asset(e.key)} alt="" />
              </Show>
              <span class="emoji-admin-img">:{e.name}:</span>
              <span style={{ flex: 1 }}>{e.name}</span>
              <button
                class="ns-icon-btn"
                aria-label={`Удалить :${e.name}:`}
                onClick={async () => {
                  const r = await api("DELETE", `/guilds/${props.g.id}/emojis/${e.id}`);
                  if (r.status === 204) void refetch();
                  else toast(errText(r, "Не удалось удалить"), "error");
                }}
              >
                <Trash2 size={16} />
              </button>
            </div>
          )}
        </For>
      </div>
    </Page>
    <Stickers g={props.g} />
    </>
  );
}

function Stickers(props: { g: GuildView }) {
  const [list, { refetch }] = createResource(async () => (await api<Array<{ id: string; name: string; key?: string }>>("GET", `/guilds/${props.g.id}/stickers`)).body ?? []);
  const [name, setName] = createSignal("");
  let file!: HTMLInputElement;
  const add = async (f: File) => {
    const n = (name().trim() || f.name.replace(/\.[^.]+$/, "")).slice(0, 30);
    try {
      const key = await uploadImage(`/guilds/${props.g.id}/uploads/sticker`, f);
      const format = f.type === "image/webp" ? "webp" : f.type === "image/gif" ? "apng" : "png";
      const r = await api("POST", `/guilds/${props.g.id}/stickers`, { name: n, key, format });
      if (r.status === 201 || r.status === 200) {
        toast(`Стикер «${n}» добавлен`);
        setName("");
        void refetch();
      } else toast(errText(r, "Не удалось добавить стикер"), "error");
    } catch (e) {
      toast((e as Error).message, "error");
    }
  };
  return (
    <div class="settings-page" style={{ "margin-top": "32px" }}>
      <h2 class="settings-h2">Стикеры</h2>
      <p class="pe-hint">PNG, WebP или GIF до 512 КБ, лучше 320×320. Отправляются из выбора эмодзи → «Стикеры».</p>
      <div class="pe-row">
        <TextField label="Название стикера" value={name()} onInput={setName} />
        <input ref={file} type="file" accept="image/png,image/webp,image/gif" hidden onChange={(e) => (e.currentTarget.files?.[0] && add(e.currentTarget.files[0]), (e.currentTarget.value = ""))} />
        <Button onClick={() => file.click()}>
          <ImagePlus size={16} /> Загрузить стикер
        </Button>
      </div>
      <div class="sticker-admin">
        <For each={list() ?? []} fallback={<p class="edited">Стикеров пока нет.</p>}>
          {(st) => (
            <div class="sticker-admin-item">
              <Show when={asset(st.key)}>
                <img src={asset(st.key)} alt="" />
              </Show>
              <span>{st.name}</span>
              <button
                class="ns-icon-btn"
                aria-label={`Удалить стикер ${st.name}`}
                onClick={async () => {
                  const r = await api("DELETE", `/guilds/${props.g.id}/stickers/${st.id}`);
                  if (r.status === 204) void refetch();
                  else toast(errText(r, "Не удалось удалить"), "error");
                }}
              >
                <Trash2 size={16} />
              </button>
            </div>
          )}
        </For>
      </div>
    </div>
  );
}

function Integrations(props: { g: GuildView }) {
  const [hooks, { refetch }] = createResource(async () => (await api<Array<{ id: string; channel_id: string; name: string }>>("GET", `/guilds/${props.g.id}/webhooks`)).body ?? []);
  const textChannels = () => props.g.channels.filter((c) => c.type === ChannelType.TEXT || c.type === ChannelType.ANNOUNCEMENT);
  const [name, setName] = createSignal("Вебхук");
  const [channel, setChannel] = createSignal(textChannels()[0] ? sid(textChannels()[0].id) : "");
  const [created, setCreated] = createSignal<string>();
  const create = async () => {
    const r = await api<{ url: string; message?: string }>("POST", `/channels/${channel()}/webhooks`, { name: name().trim() || "Вебхук" });
    if (r.status === 201 || r.status === 200) {
      setCreated(r.body.url.startsWith("http") ? r.body.url : `${location.origin}${r.body.url}`);
      void refetch();
    } else toast(errText(r, "Не удалось создать вебхук"), "error");
  };
  return (
    <Page title="Интеграции" hint="Вебхуки публикуют сообщения в канал из внешних сервисов (CI, мониторинг, RSS) простым HTTP POST.">
      <div class="pe-row">
        <TextField label="Имя вебхука" value={name()} onInput={setName} />
        <label class="ns-field">
          <span class="ns-label">Канал</span>
          <select class="ns-select" value={channel()} onChange={(e) => setChannel(e.currentTarget.value)}>
            <For each={textChannels()}>{(c) => <option value={sid(c.id)}>#{c.name}</option>}</For>
          </select>
        </label>
        <Button disabled={!channel()} onClick={create}>Создать вебхук</Button>
      </div>
      <Show when={created()}>
        <div class="rule-card">
          <div style={{ flex: 1, "min-width": 0 }}>
            <b>Адрес вебхука — сохраните его, он показывается один раз</b>
            <code class="md-code" style={{ "word-break": "break-all" }}>{created()}</code>
            <small class="edited">Пример: curl -X POST -H "content-type: application/json" -d &#123;"content":"Привет!"&#125; адрес</small>
          </div>
          <Button size="sm" variant="secondary" onClick={() => void copy(created()!, "Адрес скопирован")}>Копировать</Button>
        </div>
      </Show>
      <div class="admin-table">
        <For each={hooks() ?? []} fallback={<p class="edited">Вебхуков пока нет.</p>}>
          {(h) => (
            <div class="admin-row">
              <span class="admin-who">
                <b>{h.name}</b>
                <small class="edited">#{props.g.channels.find((c) => sid(c.id) === h.channel_id)?.name ?? "канал"}</small>
              </span>
              <span class="admin-roles" />
              <span class="admin-actions">
                <Button
                  size="sm"
                  variant="danger"
                  onClick={async () => {
                    if (!confirm(`Удалить вебхук «${h.name}»?`)) return;
                    const r = await api("DELETE", `/webhooks/${h.id}`);
                    if (r.status === 204) void refetch();
                    else toast(errText(r, "Не удалось удалить"), "error");
                  }}
                >
                  Удалить
                </Button>
              </span>
            </div>
          )}
        </For>
      </div>
    </Page>
  );
}

// ---------------------------------------------------------------- invites, bans, audit

function Invites(props: { g: GuildView }) {
  const textChannels = () => props.g.channels.filter((c) => c.type !== ChannelType.CATEGORY && c.type !== ChannelType.THREAD);
  const [list, { refetch }] = createResource(async () => {
    const all = await Promise.all(textChannels().map(async (c) => ((await api<Array<{ code: string; uses?: number; max_uses?: number; expires_at?: string; inviter_id?: string }>>("GET", `/channels/${sid(c.id)}/invites`)).body ?? []).map((i) => ({ ...i, channel: c.name }))));
    return all.flat();
  });
  return (
    <Page title="Приглашения" hint="Действующие приглашения на сервер. Удалённое приглашение перестаёт работать сразу.">
      <div class="admin-table">
        <For each={list() ?? []} fallback={<p class="edited">Нет активных приглашений.</p>}>
          {(i) => (
            <div class="admin-row">
              <span class="admin-who">
                <b class="mono">{i.code}</b>
                <small class="edited">#{i.channel} · {state.users[i.inviter_id ?? ""]?.username ?? ""}</small>
              </span>
              <span class="admin-roles">
                <small>
                  использовано {i.uses ?? 0}
                  {i.max_uses ? ` из ${i.max_uses}` : ""}
                  {i.expires_at && Number(i.expires_at) ? ` · до ${new Date(Number(i.expires_at)).toLocaleDateString("ru-RU")}` : ""}
                </small>
              </span>
              <span class="admin-actions">
                <Button size="sm" variant="ghost" onClick={() => void copy(inviteLink(i.code), "Ссылка скопирована")}>Копировать</Button>
                <Button
                  size="sm"
                  variant="danger"
                  onClick={async () => {
                    const r = await api("DELETE", `/invites/${i.code}`);
                    if (r.status === 204) void refetch();
                    else toast(errText(r, "Не удалось удалить"), "error");
                  }}
                >
                  Удалить
                </Button>
              </span>
            </div>
          )}
        </For>
      </div>
    </Page>
  );
}

function Bans(props: { g: GuildView }) {
  const [list, { refetch }] = createResource(async () => (await api<Array<{ user?: { id: string; username: string }; reason?: string; created_at?: number }>>("GET", `/guilds/${props.g.id}/bans`)).body ?? []);
  return (
    <Page title="Баны">
      <div class="admin-table">
        <For each={list() ?? []} fallback={<p class="edited">Никто не забанен.</p>}>
          {(b) => (
            <div class="admin-row">
              <span class="admin-who">
                <b>{b.user?.username}</b>
                <small class="edited">{b.reason || "без причины"}</small>
              </span>
              <span class="admin-roles">
                <small>{b.created_at ? new Date(b.created_at).toLocaleString("ru-RU") : ""}</small>
              </span>
              <span class="admin-actions">
                <Button
                  size="sm"
                  variant="secondary"
                  onClick={async () => {
                    const r = await api("DELETE", `/guilds/${props.g.id}/bans/${b.user?.id}`);
                    if (r.status === 204) void refetch();
                    else toast(errText(r, "Не удалось разбанить"), "error");
                  }}
                >
                  Разбанить
                </Button>
              </span>
            </div>
          )}
        </For>
      </div>
    </Page>
  );
}

const AUDIT_LABELS: Record<string, string> = {
  "guild.update": "изменил сервер",
  "channel.create": "создал канал",
  "channel.update": "изменил канал",
  "channel.delete": "удалил канал",
  "role.create": "создал роль",
  "role.update": "изменил роль",
  "role.delete": "удалил роль",
  "member.kick": "выгнал участника",
  "member.ban": "забанил",
  "member.unban": "разбанил",
  "member.timeout": "отправил подумать",
  "member.timeout_clear": "снял тайм-аут",
  "member.roles": "изменил роли",
  "member.nick": "изменил никнейм",
  "message.delete": "удалил сообщение",
  "message.pin": "закрепил сообщение",
  "automod.trigger": "сработала автомодерация на",
  "automod.rule_create": "добавил правило автомодерации",
  "invite.create": "создал приглашение",
  "invite.delete": "удалил приглашение",
  "channel.overwrite": "изменил права в канале",
  "channel.overwrite_delete": "сбросил права в канале",
  "emoji.create": "добавил эмодзи",
  "guild.transfer": "передал сервер",
  "member.role_add": "выдал роль",
  "member.role_remove": "снял роль",
  "message.bulk_delete": "удалил сообщения",
  "webhook.create": "создал вебхук",
  "webhook.delete": "удалил вебхук",
};

function Audit(props: { g: GuildView }) {
  const [list] = createResource(async () => {
    const rows = (await api<Array<{ id: string; user_id?: string; action: string; target_id?: string; reason?: string; changes_json?: string }>>("GET", `/guilds/${props.g.id}/audit-log?limit=100`)).body ?? [];
    ensureUsers(rows.flatMap((r) => [r.user_id, r.target_id]).filter((x): x is string => !!x));
    return rows;
  });
  const who = (id?: string) => (id && state.users[id] ? state.users[id].displayName || state.users[id].username : id ?? "система");
  const target = (id?: string) => {
    if (!id) return "";
    const ch = props.g.channels.find((c) => sid(c.id) === id);
    if (ch) return `#${ch.name}`;
    const r = props.g.roles.find((x) => sid(x.id) === id);
    if (r) return `@${r.name}`;
    return state.users[id] ? `@${state.users[id].username}` : "";
  };
  return (
    <Page title="Журнал аудита" hint="Последние 100 действий модераторов и администраторов.">
      <div class="admin-table">
        <For each={list() ?? []} fallback={<p class="edited">Журнал пуст.</p>}>
          {(e) => (
            <div class="admin-row audit">
              <span class="admin-who">
                <span>
                  <b>{who(e.user_id)}</b> {AUDIT_LABELS[e.action] ?? e.action} <b>{target(e.target_id)}</b>
                </span>
                <Show when={e.reason}>
                  <small class="edited">Причина: {e.reason}</small>
                </Show>
              </span>
              <span class="admin-roles">
                <small>{new Date(snowflakeMs(e.id)).toLocaleString("ru-RU")}</small>
              </span>
            </div>
          )}
        </For>
      </div>
    </Page>
  );
}

const snowflakeMs = (id: string) => snowflakeTime(BigInt(id));

// ---------------------------------------------------------------- automod

interface RuleJson {
  id: string;
  name: string;
  enabled: boolean;
  action: "block" | "timeout" | "alert";
  config: { kind: "keyword" | "mention_spam" | "link"; keywords?: string[]; allow?: string[]; max?: number; allowed_domains?: string[] };
}

function Automod(props: { g: GuildView }) {
  const [rules, { refetch }] = createResource(async () => (await api<RuleJson[]>("GET", `/guilds/${props.g.id}/automod/rules`)).body ?? []);
  const [kind, setKind] = createSignal<RuleJson["config"]["kind"]>("keyword");
  const [name, setName] = createSignal("");
  const [words, setWords] = createSignal("");
  const [max, setMax] = createSignal(5);
  const [action, setAction] = createSignal<RuleJson["action"]>("block");
  const list = (s: string) => s.split(/[,\n]/).map((x) => x.trim()).filter(Boolean);
  const create = async () => {
    const cfg = kind() === "keyword" ? { kind: "keyword", keywords: list(words()) } : kind() === "link" ? { kind: "link", allowed_domains: list(words()) } : { kind: "mention_spam", max: max() };
    const r = await api("POST", `/guilds/${props.g.id}/automod/rules`, { name: name().trim() || "Правило", action: action(), enabled: true, ...cfg });
    if (r.status === 201 || r.status === 200) {
      setName("");
      setWords("");
      void refetch();
    } else toast(errText(r, "Не удалось создать правило"), "error");
  };
  const KIND_LABEL = { keyword: "Запрещённые слова", mention_spam: "Спам упоминаниями", link: "Ссылки только на разрешённые сайты" } as const;
  const ACTION_LABEL = { block: "Не пропускать сообщение", timeout: "Не пропускать и тайм-аут 10 минут", alert: "Только записать в журнал" } as const;
  return (
    <Page title="Автомодерация" hint="Правила проверяются сервером до того, как сообщение увидят другие.">
      <For each={rules() ?? []} fallback={<p class="edited">Правил пока нет.</p>}>
        {(r) => (
          <div class="rule-card">
            <div style={{ flex: 1 }}>
              <b>{r.name}</b>
              <small class="edited">
                {KIND_LABEL[r.config.kind]}: {r.config.kind === "mention_spam" ? `больше ${r.config.max} упоминаний` : (r.config.keywords ?? r.config.allowed_domains ?? []).join(", ") || "—"} · {ACTION_LABEL[r.action]}
              </small>
            </div>
            <Switch
              label="Включено"
              checked={r.enabled}
              onChange={async (on) => {
                await api("PATCH", `/guilds/${props.g.id}/automod/rules/${r.id}`, { enabled: on });
                void refetch();
              }}
            />
            <button
              class="ns-icon-btn"
              aria-label={`Удалить правило ${r.name}`}
              onClick={async () => {
                await api("DELETE", `/guilds/${props.g.id}/automod/rules/${r.id}`);
                void refetch();
              }}
            >
              <Trash2 size={16} />
            </button>
          </div>
        )}
      </For>
      <h2 class="settings-h2">Новое правило</h2>
      <div class="pe-chips" role="radiogroup" aria-label="Тип правила">
        <For each={Object.entries(KIND_LABEL) as Array<[RuleJson["config"]["kind"], string]>}>
          {([k, l]) => (
            <button role="radio" class="pe-chip" aria-checked={kind() === k} onClick={() => setKind(k)}>
              {l}
            </button>
          )}
        </For>
      </div>
      <TextField label="Название правила" value={name()} onInput={setName} />
      <Show when={kind() !== "mention_spam"} fallback={<TextField label="Сколько упоминаний в сообщении можно" value={String(max())} onInput={(v) => setMax(Math.max(1, Number(v) || 1))} inputmode="numeric" />}>
        <TextArea
          label={kind() === "keyword" ? "Слова через запятую (* — любые символы: спам*)" : "Разрешённые домены через запятую"}
          value={words()}
          onInput={setWords}
          max={2000}
          rows={3}
        />
      </Show>
      <label class="ns-field">
        <span class="ns-label">Что делать</span>
        <select class="ns-select" value={action()} onChange={(e) => setAction(e.currentTarget.value as RuleJson["action"])}>
          <For each={Object.entries(ACTION_LABEL)}>{([k, l]) => <option value={k}>{l}</option>}</For>
        </select>
      </label>
      <div>
        <Button onClick={create}>Добавить правило</Button>
      </div>
    </Page>
  );
}

