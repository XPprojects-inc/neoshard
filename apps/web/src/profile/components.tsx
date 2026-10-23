// Profile building blocks: avatar with frame, styled name with server tag, and
// the profile card used by the popout, the full profile and the editor preview.

import type { User } from "@neoshard/core";
import { Avatar, avatarColor, type StatusName } from "@neoshard/ui";
import { createSignal, For, Show, type JSX } from "solid-js";

import { mentionResolver, STATUS_LABEL } from "../actions";
import { BadgeCheck, ExternalLink, Link } from "../icons";
import { Markdown } from "../markdown";
import { api, asset, sid, state, type Id } from "../state";
import { Decoration, hex, nameStyle, ProfileEffect, TagBadge } from "./cosmetics";
import type { ProfileView } from "./data";

export function TagChip(props: { tag: string; badge: string }) {
  return (
    <span class="tag-chip" title={`Тег сервера ${props.tag}`}>
      <TagBadge id={props.badge} size={11} />
      {props.tag}
    </span>
  );
}

/** Avatar of a user with their frame; the per-server avatar wins when given. */
export function UserAvatar(props: { user?: User; name?: string; size?: number; status?: StatusName; memberAvatar?: string; decoration?: string; src?: string; noDeco?: boolean }) {
  const deco = () => (props.noDeco ? "" : (props.decoration ?? props.user?.avatarDecoration ?? ""));
  return (
    <Avatar
      name={props.name ?? (props.user?.displayName || props.user?.username || "?")}
      seed={props.user ? sid(props.user.id) : undefined}
      src={props.src ?? asset(props.memberAvatar || props.user?.avatar)}
      size={props.size}
      status={props.status}
      decoration={deco() ? <Decoration id={deco()} /> : undefined}
    />
  );
}

/** Display name in the user's style (falls back to the role colour), plus the server tag. */
export function UserName(props: { user?: User; text?: string; color?: string; tag?: boolean; class?: string }) {
  const style = () => nameStyle(props.user?.nameStyle ? { font: props.user.nameStyle.font, effect: props.user.nameStyle.effect, colors: [...props.user.nameStyle.colors] } : undefined);
  const text = () => props.text ?? (props.user?.displayName || props.user?.username || "удалённый пользователь");
  return (
    <>
      <span class={`uname ${style().class} ${props.class ?? ""}`} style={{ ...(props.color && !style().class ? { color: props.color } : {}), ...style().style }}>
        {text()}
      </span>
      <Show when={props.tag !== false && props.user?.guildTag?.tag}>
        <TagChip tag={props.user!.guildTag!.tag} badge={props.user!.guildTag!.badge} />
      </Show>
    </>
  );
}

/** CSS url() for a server-provided http(s) URL, with quotes and escapes neutralised. */
export function cssUrl(u: string): string {
  const safe = /^(https?:|blob:)/.test(u) && !/[\s"'\\()]/.test(u) ? u : "about:invalid";
  return `url("${safe}")`;
}

const grow = (t: HTMLTextAreaElement) => {
  t.style.height = "auto";
  t.style.height = `${t.scrollHeight}px`;
};

function ShardMark() {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" aria-hidden="true" style={{ "vertical-align": "-2px" }}>
      <path d="M12 2l7 6-3 14H8L5 8z" fill="var(--ns-accent)" />
    </svg>
  );
}

const fmtDay = new Intl.DateTimeFormat("ru-RU", { day: "numeric", month: "long", year: "numeric" });

export function ProfileCard(props: {
  view: ProfileView;
  status?: StatusName;
  custom?: { text: string; emoji: string };
  full?: boolean;
  /** Editor preview: no network actions. */
  preview?: boolean;
  actions?: JSX.Element;
  onMessage?: (text: string) => void;
  onOpenFull?: () => void;
}) {
  const v = () => props.view;
  const isMe = () => v().userId === sid(state.me?.id ?? 0n);
  // No banner image or colour: a tint of the avatar colour, like the avatar placeholder.
  const banner = () =>
    v().banner ? `center / cover no-repeat ${cssUrl(v().banner!)}` : v().bannerColor !== undefined ? hex(v().bannerColor!) : v().theme ? hex(v().theme![0]) : `color-mix(in srgb, ${avatarColor(v().userId)} 75%, #000)`;
  const [tab, setTab] = createSignal<"about" | "mutual" | "links">("about");
  const [note, setNote] = createSignal(v().note);
  const [msg, setMsg] = createSignal("");
  const saveNote = async () => {
    if (props.preview || note() === v().note) return;
    await api("PUT", `/users/${v().userId}/note`, { note: note() });
  };
  const guild = () => (v().guildId ? state.guilds[v().guildId!] : undefined);
  const roles = () => (guild()?.roles ?? []).filter((r) => v().roleIds.includes(sid(r.id)) && sid(r.id) !== v().guildId).sort((a, b) => b.position - a.position);
  const user = (): User | undefined => state.users[v().userId];
  const nameStyleView = () => nameStyle(v().nameStyle);
  const sections = () => (
    <>
      <Show when={v().bio}>
        <section class="pcard-section">
          <h3>Обо мне</h3>
          <div class="pcard-bio">
            <Markdown source={v().bio} resolver={mentionResolver} />
          </div>
        </section>
      </Show>
      <section class="pcard-section">
        <h3>Участник с</h3>
        <div class="pcard-since">
          <Show when={v().createdAt}>
            <span title="В NeoShard с">
              <ShardMark /> {fmtDay.format(v().createdAt)}
            </span>
          </Show>
          <Show when={guild() && v().joinedAt}>
            <span class="pcard-since-sep" aria-hidden="true" />
            <span title={`На сервере «${guild()!.name}» с`}>
              <span class="guild-mini tiny">{guild()!.name.slice(0, 1)}</span> {fmtDay.format(v().joinedAt)}
            </span>
          </Show>
        </div>
      </section>
      <Show when={roles().length}>
        <section class="pcard-section">
          <h3>Роли</h3>
          <div class="pcard-roles">
            <For each={roles()}>
              {(r) => (
                <span class="role-chip" style={{ "--rc": r.color ? hex(r.color) : undefined }}>
                  <i />
                  {r.name}
                </span>
              )}
            </For>
          </div>
        </section>
      </Show>
      <Show when={v().links.length && !props.full}>
        <LinksSection links={v().links} />
      </Show>
      <Show when={!isMe() && !props.preview}>
        <section class="pcard-section">
          <h3>Заметка</h3>
          <textarea
            class="pcard-note"
            rows={1}
            maxLength={256}
            placeholder="Добавить заметку — видна только вам"
            aria-label="Заметка о пользователе"
            value={note()}
            ref={(t) => queueMicrotask(() => grow(t))}
            onInput={(e) => (setNote(e.currentTarget.value), grow(e.currentTarget))}
            onBlur={saveNote}
          />
        </section>
      </Show>
    </>
  );
  return (
    <div
      class="pcard"
      classList={{ themed: !!v().theme, full: !!props.full }}
      style={v().theme ? { "--pc-1": hex(v().theme![0]), "--pc-2": hex(v().theme![1]) } : undefined}
    >
      <div class="pcard-inner">
        <ProfileEffect id={v().effect} />
        <div class="pcard-banner" style={{ "--pc-banner": banner() }} />
        <Show when={props.actions}>
          <div class="pcard-actions">{props.actions}</div>
        </Show>
        <div class="pcard-av">
          <Avatar
            name={v().displayName || v().username}
            seed={v().userId}
            src={v().avatar}
            size={props.full ? 120 : 80}
            status={props.status}
            decoration={v().decoration ? <Decoration id={v().decoration} /> : undefined}
          />
        </div>
        <Show when={props.custom}>
          <div class="pcard-status" style={props.full ? { left: "160px" } : undefined}>
            <Show when={props.custom!.emoji}>
              <span aria-hidden="true">{props.custom!.emoji} </span>
            </Show>
            {props.custom!.text}
          </div>
        </Show>
        <div class="pcard-body" style={props.full ? { "padding-top": "72px" } : undefined}>
          <div class="pcard-names">
            <h2>
              <span class={nameStyleView().class} style={nameStyleView().style}>
                {v().displayName || v().username}
              </span>
              <Show when={v().tag}>
                <TagChip tag={v().tag!.tag} badge={v().tag!.badge} />
              </Show>
            </h2>
            <div class="sub">
              <span>@{v().username}</span>
              <Show when={v().pronouns}>
                <span>· {v().pronouns}</span>
              </Show>
              <Show when={v().bot}>
                <span class="tag-chip">БОТ</span>
              </Show>
              <Show when={props.status && !props.custom}>
                <span>· {STATUS_LABEL[props.status!]}</span>
              </Show>
            </div>
            <Show when={v().serverProfile && guild()}>
              <small class="edited">Профиль на сервере «{guild()!.name}»</small>
            </Show>
          </div>
          <Show
            when={props.full}
            fallback={
              <div class="pcard-box">
                {sections()}
                <Show when={!props.preview && props.onOpenFull}>
                  <button class="ns-btn ns-btn--ghost ns-btn--sm" style={{ "justify-self": "start" }} onClick={props.onOpenFull}>
                    Открыть полный профиль
                  </button>
                </Show>
              </div>
            }
          >
            <div class="pcard-tabs" role="tablist">
              <button role="tab" aria-selected={tab() === "about"} onClick={() => setTab("about")}>Обо мне</button>
              <Show when={!isMe()}>
                <button role="tab" aria-selected={tab() === "mutual"} onClick={() => setTab("mutual")}>Общие серверы — {v().mutualGuildIds.length}</button>
              </Show>
              <Show when={v().links.length}>
                <button role="tab" aria-selected={tab() === "links"} onClick={() => setTab("links")}>Ссылки</button>
              </Show>
            </div>
            <div class="pcard-box" role="tabpanel">
              <Show when={tab() === "about"}>{sections()}</Show>
              <Show when={tab() === "mutual"}>
                <For each={v().mutualGuildIds} fallback={<span class="edited">Общих серверов нет</span>}>
                  {(gid) => (
                    <div class="mutual-row">
                      <span class="guild-mini">
                        <Show when={asset(state.guilds[gid]?.icon)} fallback={(state.guilds[gid]?.name ?? "?").slice(0, 2)}>
                          <img src={asset(state.guilds[gid]?.icon)} alt="" />
                        </Show>
                      </span>
                      {state.guilds[gid]?.name ?? "Сервер"}
                    </div>
                  )}
                </For>
              </Show>
              <Show when={tab() === "links"}>
                <LinksSection links={v().links} />
              </Show>
            </div>
          </Show>
          <Show when={!isMe() && !props.preview && props.onMessage && !user()?.bot}>
            <input
              class="pcard-dm"
              placeholder={`Написать @${v().username}`}
              aria-label={`Сообщение для ${v().displayName || v().username}`}
              value={msg()}
              onInput={(e) => setMsg(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && msg().trim()) {
                  props.onMessage!(msg().trim());
                  setMsg("");
                }
              }}
            />
          </Show>
        </div>
      </div>
    </div>
  );
}

function LinksSection(props: { links: ProfileView["links"] }) {
  return (
    <section class="pcard-section">
      <h3>Ссылки</h3>
      <div class="pcard-links">
        <For each={props.links}>
          {(l) => (
            <a href={l.url} target="_blank" rel="noopener noreferrer nofollow me">
              <Link size={16} aria-hidden="true" />
              <span style={{ flex: 1 }}>{l.label}</span>
              <Show when={l.verified}>
                <span class="verified" title="Подтверждено: страница ссылается на этот профиль">
                  <BadgeCheck size={16} aria-label="подтверждено" />
                </span>
              </Show>
              <ExternalLink size={14} aria-hidden="true" />
            </a>
          )}
        </For>
      </div>
    </section>
  );
}

