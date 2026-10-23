// Profile editor (Settings → Profiles): the user profile and per-server
// profiles, with a live preview. Every cosmetic is free.

import type { User } from "@neoshard/core";
import { Avatar, Button, ColorField, Switch, TextArea, TextField, toast } from "@neoshard/ui";
import { createEffect, createMemo, createResource, createSignal, For, on, Show, type JSX } from "solid-js";
import { createStore, reconcile, unwrap } from "solid-js/store";

import { activeCustom, statusName } from "../actions";
import { BadgeCheck, ImagePlus, Trash2 } from "../icons";
import { api, asset, sid, state, type Id } from "../state";
import { ProfileCard } from "./components";
import { DECORATIONS, Decoration, EFFECTS, hex, NAME_EFFECTS, NAME_FONTS, NAMEPLATES, nameStyle, ProfileEffect, TagBadge } from "./cosmetics";
import { fetchProfile, invalidateProfile, uploadImage, viewFromJson, type ProfileJson, type ProfileView } from "./data";

interface MainDraft {
  display_name: string;
  pronouns: string;
  bio: string;
  avatar: string;
  banner: string;
  banner_color: number | null;
  theme: [number, number] | null;
  avatar_decoration: string;
  profile_effect: string;
  nameplate: string;
  font: string;
  effect: string;
  colors: number[];
  primary_guild_id: string;
}

function draftFrom(p: ProfileJson | undefined, u: User | undefined): MainDraft {
  return {
    display_name: u?.displayName ?? "",
    pronouns: p?.pronouns ?? "",
    bio: p?.bio ?? "",
    avatar: u?.avatar ?? "",
    banner: p?.banner ?? "",
    banner_color: p?.banner_color ?? null,
    theme: p?.theme_colors?.length === 2 ? [p.theme_colors[0], p.theme_colors[1]] : null,
    avatar_decoration: u?.avatarDecoration ?? "",
    profile_effect: p?.profile_effect ?? "",
    nameplate: u?.nameplate ?? "",
    font: u?.nameStyle?.font ?? "",
    effect: u?.nameStyle?.effect ?? "",
    colors: u?.nameStyle ? [...u.nameStyle.colors] : [],
    primary_guild_id: u?.guildTag ? sid(u.guildTag.guildId) : "0",
  };
}

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

export function ProfileEditor() {
  const [tab, setTab] = createSignal<"main" | "server">("main");
  return (
    <div class="settings-page">
      <h1>Профили</h1>
      <div class="pcard-tabs" role="tablist" style={{ "margin-bottom": "20px" }}>
        <button role="tab" aria-selected={tab() === "main"} onClick={() => setTab("main")}>Основной профиль</button>
        <button role="tab" aria-selected={tab() === "server"} onClick={() => setTab("server")}>Профили на серверах</button>
      </div>
      <Show when={tab() === "main"} fallback={<ServerProfileEditor />}>
        <MainProfileEditor />
      </Show>
    </div>
  );
}

function Section(props: { title: string; hint?: string; children: JSX.Element }) {
  return (
    <section class="pe-section">
      <h3>{props.title}</h3>
      <Show when={props.hint}>
        <p class="pe-hint">{props.hint}</p>
      </Show>
      {props.children}
    </section>
  );
}

function ImagePicker(props: { label: string; has: boolean; endpoint: string; onKey: (k: string) => void; onRemove: () => void }) {
  let input!: HTMLInputElement;
  const [busy, setBusy] = createSignal(false);
  return (
    <div class="pe-row">
      <input
        ref={input}
        type="file"
        accept="image/png,image/jpeg,image/webp,image/gif"
        hidden
        onChange={async (e) => {
          const f = e.currentTarget.files?.[0];
          e.currentTarget.value = "";
          if (!f) return;
          setBusy(true);
          try {
            props.onKey(await uploadImage(props.endpoint, f));
          } catch (err) {
            toast(String((err as Error).message ?? err), "error");
          } finally {
            setBusy(false);
          }
        }}
      />
      <Button size="sm" loading={busy()} onClick={() => input.click()}>
        <ImagePlus size={16} /> {props.label}
      </Button>
      <Show when={props.has}>
        <Button size="sm" variant="ghost" onClick={props.onRemove}>
          <Trash2 size={16} /> Удалить
        </Button>
      </Show>
    </div>
  );
}

/** Grid of catalog tiles with a "none" option; the preview is drawn per tile. */
function Tiles<T extends { id: string; label: string }>(props: { items: T[]; value: string; onPick: (id: string) => void; render: (id: string) => JSX.Element; label: string }) {
  return (
    <div class="pe-tiles" role="radiogroup" aria-label={props.label}>
      <button role="radio" aria-checked={props.value === ""} class="pe-tile" onClick={() => props.onPick("")}>
        <span class="pe-tile-art pe-none">∅</span>
        <span>Нет</span>
      </button>
      <For each={props.items}>
        {(it) => (
          <button role="radio" aria-checked={props.value === it.id} class="pe-tile" onClick={() => props.onPick(it.id)}>
            <span class="pe-tile-art">{props.render(it.id)}</span>
            <span>{it.label}</span>
          </button>
        )}
      </For>
    </div>
  );
}

function MainProfileEditor() {
  const me = () => state.me;
  const [profile, { refetch }] = createResource(() => sid(state.me?.id ?? 0n), (id) => fetchProfile(id, undefined, true));
  const [saved, setSaved] = createStore<MainDraft>(draftFrom(undefined, undefined));
  const [d, setD] = createStore<MainDraft>(draftFrom(undefined, undefined));
  createEffect(
    on(profile, (p) => {
      if (!p) return;
      const fresh = draftFrom(p, me());
      setSaved(reconcile(fresh));
      setD(reconcile(structuredClone(fresh)));
    }),
  );
  // Reading through the store proxies keeps this reactive (unwrap would not track).
  const dirty = () => JSON.stringify(d) !== JSON.stringify(saved);
  const [busy, setBusy] = createSignal(false);
  const save = async () => {
    setBusy(true);
    const body: Record<string, unknown> = {};
    const s = unwrap(saved);
    const k = unwrap(d);
    if (k.display_name !== s.display_name) body.display_name = k.display_name;
    if (k.pronouns !== s.pronouns) body.pronouns = k.pronouns;
    if (k.bio !== s.bio) body.bio = k.bio;
    if (k.avatar !== s.avatar) body.avatar = k.avatar;
    if (k.banner !== s.banner) body.banner = k.banner;
    if (k.banner_color !== s.banner_color) body.banner_color = k.banner_color ?? -1;
    if (!same(k.theme, s.theme)) body.theme_colors = k.theme ?? [];
    if (k.avatar_decoration !== s.avatar_decoration) body.avatar_decoration = k.avatar_decoration;
    if (k.profile_effect !== s.profile_effect) body.profile_effect = k.profile_effect;
    if (k.nameplate !== s.nameplate) body.nameplate = k.nameplate;
    if (k.font !== s.font || k.effect !== s.effect || !same(k.colors, s.colors)) body.name_style = { font: k.font, effect: k.effect, colors: k.colors };
    if (k.primary_guild_id !== s.primary_guild_id) body.primary_guild_id = k.primary_guild_id;
    const r = await api<{ message?: string }>("PATCH", "/users/@me", body);
    setBusy(false);
    if (r.status === 200) {
      invalidateProfile(sid(state.me!.id));
      toast("Профиль сохранён");
      void refetch();
    } else toast(r.body?.message ?? "Не удалось сохранить профиль", "error");
  };
  const previewView = createMemo<ProfileView | undefined>(() => {
    const p = profile();
    const u = me();
    if (!u) return undefined;
    const v = viewFromJson(p ?? { user: { id: sid(u.id), username: u.username } }, u);
    const tagGuild = d.primary_guild_id !== "0" ? state.guilds[d.primary_guild_id] : undefined;
    return {
      ...v,
      displayName: d.display_name || u.username,
      pronouns: d.pronouns,
      bio: d.bio,
      avatar: asset(d.avatar),
      banner: asset(d.banner),
      bannerColor: d.banner_color ?? undefined,
      theme: d.theme ?? undefined,
      effect: d.profile_effect,
      decoration: d.avatar_decoration,
      nameStyle: d.font || d.effect || d.colors.length ? { font: d.font, effect: d.effect, colors: [...d.colors] } : undefined,
      tag: tagGuild?.tag ? { tag: tagGuild.tag, badge: tagGuild.tagBadge, guildId: tagGuild.id } : undefined,
      guildId: undefined,
      roleIds: [],
    };
  });
  const taggedGuilds = () => state.guildOrder.map((id) => state.guilds[id]).filter((g) => g?.tag);
  const ns = () => nameStyle(d.font || d.effect || d.colors.length ? { font: d.font, effect: d.effect, colors: d.colors } : undefined);
  return (
    <div class="pe-grid">
      <div class="pe-form">
        <section class="pe-section">
          <TextField label="Отображаемое имя" value={d.display_name} onInput={(v) => setD("display_name", v.slice(0, 32))} />
          <TextField label="Местоимения (например: он/его, она/её)" value={d.pronouns} onInput={(v) => setD("pronouns", v.slice(0, 40))} />
        </section>
        <Section title="Аватар">
          <ImagePicker label="Сменить аватар" has={!!d.avatar} endpoint="/uploads/avatar" onKey={(k) => setD("avatar", k)} onRemove={() => setD("avatar", "")} />
        </Section>
        <Section title="Рамка аватара" hint="Рамка видна в чате, списке участников и профиле.">
          <Tiles
            label="Рамка аватара"
            items={DECORATIONS}
            value={d.avatar_decoration}
            onPick={(id) => setD("avatar_decoration", id)}
            render={(id) => <Avatar name={me()?.username ?? "?"} seed={sid(me()?.id ?? 0n)} src={asset(d.avatar)} size={40} decoration={<Decoration id={id} />} />}
          />
        </Section>
        <Section title="Эффект профиля" hint="Анимация поверх карточки профиля. Отключается, если в системе включено «уменьшение движения».">
          <Tiles
            label="Эффект профиля"
            items={EFFECTS}
            value={d.profile_effect}
            onPick={(id) => setD("profile_effect", id)}
            render={(id) => (
              <span class="pe-fx-mini">
                <ProfileEffect id={id} />
              </span>
            )}
          />
        </Section>
        <Section title="Баннер">
          <ImagePicker label="Сменить баннер" has={!!d.banner} endpoint="/uploads/banner" onKey={(k) => setD("banner", k)} onRemove={() => setD("banner", "")} />
          <div class="pe-row">
            <ColorField label="Цвет баннера" value={d.banner_color ?? 0x2ec4a6} onInput={(v) => setD("banner_color", v)} />
            <Show when={d.banner_color !== null}>
              <Button size="sm" variant="ghost" onClick={() => setD("banner_color", null)}>Сбросить цвет</Button>
            </Show>
          </div>
        </Section>
        <Section title="Тема профиля" hint="Два цвета рамки карточки профиля.">
          <Switch label="Двухцветная тема" checked={!!d.theme} onChange={(on) => setD("theme", on ? [0x1b2a4a, 0x7b5ea7] : null)} />
          <Show when={d.theme}>
            <div class="pe-row">
              <ColorField label="Основной" value={d.theme![0]} onInput={(v) => setD("theme", [v, d.theme![1]])} />
              <ColorField label="Акцент" value={d.theme![1]} onInput={(v) => setD("theme", [d.theme![0], v])} />
            </div>
          </Show>
        </Section>
        <Section title="Обо мне">
          <TextArea label="Расскажите о себе" hint="Поддерживается разметка" value={d.bio} onInput={(v) => setD("bio", v)} max={190} />
        </Section>
        <Section title="Стиль имени" hint="Шрифт и эффект вашего отображаемого имени в чате и профиле.">
          <div class="pe-chips" role="radiogroup" aria-label="Шрифт имени">
            <For each={NAME_FONTS}>
              {(f) => (
                <button role="radio" class="pe-chip" aria-checked={(d.font || "sans") === f.id} onClick={() => setD("font", f.id === "sans" ? "" : f.id)} style={nameStyle({ font: f.id, effect: "", colors: [] }).style}>
                  {f.label}
                </button>
              )}
            </For>
          </div>
          <div class="pe-chips" role="radiogroup" aria-label="Эффект имени">
            <button role="radio" class="pe-chip" aria-checked={d.effect === "" && d.colors.length === 0} onClick={() => (setD("effect", ""), setD("colors", []))}>
              Без эффекта
            </button>
            <For each={NAME_EFFECTS}>
              {(f) => (
                <button
                  role="radio"
                  class="pe-chip"
                  aria-checked={d.effect === f.id}
                  onClick={() => {
                    setD("effect", f.id);
                    if (d.colors.length < 2) setD("colors", [d.colors[0] ?? 0x2ec4a6, 0x9d7bff]);
                  }}
                >
                  <span {...nameStyle({ font: "", effect: f.id, colors: d.colors.length ? d.colors : [0x2ec4a6, 0x9d7bff] })}>{f.label}</span>
                </button>
              )}
            </For>
          </div>
          <Show when={d.effect || d.colors.length}>
            <div class="pe-row">
              <ColorField label="Цвет 1" value={d.colors[0] ?? 0x2ec4a6} onInput={(v) => setD("colors", [v, d.colors[1] ?? v])} />
              <ColorField label="Цвет 2" value={d.colors[1] ?? 0x9d7bff} onInput={(v) => setD("colors", [d.colors[0] ?? 0x2ec4a6, v])} />
            </div>
          </Show>
        </Section>
        <Section title="Табличка" hint="Фон под вашим именем в списке участников и личных сообщениях.">
          <Tiles
            label="Табличка"
            items={NAMEPLATES}
            value={d.nameplate}
            onPick={(id) => setD("nameplate", id)}
            render={(id) => <span class={`pe-np np np-${id}`}>{d.display_name || me()?.username}</span>}
          />
        </Section>
        <Section title="Тег сервера" hint="Короткий тег сервера рядом с вашим именем. Доступны серверы, где задан тег.">
          <select class="ns-select" aria-label="Тег сервера" value={d.primary_guild_id} onChange={(e) => setD("primary_guild_id", e.currentTarget.value)}>
            <option value="0">Не показывать</option>
            <For each={taggedGuilds()}>{(g) => <option value={g!.id}>{g!.tag} — {g!.name}</option>}</For>
          </select>
          <Show when={!taggedGuilds().length}>
            <p class="pe-hint">Ни на одном из ваших серверов пока нет тега. Владелец задаёт его в настройках сервера.</p>
          </Show>
        </Section>
        <LinksEditor username={me()?.username ?? ""} links={profile()?.links ?? []} />
      </div>
      <aside class="pe-preview" aria-label="Предпросмотр">
        <h3>Предпросмотр</h3>
        <Show when={previewView()}>
          <ProfileCard view={previewView()!} preview status={statusName(state.myPresence.status)} custom={activeCustom(state.myPresence)} />
        </Show>
        <h3>В списке участников</h3>
        <div class={`member pe-member-preview ${d.nameplate ? `np np-${d.nameplate}` : ""}`}>
          <Avatar name={me()?.username ?? "?"} seed={sid(me()?.id ?? 0n)} src={asset(d.avatar)} size={32} status={statusName(state.myPresence.status)} decoration={d.avatar_decoration ? <Decoration id={d.avatar_decoration} /> : undefined} />
          <div class="member-name">
            <span class={ns().class} style={ns().style}>{d.display_name || me()?.username}</span>
            <Show when={previewView()?.tag}>
              <span class="tag-chip">
                <TagBadge id={previewView()!.tag!.badge} size={11} />
                {previewView()!.tag!.tag}
              </span>
            </Show>
          </div>
        </div>
      </aside>
      <SaveBar dirty={dirty()} busy={busy()} onReset={() => setD(reconcile(structuredClone(unwrap(saved))))} onSave={save} />
    </div>
  );
}

function SaveBar(props: { dirty: boolean; busy: boolean; onReset: () => void; onSave: () => void }) {
  return (
    <Show when={props.dirty}>
      <div class="pe-savebar" role="region" aria-label="Несохранённые изменения">
        <span>Есть несохранённые изменения</span>
        <Button variant="ghost" size="sm" onClick={props.onReset}>Сбросить</Button>
        <Button size="sm" loading={props.busy} onClick={props.onSave}>Сохранить изменения</Button>
      </div>
    </Show>
  );
}

function LinksEditor(props: { username: string; links: Array<{ label: string; url: string; verified?: boolean }> }) {
  const [rows, setRows] = createStore<Array<{ label: string; url: string; verified: boolean }>>([]);
  createEffect(on(() => props.links, (l) => setRows(reconcile(l.map((x) => ({ label: x.label, url: x.url, verified: !!x.verified }))))));
  const [busy, setBusy] = createSignal(false);
  const profileUrl = () => `${location.origin}/u/${props.username}`;
  const save = async (verify: boolean) => {
    setBusy(true);
    const r = await api<Array<{ label: string; url: string; verified?: boolean }> & { message?: string }>("PUT", "/users/@me/links", rows.filter((x) => x.url.trim()).map((x) => ({ label: x.label.trim() || new URL(x.url).hostname, url: x.url.trim() })));
    if (r.status !== 200) {
      setBusy(false);
      toast((r.body as { message?: string })?.message ?? "Проверьте ссылки: нужен адрес http(s)", "error");
      return;
    }
    let list = r.body;
    if (verify) {
      const v = await api<typeof list>("POST", "/users/@me/links/verify", {});
      if (v.status === 200) list = v.body;
    }
    setBusy(false);
    setRows(reconcile(list.map((x) => ({ label: x.label, url: x.url, verified: !!x.verified }))));
    invalidateProfile(sid(state.me?.id ?? 0n));
    toast(verify ? `Подтверждено ссылок: ${list.filter((x) => x.verified).length} из ${list.length}` : "Ссылки сохранены");
  };
  return (
    <Section title="Ссылки" hint="До 6 ссылок на сайты и профили в других сервисах. Ссылка получает галочку, когда на странице есть обратная ссылка на ваш профиль с rel=&quot;me&quot;.">
      <For each={rows}>
        {(row, i) => (
          <div class="pe-link">
            <input class="ns-input" aria-label="Название ссылки" placeholder="Название" maxLength={32} value={row.label} onInput={(e) => setRows(i(), "label", e.currentTarget.value)} />
            <input class="ns-input" aria-label="Адрес" placeholder="https://…" maxLength={512} value={row.url} onInput={(e) => (setRows(i(), "url", e.currentTarget.value), setRows(i(), "verified", false))} />
            <Show when={row.verified}>
              <span class="verified" title="Подтверждено">
                <BadgeCheck size={18} aria-label="подтверждено" />
              </span>
            </Show>
            <button class="ns-icon-btn" aria-label="Удалить ссылку" onClick={() => setRows((r) => r.filter((_, k) => k !== i()))}>
              <Trash2 size={16} />
            </button>
          </div>
        )}
      </For>
      <div class="pe-row">
        <Show when={rows.length < 6}>
          <Button size="sm" variant="secondary" onClick={() => setRows(rows.length, { label: "", url: "", verified: false })}>Добавить ссылку</Button>
        </Show>
        <Button size="sm" loading={busy()} onClick={() => save(false)}>Сохранить ссылки</Button>
        <Button size="sm" variant="ghost" loading={busy()} onClick={() => save(true)}>Проверить</Button>
      </div>
      <p class="pe-hint">
        Для подтверждения добавьте на свою страницу: <code class="md-code">{`<a rel="me" href="${profileUrl()}">NeoShard</a>`}</code>
      </p>
    </Section>
  );
}

interface ServerDraft {
  nick: string;
  pronouns: string;
  bio: string;
  avatar: string;
  banner: string;
  theme: [number, number] | null;
}

function ServerProfileEditor() {
  const [gid, setGid] = createSignal<Id | undefined>(state.selection.guild !== "@me" ? state.selection.guild : state.guildOrder[0]);
  const [profile, { refetch }] = createResource(() => (gid() ? { g: gid()!, u: sid(state.me?.id ?? 0n) } : undefined), (k) => fetchProfile(k.u, k.g, true));
  const blank: ServerDraft = { nick: "", pronouns: "", bio: "", avatar: "", banner: "", theme: null };
  const [saved, setSaved] = createStore<ServerDraft>({ ...blank });
  const [d, setD] = createStore<ServerDraft>({ ...blank });
  createEffect(
    on(profile, (p) => {
      const m = p?.member;
      const fresh: ServerDraft = { nick: m?.nick ?? "", pronouns: m?.pronouns ?? "", bio: m?.bio ?? "", avatar: m?.avatar ?? "", banner: m?.banner ?? "", theme: m?.theme_colors?.length === 2 ? [m.theme_colors[0], m.theme_colors[1]] : null };
      setSaved(reconcile(fresh));
      setD(reconcile(structuredClone(fresh)));
    }),
  );
  // Reading through the store proxies keeps this reactive (unwrap would not track).
  const dirty = () => JSON.stringify(d) !== JSON.stringify(saved);
  const [busy, setBusy] = createSignal(false);
  const save = async () => {
    const g = gid();
    if (!g) return;
    setBusy(true);
    const s = unwrap(saved);
    const k = unwrap(d);
    const body: Record<string, unknown> = {};
    if (k.nick !== s.nick) body.nick = k.nick;
    if (k.pronouns !== s.pronouns) body.pronouns = k.pronouns;
    if (k.bio !== s.bio) body.bio = k.bio;
    if (k.avatar !== s.avatar) body.avatar = k.avatar;
    if (k.banner !== s.banner) body.banner = k.banner;
    if (!same(k.theme, s.theme)) body.theme_colors = k.theme ?? [];
    const r = await api<{ message?: string }>("PATCH", `/guilds/${g}/members/@me`, body);
    setBusy(false);
    if (r.status === 200) {
      invalidateProfile(sid(state.me!.id));
      toast("Профиль на сервере сохранён");
      void refetch();
    } else toast(r.status === 403 ? "На этом сервере вам нельзя менять никнейм" : (r.body?.message ?? "Не удалось сохранить"), "error");
  };
  const preview = createMemo<ProfileView | undefined>(() => {
    const p = profile();
    const u = state.me;
    if (!u || !p) return undefined;
    const v = viewFromJson(p, u);
    return {
      ...v,
      displayName: d.nick || u.displayName || u.username,
      pronouns: d.pronouns || p.pronouns || "",
      bio: d.bio || p.bio || "",
      avatar: asset(d.avatar || u.avatar),
      banner: asset(d.banner || p.banner),
      theme: d.theme ?? (p.theme_colors?.length === 2 ? [p.theme_colors[0], p.theme_colors[1]] : undefined),
      serverProfile: true,
    };
  });
  return (
    <div class="pe-grid">
      <div class="pe-form">
        <p class="pe-hint">Профиль на сервере показывается только участникам этого сервера. Пустые поля берутся из основного профиля.</p>
        <label class="ns-field">
          <span class="ns-label">Сервер</span>
          <select class="ns-select" value={gid() ?? ""} onChange={(e) => setGid(e.currentTarget.value)}>
            <For each={state.guildOrder}>{(id) => <option value={id}>{state.guilds[id]?.name}</option>}</For>
          </select>
        </label>
        <Show when={gid()} fallback={<p class="pe-hint">Вы пока не состоите ни в одном сервере.</p>}>
          <section class="pe-section">
            <TextField label="Никнейм на сервере" value={d.nick} onInput={(v) => setD("nick", v.slice(0, 32))} />
            <TextField label="Местоимения на сервере" value={d.pronouns} onInput={(v) => setD("pronouns", v.slice(0, 40))} />
          </section>
          <Section title="Аватар на сервере">
            <ImagePicker label="Сменить аватар" has={!!d.avatar} endpoint={`/guilds/${gid()}/uploads/member-avatar`} onKey={(k) => setD("avatar", k)} onRemove={() => setD("avatar", "")} />
          </Section>
          <Section title="Баннер на сервере">
            <ImagePicker label="Сменить баннер" has={!!d.banner} endpoint={`/guilds/${gid()}/uploads/member-banner`} onKey={(k) => setD("banner", k)} onRemove={() => setD("banner", "")} />
          </Section>
          <Section title="Тема профиля на сервере">
            <Switch label="Своя тема на этом сервере" checked={!!d.theme} onChange={(on) => setD("theme", on ? [0x10324a, 0x2ec4a6] : null)} />
            <Show when={d.theme}>
              <div class="pe-row">
                <ColorField label="Основной" value={d.theme![0]} onInput={(v) => setD("theme", [v, d.theme![1]])} />
                <ColorField label="Акцент" value={d.theme![1]} onInput={(v) => setD("theme", [d.theme![0], v])} />
              </div>
            </Show>
          </Section>
          <Section title="Обо мне на сервере">
            <TextArea label="О себе для этого сервера" value={d.bio} onInput={(v) => setD("bio", v)} max={190} />
          </Section>
        </Show>
      </div>
      <aside class="pe-preview" aria-label="Предпросмотр">
        <h3>Предпросмотр</h3>
        <Show when={preview()}>
          <ProfileCard view={preview()!} preview status={statusName(state.myPresence.status)} custom={activeCustom(state.myPresence)} />
        </Show>
      </aside>
      <SaveBar dirty={dirty()} busy={busy()} onReset={() => setD(reconcile(structuredClone(unwrap(saved))))} onSave={save} />
    </div>
  );
}


