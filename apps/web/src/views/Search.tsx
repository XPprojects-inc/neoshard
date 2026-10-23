// Message search: a box in the channel header and a results panel.
// Query syntax on top of plain words: from:имя  in:канал  has:файл|ссылка|закреп

import { IconButton } from "@neoshard/ui";
import { createSignal, For, Show } from "solid-js";

import { select } from "../actions";
import { Search as SearchIcon, X } from "../icons";
import { Markdown } from "../markdown";
import { mentionResolver } from "../actions";
import { UserAvatar } from "../profile/components";
import { api, sid, state, type Id } from "../state";
import { snowflakeTime } from "../time";

export interface ParsedQuery {
  text: string;
  authorId?: Id;
  channelId?: Id;
  has?: "attachment" | "link" | "pinned";
  /** Filters that could not be resolved (unknown user/channel). */
  unknown: string[];
}

const HAS: Record<string, ParsedQuery["has"]> = {
  файл: "attachment", file: "attachment", вложение: "attachment", attachment: "attachment",
  ссылка: "link", link: "link",
  закреп: "pinned", pinned: "pinned",
};

/** Split filters out of the query and resolve names against the local cache. */
export function parseQuery(raw: string, guildId: Id | undefined): ParsedQuery {
  const out: ParsedQuery = { text: "", unknown: [] };
  const words: string[] = [];
  for (const tok of raw.trim().split(/\s+/).filter(Boolean)) {
    const m = /^(from|от|in|в|has|есть):(.+)$/i.exec(tok);
    if (!m) {
      words.push(tok);
      continue;
    }
    const key = m[1].toLowerCase();
    const val = m[2].replace(/^[@#]/, "").toLowerCase();
    if (key === "from" || key === "от") {
      const u = Object.values(state.users).find((x) => x.username.toLowerCase() === val || x.displayName.toLowerCase() === val);
      if (u) out.authorId = sid(u.id);
      else out.unknown.push(tok);
    } else if (key === "in" || key === "в") {
      const c = guildId ? state.guilds[guildId]?.channels.find((x) => x.name.toLowerCase() === val) : undefined;
      if (c) out.channelId = sid(c.id);
      else out.unknown.push(tok);
    } else {
      const h = HAS[val];
      if (h) out.has = h;
      else out.unknown.push(tok);
    }
  }
  out.text = words.join(" ");
  return out;
}

interface Hit {
  id: string;
  channel_id: string;
  guild_id?: string;
  content?: string;
  author?: { id: string; username: string; display_name?: string };
  attachments?: unknown[];
}

const [query, setQuery] = createSignal("");
const [results, setResults] = createSignal<{ hits: Hit[]; done: boolean; error?: string; scope: string }>();
const [loading, setLoading] = createSignal(false);
export const searchOpen = () => results() !== undefined || loading();

/** A message to scroll to once its channel is open. */
const [jump, setJump] = createSignal<{ channel: Id; id: bigint }>();
export const jumpTarget = jump;
export const clearJump = () => setJump(undefined);

async function run(more = false) {
  const guild = state.selection.guild === "@me" ? undefined : state.selection.guild;
  const p = parseQuery(query(), guild);
  if (!p.text && !p.authorId && !p.has) return;
  const prev = more ? (results()?.hits ?? []) : [];
  const params = new URLSearchParams();
  if (p.text) params.set("q", p.text);
  if (p.authorId) params.set("author_id", p.authorId);
  if (p.has) params.set("has", p.has);
  if (p.channelId) params.set("channel_id", p.channelId);
  params.set("offset", String(prev.length));
  params.set("limit", "25");
  const path = guild ? `/guilds/${guild}/messages/search?${params}` : `/channels/${state.selection.channel}/messages/search?${params}`;
  setLoading(true);
  const r = await api<{ messages: Hit[]; message?: string }>("GET", path);
  setLoading(false);
  const scope = guild ? `на сервере «${state.guilds[guild]?.name ?? ""}»` : "в этой беседе";
  if (r.status !== 200) {
    setResults({ hits: prev, done: true, scope, error: r.body?.message?.includes("E2EE") ? "Зашифрованные беседы сервер не может искать" : "Поиск не удался" });
    return;
  }
  const hits = [...prev, ...r.body.messages];
  setResults({ hits, done: r.body.messages.length < 25, scope, error: p.unknown.length ? `Не понял фильтр: ${p.unknown.join(", ")}` : undefined });
}

export function closeSearch() {
  setResults(undefined);
  setQuery("");
}

export function SearchBox() {
  let input!: HTMLInputElement;
  return (
    <div class="search-box" role="search">
      <input
        ref={input}
        value={query()}
        placeholder="Поиск"
        aria-label="Поиск сообщений (from:имя, in:канал, has:файл|ссылка|закреп)"
        title="from:имя  in:канал  has:файл|ссылка|закреп"
        onInput={(e) => setQuery(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") void run();
          if (e.key === "Escape") closeSearch();
        }}
      />
      <Show when={query()} fallback={<SearchIcon size={16} aria-hidden="true" />}>
        <button class="search-clear" aria-label="Очистить поиск" onClick={() => (closeSearch(), input.focus())}>
          <X size={14} />
        </button>
      </Show>
    </div>
  );
}

const fmt = new Intl.DateTimeFormat("ru-RU", { day: "numeric", month: "short", year: "numeric", hour: "2-digit", minute: "2-digit" });

export function SearchPanel() {
  const channelName = (h: Hit) => {
    if (!h.guild_id || h.guild_id === "0") return "";
    return state.guilds[h.guild_id]?.channels.find((c) => sid(c.id) === h.channel_id)?.name ?? "";
  };
  const open = (h: Hit) => {
    const guild = !h.guild_id || h.guild_id === "0" ? "@me" : h.guild_id;
    select(guild, h.channel_id);
    setJump({ channel: h.channel_id, id: BigInt(h.id) });
  };
  return (
    <aside class="search-panel" aria-label="Результаты поиска">
      <header>
        <b>
          <Show when={!loading()} fallback="Ищем…">
            {results()?.hits.length ?? 0}
            {results()?.done ? "" : "+"} результатов {results()?.scope}
          </Show>
        </b>
        <IconButton label="Закрыть поиск" onClick={closeSearch}>
          <X size={18} />
        </IconButton>
      </header>
      <Show when={results()?.error}>
        <p class="ns-error" style={{ padding: "0 12px" }}>{results()!.error}</p>
      </Show>
      <div class="search-results">
        <For each={results()?.hits ?? []} fallback={<Show when={!loading()}><p class="empty">Ничего не найдено. Попробуйте другие слова или фильтры: from:имя, in:канал, has:файл.</p></Show>}>
          {(h) => (
            <button class="search-hit" onClick={() => open(h)}>
              <Show when={channelName(h)}>
                <span class="search-hit-channel">#{channelName(h)}</span>
              </Show>
              <span class="search-hit-body">
                <UserAvatar user={h.author ? state.users[h.author.id] : undefined} name={h.author?.display_name || h.author?.username || "?"} size={32} noDeco />
                <span>
                  <span class="search-hit-meta">
                    <b>{h.author?.display_name || h.author?.username}</b>
                    <small>{fmt.format(snowflakeTime(BigInt(h.id)))}</small>
                  </span>
                  <span class="search-hit-text">
                    <Show when={h.content} fallback={<i class="edited">вложение</i>}>
                      <Markdown source={h.content!.slice(0, 400)} resolver={mentionResolver} />
                    </Show>
                  </span>
                </span>
              </span>
            </button>
          )}
        </For>
        <Show when={results() && !results()!.done && !loading()}>
          <button class="ns-btn ns-btn--secondary ns-btn--sm" style={{ margin: "8px auto" }} onClick={() => void run(true)}>
            Показать ещё
          </button>
        </Show>
      </div>
    </aside>
  );
}
