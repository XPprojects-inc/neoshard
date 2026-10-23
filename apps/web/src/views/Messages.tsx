import { snowflakeTime } from "../time";
import type { Channel } from "@neoshard/core";
import { IconButton, toast, Tooltip, type MenuItem } from "@neoshard/ui";
import { createStore } from "solid-js/store";
import { createVirtualizer } from "@tanstack/solid-virtual";
import { CirclePlus, Copy, Pencil, Pin, Reply, RotateCcw, Smile, SmilePlus, Trash2, X } from "../icons";
import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from "solid-js";

/** Only http(s) URLs from server data may become links or media sources. */
export function safeUrl(u: string): string | undefined {
  try {
    const p = new URL(u, location.origin);
    return p.protocol === "https:" || p.protocol === "http:" ? p.href : undefined;
  } catch {
    return undefined;
  }
}

import { copy, mentionResolver, roleColor } from "../actions";
import { Markdown, plainText, type MentionResolver } from "../markdown";
import { UserAvatar, UserName } from "../profile/components";
import { QUICK_EMOJI } from "../profile/CustomStatus";
import { currentGuild, openProfile } from "../profile/data";
import { api, asset, core, sid, state, type MessageView } from "../state";
import { AttachmentList, EncryptedAttachments, MAX_FILES, uploadAttachment, UploadTray, type UploadItem } from "./Attachments";
import { encryptFile, parseManifest, withManifest } from "../e2eeFiles";
import { clearJump, jumpTarget } from "./Search";
import { openContextMenu, openUserMenu } from "./Shell";

const GROUP_MS = 7 * 60 * 1000;
const QUICK_REACTIONS = ["👍", "❤️", "😂", "🎉", "😮", "😢"];
const fmtTime = new Intl.DateTimeFormat("ru-RU", { hour: "2-digit", minute: "2-digit" });
const fmtDate = new Intl.DateTimeFormat("ru-RU", { day: "numeric", month: "long", year: "numeric", hour: "2-digit", minute: "2-digit" });
const fmtDay = new Intl.DateTimeFormat("ru-RU", { day: "numeric", month: "long", year: "numeric" });

/** The live user record (UserUpdate) wins over the copy embedded in the message. */
function authorOf(m: MessageView) {
  return (m.author && state.users[sid(m.author.id)]) || m.author;
}

function authorName(m: MessageView) {
  const a = authorOf(m);
  return memberOf(m)?.nick || a?.displayName || a?.username || "удалённый пользователь";
}

/** Server member of the author (nick, per-server avatar, roles) when loaded. */
function memberOf(m: MessageView) {
  if (m.guildId === 0n || !m.author) return undefined;
  return state.members[sid(m.guildId)]?.find((x) => x?.user?.id === m.author!.id);
}

const msgTime = (m: MessageView) => (m.id === 0n ? Date.now() : snowflakeTime(m.id));
const dayKey = (t: number) => new Date(t).toDateString();

export function Messages(props: { channel: Channel; canSend: boolean; canManage: boolean; canAttach: boolean }) {
  let addFiles: (files: File[]) => void = () => {};
  const [dragging, setDragging] = createSignal(false);
  const cid = () => sid(props.channel.id);
  const list = createMemo(() => state.messages[cid()] ?? []);
  const [replyTo, setReplyTo] = createSignal<MessageView>();
  const [editing, setEditing] = createSignal<string>();
  let scroller!: HTMLDivElement;
  let startEl: HTMLDivElement | undefined;
  let atBottom = true;
  const [loadingOlder, setLoadingOlder] = createSignal(false);
  // Messages that arrive while the channel is open get an entrance animation.
  const openedAt = Date.now();
  // "New messages" line: the read marker as it was when the channel opened.
  const unreadFrom = state.readStates[cid()]?.last ?? "";
  createEffect(on(() => [list().length, state.reachedStart[cid()]], () => setLoadingOlder(false), { defer: true }));

  const resolver: MentionResolver = mentionResolver;

  const virt = createVirtualizer({
    get count() {
      return list().length;
    },
    getScrollElement: () => scroller,
    // The "start of channel" block sits above the rows inside the scroller.
    get scrollMargin() {
      return startEl?.offsetHeight ?? 0;
    },
    estimateSize: () => 44,
    overscan: 12,
    getItemKey: (i) => list()[i]?.key ?? i,
  });

  const newDay = (i: number) => {
    const prev = list()[i - 1];
    return !prev || dayKey(msgTime(prev)) !== dayKey(msgTime(list()[i]));
  };
  const firstUnread = (i: number) => {
    if (!unreadFrom) return false;
    const m = list()[i];
    const prev = list()[i - 1];
    const last = BigInt(unreadFrom);
    return m.id > last && (!prev || prev.id <= last) && m.author?.id !== state.me?.id && i > 0;
  };
  const grouped = (i: number) => {
    const m = list()[i];
    const prev = list()[i - 1];
    if (!prev || !m.author || !prev.author || prev.author.id !== m.author.id || m.replyToId !== 0n) return false;
    if (newDay(i) || firstUnread(i)) return false;
    return msgTime(m) - msgTime(prev) < GROUP_MS;
  };

  const loadOlder = () => {
    if (loadingOlder() || state.reachedStart[cid()]) return;
    const oldest = list().find((m) => m.id !== 0n);
    setLoadingOlder(true);
    core().send({ case: "loadHistory", value: { channelId: props.channel.id, beforeId: oldest?.id ?? 0n, limit: 50 } });
  };

  createEffect(
    on(cid, () => {
      atBottom = true;
      setReplyTo(undefined);
      if (list().length < 30) core().send({ case: "loadHistory", value: { channelId: props.channel.id, beforeId: 0n, limit: 50 } });
      requestAnimationFrame(() => virt.scrollToIndex(Math.max(0, list().length - 1), { align: "end" }));
    }),
  );

  // Keep the newest message in view; when older ones are prepended, keep the
  // viewport anchored by shifting scrollTop by the height that was added above.
  let prevFirst: string | undefined;
  let prevTotal = 0;
  createEffect(
    on(list, (l) => {
      const first = l[0]?.key;
      const prepended = prevFirst !== undefined && first !== prevFirst && !atBottom;
      if (prepended) {
        const before = prevTotal;
        queueMicrotask(() => {
          scroller.scrollTop += virt.getTotalSize() - before;
        });
      } else if (atBottom && l.length) {
        queueMicrotask(() => virt.scrollToIndex(l.length - 1, { align: "end" }));
      }
      prevFirst = first;
      prevTotal = virt.getTotalSize();
      const last = [...l].reverse().find((m) => m.id !== 0n);
      if (last && atBottom && document.visibilityState === "visible") ack(last.id);
      const newest = l[l.length - 1];
      if (newest && newest.id !== 0n && newest.author?.id !== state.me?.id) setAnnounce(`${authorName(newest)}: ${newest.content.slice(0, 140)}`);
    }),
  );
  const [announce, setAnnounce] = createSignal("");
  // Jump to a message (from search): load the page around it if needed, then highlight.
  const [flash, setFlash] = createSignal<string>();
  let jumpRequested: bigint | undefined;
  createEffect(() => {
    const j = jumpTarget();
    if (!j || j.channel !== cid()) return;
    const i = list().findIndex((m) => m.id === j.id);
    if (i >= 0) {
      atBottom = false;
      requestAnimationFrame(() => virt.scrollToIndex(i, { align: "center" }));
      setFlash(list()[i].key);
      setTimeout(() => setFlash(undefined), 2200);
      jumpRequested = undefined;
      clearJump();
    } else if (jumpRequested !== j.id) {
      jumpRequested = j.id;
      core().send({ case: "loadHistory", value: { channelId: props.channel.id, beforeId: j.id + 1n, limit: 50 } });
    }
  });
  const [showJump, setShowJump] = createSignal(false);
  const onVisible = () => {
    const last = [...list()].reverse().find((m) => m.id !== 0n);
    if (last && atBottom && document.visibilityState === "visible") ack(last.id);
  };
  document.addEventListener("visibilitychange", onVisible);
  onCleanup(() => document.removeEventListener("visibilitychange", onVisible));

  let lastAck = 0n;
  const ack = (id: bigint) => {
    if (id <= lastAck) return;
    lastAck = id;
    const rs = state.readStates[cid()];
    if (rs && BigInt(rs.last) >= id && rs.mentions === 0) return;
    core().send({ case: "ackChannel", value: { channelId: props.channel.id, messageId: id } });
  };

  const onScroll = () => {
    atBottom = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 40;
    setShowJump(scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight > 1200);
    if (scroller.scrollTop < 400) loadOlder();
  };

  const react = (m: MessageView, emoji: string) => {
    const mine = m.reactions.find((r) => r.emoji === emoji)?.me ?? false;
    core().send({ case: "react", value: { channelId: m.channelId, messageId: m.id, emoji, add: !mine } });
  };

  const scrollTo = (id: bigint) => {
    const i = list().findIndex((m) => m.id === id);
    if (i >= 0) virt.scrollToIndex(i, { align: "center" });
  };

  const menuFor = (m: MessageView, onPicker: () => void): MenuItem[] => {
    const mine = m.author?.id === state.me?.id;
    const link = `${location.origin}/#/${m.guildId === 0n ? "@me" : sid(m.guildId)}/${sid(m.channelId)}/${sid(m.id)}`;
    return [
      { label: "Добавить реакцию", icon: <SmilePlus size={16} />, onSelect: onPicker },
      { label: "Ответить", icon: <Reply size={16} />, onSelect: () => setReplyTo(m) },
      ...(mine ? [{ label: "Изменить", icon: <Pencil size={16} />, onSelect: () => setEditing(m.key) }] : []),
      ...(props.canManage && m.guildId !== 0n
        ? [
            {
              label: m.pinned ? "Открепить" : "Закрепить",
              icon: <Pin size={16} />,
              onSelect: () => void api(m.pinned ? "DELETE" : "PUT", `/channels/${sid(m.channelId)}/pins/${sid(m.id)}`, m.pinned ? undefined : {}),
            },
          ]
        : []),
      { separator: true },
      { label: "Копировать текст", icon: <Copy size={16} />, disabled: !m.content, onSelect: () => void copy(m.content, "Текст скопирован") },
      { label: "Копировать ссылку на сообщение", onSelect: () => void copy(link, "Ссылка скопирована") },
      { label: "Копировать ID сообщения", onSelect: () => void copy(sid(m.id), "ID скопирован") },
      ...(mine || props.canManage
        ? [
            { separator: true },
            {
              label: "Удалить сообщение",
              icon: <Trash2 size={16} />,
              danger: true,
              onSelect: () => confirm("Удалить сообщение?") && core().send({ case: "deleteMessage", value: { channelId: m.channelId, messageId: m.id } }),
            },
          ]
        : []),
    ];
  };

  return (
    <>
      {/* New messages are announced separately: the virtualized log itself is not a live region. */}
      <div class="ns-sr-only" aria-live="polite">{announce()}</div>
      <Show when={dragging()}>
        <div class="drop-hint" aria-hidden="true">
          <div>
            <b>Отпустите, чтобы прикрепить</b>
            <span>до {MAX_FILES} файлов, до 25 МБ каждый</span>
          </div>
        </div>
      </Show>
      <div
        class="feed"
        onDragOver={(e) => {
          if (e.dataTransfer?.types.includes("Files")) {
            e.preventDefault();
            setDragging(true);
          }
        }}
        onDragLeave={(e) => e.currentTarget === e.target && setDragging(false)}
        onDrop={(e) => {
          e.preventDefault();
          setDragging(false);
          addFiles([...(e.dataTransfer?.files ?? [])]);
        }}
        ref={scroller}
        onScroll={onScroll} role="log" aria-label={`Сообщения в ${props.channel.name || "беседе"}`} aria-busy={loadingOlder()}>
        <Show when={state.reachedStart[cid()]}>
          <div ref={startEl}>
            <ChannelStart channel={props.channel} />
          </div>
        </Show>
        <div class="feed-inner" style={{ height: `${virt.getTotalSize()}px` }}>
          {/* Rows are keyed by message key (not by TanStack item objects) so they never remount on re-measure. */}
          <For each={virt.getVirtualItems().map((v) => v.key as string)}>
            {(key) => {
              const row = () => virt.getVirtualItems().find((v) => v.key === key);
              const m = () => {
                const r = row();
                return r ? list()[r.index] : undefined;
              };
              return (
                <Show when={m() && row()}>
                  <MessageRow
                    m={m()!}
                    index={row()!.index}
                    start={row()!.start - virt.options.scrollMargin}
                    grouped={grouped(row()!.index)}
                    flash={flash() === m()!.key}
                    enter={!!m()!.pending || (m()!.id !== 0n && msgTime(m()!) > openedAt)}
                    day={newDay(row()!.index) ? fmtDay.format(msgTime(m()!)) : undefined}
                    newLine={firstUnread(row()!.index)}
                    measure={(el) => virt.measureElement(el)}
                    resolver={resolver}
                    canManage={props.canManage}
                    editing={editing() === m()!.key}
                    onEdit={() => setEditing(m()!.key)}
                    onEditDone={() => setEditing(undefined)}
                    onReply={() => setReplyTo(m())}
                    onReact={(e) => react(m()!, e)}
                    onJump={scrollTo}
                    menu={menuFor}
                    parent={m()!.replyToId ? list().find((x) => x.id === m()!.replyToId) : undefined}
                  />
                </Show>
              );
            }}
          </For>
        </div>
      </div>
      <Show when={showJump()}>
        <button class="jump-present" onClick={() => virt.scrollToIndex(list().length - 1, { align: "end" })}>
          Вы смотрите старые сообщения · Перейти к новым
        </button>
      </Show>
      <Typing channelId={cid()} />
      <Composer
        channel={props.channel}
        canSend={props.canSend}
        canAttach={props.canAttach}
        bindFiles={(f) => (addFiles = f)}
        replyTo={replyTo()}
        onCancelReply={() => setReplyTo(undefined)}
        onEditLast={() => {
          const mine = [...list()].reverse().find((m) => m.author?.id === state.me?.id && m.id !== 0n);
          if (mine) setEditing(mine.key);
        }}
      />
    </>
  );
}

function ChannelStart(props: { channel: Channel }) {
  const peer = () => (props.channel.guildId === 0n ? props.channel.recipientIds.find((u) => u !== state.me?.id) : undefined);
  return (
    <div class="history-edge">
      <Show
        when={peer() !== undefined}
        fallback={
          <>
            <div class="start-icon" aria-hidden="true">#</div>
            <h2>Добро пожаловать в #{props.channel.name}!</h2>
            <p class="edited">Это начало канала #{props.channel.name}. {props.channel.topic}</p>
          </>
        }
      >
        <UserAvatar user={state.users[sid(peer()!)]} size={80} />
        <h2>{state.users[sid(peer()!)]?.displayName || state.users[sid(peer()!)]?.username}</h2>
        <p class="edited">
          Это начало вашей истории личных сообщений с @{state.users[sid(peer()!)]?.username}.
          <Show when={props.channel.e2ee}> Беседа защищена сквозным шифрованием.</Show>
        </p>
      </Show>
    </div>
  );
}

function MessageRow(props: {
  m: MessageView;
  index: number;
  start: number;
  grouped: boolean;
  flash?: boolean;
  enter?: boolean;
  day?: string;
  newLine: boolean;
  measure: (el: HTMLElement) => void;
  resolver: MentionResolver;
  canManage: boolean;
  editing: boolean;
  onEdit: () => void;
  onEditDone: () => void;
  onReply: () => void;
  onReact: (emoji: string) => void;
  onJump: (id: bigint) => void;
  menu: (m: MessageView, onPicker: () => void) => MenuItem[];
  parent?: MessageView;
}) {
  let el!: HTMLElement;
  const mine = () => props.m.author?.id === state.me?.id;
  const ts = () => new Date(msgTime(props.m));
  const mentionsMe = () => props.m.mentionEveryone || props.m.mentionUserIds.some((u) => u === state.me?.id);
  const member = () => memberOf(props.m);
  const color = () => roleColor(props.m.guildId === 0n ? undefined : sid(props.m.guildId), member()?.roleIds);
  // E2EE messages carry file keys after the text; the server never sees either.
  const parsed = () => (props.m.e2eePayload.length ? parseManifest(props.m.content) : { text: props.m.content, keys: [] });
  const sticker = () => (props.m.stickerId ? Object.values(state.guilds).flatMap((g) => g.stickers).find((x) => x.id === props.m.stickerId) : undefined);
  const [draft, setDraft] = createSignal(props.m.content);
  createEffect(on(() => props.editing, (e) => e && setDraft(props.m.content)));
  const [picker, setPicker] = createSignal(false);
  onMount(() => props.measure(el));
  createEffect(on(() => [props.m.content, props.m.reactions.length, props.m.embeds.length, props.editing, picker(), props.day, props.newLine], () => queueMicrotask(() => el?.isConnected && props.measure(el))));
  const profile = (e: MouseEvent) => props.m.author && openProfile({ userId: sid(props.m.author.id), guildId: currentGuild(), anchor: e.currentTarget as HTMLElement });
  return (
    <article
      ref={el}
      data-index={props.index}
      class="msg"
      classList={{ head: !props.grouped, pending: props.m.pending, failed: !!props.m.failed, mention: mentionsMe(), pinned: props.m.pinned, flash: !!props.flash, enter: !!props.enter }}
      style={{ transform: `translateY(${props.start}px)` }}
      aria-setsize={-1}
      aria-posinset={props.index + 1}
      onContextMenu={(e) => props.m.id !== 0n && openContextMenu(e, props.menu(props.m, () => setPicker(true)), "Действия с сообщением")}
    >
      <Show when={props.day}>
        <div class="day-divider" role="separator">
          <span>{props.day}</span>
        </div>
      </Show>
      <Show when={props.newLine}>
        <div class="new-divider" role="separator" aria-label="Новые сообщения">
          <span>новые</span>
        </div>
      </Show>
      <Show when={props.parent}>
        <button class="reply-ref" onClick={() => props.onJump(props.parent!.id)}>
          <span class="reply-spine" aria-hidden="true" />
          <UserAvatar user={authorOf(props.parent!)} size={16} noDeco />
          <b>@{authorName(props.parent!)}</b>
          <span class="reply-text">{plainText(props.parent!.content).slice(0, 120) || "вложение"}</span>
        </button>
      </Show>
      <Show
        when={!props.grouped}
        fallback={
          <time class="hover-time" datetime={ts().toISOString()} title={fmtDate.format(ts())}>
            {fmtTime.format(ts())}
          </time>
        }
      >
        <button class="avatar" aria-label={`Профиль ${authorName(props.m)}`} onClick={profile}>
          <UserAvatar user={authorOf(props.m)} memberAvatar={member()?.avatar} name={authorName(props.m)} size={40} />
        </button>
        <div class="meta">
          <button class="author" onClick={profile} onContextMenu={(e) => (e.stopPropagation(), props.m.author && openUserMenu(e, sid(props.m.author.id), authorName(props.m)))}>
            <UserName user={authorOf(props.m)} text={authorName(props.m)} color={color()} />
          </button>
          <Show when={props.m.author?.bot}>
            <span class="tag-chip bot">БОТ</span>
          </Show>
          <time datetime={ts().toISOString()} title={fmtDate.format(ts())}>
            {fmtTime.format(ts())}
          </time>
        </div>
      </Show>
      <div class="body">
        <Show
          when={props.editing}
          fallback={
            <Show when={props.m.content} fallback={<Show when={props.m.e2eePayload.length}><i class="edited">🔒 зашифрованное сообщение</i></Show>}>
              <Show when={parsed().text}>
                <Markdown source={parsed().text} resolver={props.resolver} />
              </Show>
            </Show>
          }
        >
          <textarea
            class="ns-input"
            style={{ width: "100%", height: "auto", padding: "8px" }}
            value={draft()}
            aria-label="Редактирование сообщения"
            onInput={(e) => setDraft(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") props.onEditDone();
              if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
                e.preventDefault();
                if (draft().trim() && draft() !== props.m.content) core().send({ case: "editMessage", value: { channelId: props.m.channelId, messageId: props.m.id, content: draft() } });
                props.onEditDone();
              }
            }}
            ref={(t) => queueMicrotask(() => t.focus())}
          />
          <small class="edited">Esc — отмена · Enter — сохранить</small>
        </Show>
        <Show when={props.m.editedAt !== 0n}>
          <span class="edited" title={fmtDate.format(Number(props.m.editedAt))}> (изменено)</span>
        </Show>
        <Show when={props.m.failed}>
          <div class="ns-error">
            Не отправлено: {props.m.failed}{" "}
            <Show when={props.m.retryable}>
              <button class="ns-btn ns-btn--ghost ns-btn--sm" onClick={() => core().send({ case: "retryMessage", value: { nonce: props.m.nonce } })}>
                <RotateCcw size={14} /> Повторить
              </button>
            </Show>
          </div>
        </Show>
      </div>
      <Show when={props.m.attachments.length}>
        <Show when={parsed().keys.length} fallback={<AttachmentList items={props.m.attachments} safeUrl={safeUrl} onLoad={() => el?.isConnected && props.measure(el)} />}>
          <EncryptedAttachments items={props.m.attachments} keys={parsed().keys} safeUrl={safeUrl} onLoad={() => el?.isConnected && props.measure(el)} />
        </Show>
      </Show>
      <Show when={props.m.stickerId !== 0n}>
        <Show when={sticker() && asset(sticker()!.key)} fallback={<span class="edited">стикер</span>}>
          <img class="msg-sticker" src={asset(sticker()!.key)} alt={`Стикер ${sticker()!.name}`} title={sticker()!.name} loading="lazy" onLoad={() => el?.isConnected && props.measure(el)} />
        </Show>
      </Show>
      <For each={props.m.embeds}>
        {(e) => (
          <div class="embed" style={e.color ? { "border-left-color": `#${e.color.toString(16).padStart(6, "0")}` } : undefined}>
            <Show when={e.siteName}>
              <small class="edited">{e.siteName}</small>
            </Show>
            <div>
              <Show when={safeUrl(e.url)} fallback={<b>{e.title}</b>}>
                {(url) => <a href={url()} target="_blank" rel="noopener noreferrer nofollow">{e.title || url()}</a>}
              </Show>
            </div>
            <Show when={e.description}>
              <div class="edited" style={{ "font-size": "14px" }}>{e.description}</div>
            </Show>
          </div>
        )}
      </For>
      <Show when={props.m.reactions.length || picker()}>
        <div class="reactions">
          <For each={props.m.reactions}>
            {(r) => (
              <button class="reaction" aria-pressed={r.me} aria-label={`${r.emoji}: ${r.count}`} onClick={() => props.onReact(r.emoji)}>
                {r.emoji} <Show when={r.count} keyed>{(n) => <span>{n}</span>}</Show>
              </button>
            )}
          </For>
          <Show when={picker()}>
            <For each={QUICK_REACTIONS}>
              {(e) => (
                <button class="reaction add" aria-label={`Реакция ${e}`} onClick={() => (props.onReact(e), setPicker(false))}>
                  {e}
                </button>
              )}
            </For>
          </Show>
        </div>
      </Show>
      <Show when={props.m.id !== 0n}>
        <div class="actions" role="toolbar" aria-label="Действия с сообщением">
          <For each={QUICK_REACTIONS.slice(0, 3)}>
            {(e) => (
              <Tooltip label={`Реакция ${e}`}>
                <button class="ns-icon-btn quick-react" aria-label={`Реакция ${e}`} onClick={() => props.onReact(e)}>
                  {e}
                </button>
              </Tooltip>
            )}
          </For>
          <span class="actions-sep" aria-hidden="true" />
          <Tooltip label="Добавить реакцию">
            <IconButton label="Добавить реакцию" onClick={() => setPicker(!picker())}>
              <SmilePlus size={18} />
            </IconButton>
          </Tooltip>
          <Tooltip label="Ответить">
            <IconButton label="Ответить" onClick={props.onReply}>
              <Reply size={18} />
            </IconButton>
          </Tooltip>
          <Show when={mine()}>
            <Tooltip label="Изменить">
              <IconButton label="Изменить" onClick={props.onEdit}>
                <Pencil size={18} />
              </IconButton>
            </Tooltip>
          </Show>
          <Tooltip label="Ещё">
            <IconButton label="Ещё" onClick={(e) => openContextMenu(e, props.menu(props.m, () => setPicker(true)), "Действия с сообщением")}>
              <span aria-hidden="true" style={{ "font-weight": 700, "letter-spacing": "1px" }}>···</span>
            </IconButton>
          </Tooltip>
        </div>
      </Show>
    </article>
  );
}

function Typing(props: { channelId: string }) {
  const [now, setNow] = createSignal(Date.now());
  const t = setInterval(() => setNow(Date.now()), 1000);
  onCleanup(() => clearInterval(t));
  const who = () =>
    Object.entries(state.typing[props.channelId] ?? {})
      .filter(([uid, until]) => until > now() && uid !== sid(state.me?.id ?? 0n))
      .map(([uid]) => state.users[uid]?.displayName || state.users[uid]?.username || "кто-то");
  return (
    <div class="typing" aria-live="polite">
      <Show when={who().length}>
        <span class="dots" aria-hidden="true"><span /><span /><span /></span>
        {who().length === 1 ? `${who()[0]} печатает…` : who().length <= 3 ? `${who().join(", ")} печатают…` : "Несколько человек печатают…"}
      </Show>
    </div>
  );
}

function Composer(props: { channel: Channel; canSend: boolean; canAttach: boolean; replyTo?: MessageView; onCancelReply: () => void; onEditLast: () => void; bindFiles: (add: (files: File[]) => void) => void }) {
  const [text, setText] = createSignal("");
  const [uploads, setUploads] = createStore<UploadItem[]>([]);
  let fileInput!: HTMLInputElement;
  let seq = 0;
  const attachAllowed = () => props.canSend && props.canAttach && (!props.channel.e2ee || core().e2ee);
  const addFiles = (files: File[]) => {
    if (!attachAllowed()) {
      toast(props.channel.e2ee ? "Включите шифрование ЛС на этом устройстве, чтобы прикреплять файлы" : "В этом канале нельзя прикреплять файлы", "error");
      return;
    }
    const room = MAX_FILES - uploads.length;
    if (files.length > room) toast(`Не больше ${MAX_FILES} файлов в сообщении`, "error");
    for (const file of files.slice(0, Math.max(0, room))) {
      const key = ++seq;
      const preview = file.type.startsWith("image/") ? URL.createObjectURL(file) : undefined;
      setUploads(uploads.length, { key, file, preview, progress: 0 });
      const at = () => uploads.findIndex((u) => u.key === key);
      // In E2EE conversations the bytes are encrypted here; only ciphertext is uploaded.
      const prepared = props.channel.e2ee
        ? encryptFile(file).then(({ blob, key: fk }) => {
            if (at() >= 0) setUploads(at(), "fileKey", fk);
            return new File([blob], "encrypted.bin", { type: "application/octet-stream" });
          })
        : Promise.resolve(file);
      prepared
        .then((f) => uploadAttachment(sid(props.channel.id), f, (p) => at() >= 0 && setUploads(at(), "progress", p)))
        .then((id) => at() >= 0 && setUploads(at(), "id", id))
        .catch((e: Error) => at() >= 0 && setUploads(at(), "error", e.message));
    }
  };
  props.bindFiles(addFiles);
  const removeUpload = (key: number) => {
    const u = uploads.find((x) => x.key === key);
    if (u?.preview) URL.revokeObjectURL(u.preview);
    setUploads((list) => list.filter((x) => x.key !== key));
  };
  const busy = () => uploads.some((u) => !u.id && !u.error);
  // Drafts and uploads belong to one channel.
  createEffect(on(() => props.channel.id, () => setUploads([]), { defer: true }));
  const [emojiOpen, setEmojiOpen] = createSignal(false);
  const [emojiTab, setEmojiTab] = createSignal<"std" | "guild" | "stickers">("std");
  const guildStickers = () => (props.channel.guildId === 0n ? [] : (state.guilds[sid(props.channel.guildId)]?.stickers ?? []));
  const sendSticker = (id: bigint) => {
    core().send({ case: "sendMessage", value: { channelId: props.channel.id, content: "", replyToId: props.replyTo?.id ?? 0n, attachmentIds: [], stickerId: id } });
    setEmojiOpen(false);
    props.onCancelReply();
  };
  const guildEmojis = () => (props.channel.guildId === 0n ? [] : (state.guilds[sid(props.channel.guildId)]?.emojis ?? []));
  let area!: HTMLTextAreaElement;
  const insert = (e: string) => {
    const at = area.selectionStart ?? text().length;
    const next = text().slice(0, at) + e + text().slice(area.selectionEnd ?? at);
    setText(next);
    setEmojiOpen(false);
    queueMicrotask(() => {
      area.focus();
      area.setSelectionRange(at + e.length, at + e.length);
      resize();
    });
  };
  let lastTyping = 0;
  const resize = () => {
    area.style.height = "auto";
    area.style.height = `${Math.min(area.scrollHeight, window.innerHeight * 0.4)}px`;
  };
  const send = () => {
    const content = text().trim();
    if (busy()) {
      toast("Дождитесь окончания загрузки файлов");
      return;
    }
    const done = uploads.filter((u) => u.id);
    const ids = done.map((u) => BigInt(u.id!));
    if (!content && !ids.length) return;
    const body = props.channel.e2ee ? withManifest(content, done.map((u) => u.fileKey!).filter(Boolean)) : content;
    core().send({ case: "sendMessage", value: { channelId: props.channel.id, content: body, replyToId: props.replyTo?.id ?? 0n, attachmentIds: ids } });
    for (const u of uploads) if (u.preview) URL.revokeObjectURL(u.preview);
    setUploads([]);
    setText("");
    props.onCancelReply();
    queueMicrotask(resize);
  };
  createEffect(on(() => props.channel.id, () => requestAnimationFrame(() => area?.focus())));
  return (
    <div class="composer">
      <Show when={props.replyTo}>
        <div class="replying">
          <span>
            Ответ пользователю <b>{authorName(props.replyTo!)}</b>
          </span>
          <IconButton label="Отменить ответ" onClick={props.onCancelReply}>
            <X size={14} />
          </IconButton>
        </div>
      </Show>
      <UploadTray items={uploads} onRemove={removeUpload} />
      <div class="composer-box">
        <input
          ref={fileInput}
          type="file"
          multiple
          hidden
          onChange={(e) => {
            addFiles([...(e.currentTarget.files ?? [])]);
            e.currentTarget.value = "";
          }}
        />
        <Tooltip label={attachAllowed() ? "Прикрепить файл" : "Прикреплять файлы здесь нельзя"}>
          <IconButton label="Прикрепить файл" disabled={!attachAllowed()} onClick={() => fileInput.click()}>
            <CirclePlus size={22} />
          </IconButton>
        </Tooltip>
        <textarea
          ref={area}
          rows={1}
          value={text()}
          disabled={!props.canSend}
          placeholder={!props.canSend ? "Нет прав на отправку сообщений" : props.channel.e2ee ? (core().e2ee ? "Зашифрованное сообщение (видно только участникам)" : "Включите шифрование ЛС в настройках, чтобы писать сюда") : `Написать ${props.channel.name ? `в #${props.channel.name}` : "сообщение"}`}
          aria-label="Сообщение"
          maxLength={4000}
          onInput={(e) => {
            setText(e.currentTarget.value);
            resize();
            if (Date.now() - lastTyping > 8000) {
              lastTyping = Date.now();
              core().send({ case: "setTyping", value: { channelId: props.channel.id } });
            }
          }}
          onPaste={(e) => {
            const files = [...(e.clipboardData?.files ?? [])];
            if (files.length) {
              e.preventDefault();
              addFiles(files);
            }
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
              e.preventDefault();
              send();
            } else if (e.key === "ArrowUp" && !text()) {
              e.preventDefault();
              props.onEditLast();
            } else if (e.key === "Escape") props.onCancelReply();
          }}
        />
        <Show when={text().length > 3500}>
          <span class="char-count" classList={{ over: text().length > 4000 }} aria-live="polite">
            {4000 - text().length}
          </span>
        </Show>
        <Tooltip label="Эмодзи">
          <IconButton label="Вставить эмодзи" aria-expanded={emojiOpen()} disabled={!props.canSend} onClick={() => setEmojiOpen(!emojiOpen())}>
            <Smile size={20} />
          </IconButton>
        </Tooltip>
      </div>
      <Show when={emojiOpen()}>
        <div class="composer-emoji">
          <div class="emoji-tabs" role="tablist">
            <button role="tab" aria-selected={emojiTab() === "std"} onClick={() => setEmojiTab("std")}>Стандартные</button>
            <Show when={guildEmojis().length}>
              <button role="tab" aria-selected={emojiTab() === "guild"} onClick={() => setEmojiTab("guild")}>Этого сервера</button>
            </Show>
            <Show when={guildStickers().length && !props.channel.e2ee}>
              <button role="tab" aria-selected={emojiTab() === "stickers"} onClick={() => setEmojiTab("stickers")}>Стикеры</button>
            </Show>
          </div>
          <Show when={emojiTab() === "stickers"}>
            <div class="sticker-grid" role="group" aria-label="Стикеры">
              <For each={guildStickers()}>
                {(st) => (
                  <button aria-label={`Стикер ${st.name}`} title={st.name} onClick={() => sendSticker(st.id)}>
                    <img src={asset(st.key)} alt="" loading="lazy" />
                  </button>
                )}
              </For>
            </div>
          </Show>
          <div class="emoji-grid" role="group" aria-label="Эмодзи" hidden={emojiTab() === "stickers"}>
            <Show
              when={emojiTab() === "guild" && guildEmojis().length}
              fallback={
                <For each={QUICK_EMOJI}>
                  {(e) => (
                    <button aria-label={e} onClick={() => insert(e)}>
                      {e}
                    </button>
                  )}
                </For>
              }
            >
              <For each={guildEmojis()}>
                {(e) => (
                  <button aria-label={`:${e.name}:`} title={`:${e.name}:`} onClick={() => insert(`:${e.name}: `)}>
                    <img src={asset(e.key)} alt="" />
                  </button>
                )}
              </For>
            </Show>
          </div>
        </div>
      </Show>
    </div>
  );
}
