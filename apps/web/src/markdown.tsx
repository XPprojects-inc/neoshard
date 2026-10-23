// Safe Markdown → Solid nodes. User content never goes through innerHTML:
// we tokenize and build elements with JSX, so there is no HTML injection.
// Supported: **bold**, *italic*, __underline__, ~~strike~~, `code`, ```blocks```,
// > quotes, ||spoilers||, http(s) links, <@user>, <@&role>, @everyone/@here.

import { createSignal, For, type JSX } from "solid-js";

export interface MentionResolver {
  user(id: string): string | undefined;
  role(id: string): { name: string; color: number } | undefined;
  isMe(id: string): boolean;
  /** URL of a custom emoji :name: (servers the user is in), if known. */
  emoji?(name: string): string | undefined;
}

export type Inline =
  | { t: "text"; v: string }
  | { t: "b" | "i" | "u" | "s" | "spoiler"; c: Inline[] }
  | { t: "code"; v: string }
  | { t: "link"; href: string }
  | { t: "user"; id: string }
  | { t: "role"; id: string }
  | { t: "everyone"; v: string }
  | { t: "emoji"; name: string };

export type Block = { t: "p"; c: Inline[] } | { t: "quote"; c: Inline[] } | { t: "pre"; lang: string; v: string };

const PAIRS: Array<[string, Inline["t"]]> = [
  ["**", "b"],
  ["__", "u"],
  ["~~", "s"],
  ["||", "spoiler"],
  ["*", "i"],
];
// Single "_" is not italic: it would mangle snake_case identifiers.

/** Parse inline markup. Pure function (unit-tested). */
export function parseInline(src: string, depth = 0): Inline[] {
  const out: Inline[] = [];
  let buf = "";
  const flush = () => {
    if (buf) out.push({ t: "text", v: buf });
    buf = "";
  };
  let i = 0;
  outer: while (i < src.length) {
    // Only look a bounded distance ahead: keeps parsing linear on long messages.
    const rest = src.slice(i, i + 64);
    if (rest[0] === "`") {
      const end = src.indexOf("`", i + 1);
      if (end > i + 1) {
        flush();
        out.push({ t: "code", v: src.slice(i + 1, end) });
        i = end + 1;
        continue;
      }
    }
    if (rest.startsWith("<@")) {
      const m = /^<@(&|!)?(\d{1,20})>/.exec(rest);
      if (m) {
        flush();
        out.push(m[1] === "&" ? { t: "role", id: m[2] } : { t: "user", id: m[2] });
        i += m[0].length;
        continue;
      }
    }
    if (rest[0] === ":") {
      const m = /^:([A-Za-z0-9_]{2,32}):/.exec(rest);
      if (m) {
        flush();
        out.push({ t: "emoji", name: m[1] });
        i += m[0].length;
        continue;
      }
    }
    if (rest.startsWith("@everyone") || rest.startsWith("@here")) {
      flush();
      const v = rest.startsWith("@everyone") ? "@everyone" : "@here";
      out.push({ t: "everyone", v });
      i += v.length;
      continue;
    }
    if (/^https?:\/\//.test(rest)) {
      const m = /^https?:\/\/[^\s<>"]+/.exec(src.slice(i, i + 2048))!;
      let href = m[0];
      while (/[.,!?;:)]$/.test(href)) href = href.slice(0, -1);
      flush();
      out.push({ t: "link", href });
      i += href.length;
      continue;
    }
    if (depth < 4) {
      for (const [mark, t] of PAIRS) {
        if (rest.startsWith(mark)) {
          let end = src.indexOf(mark, i + mark.length);
          // "***": close with the last two, so the inner "*" pair stays balanced.
          while (end > 0 && src[end + mark.length] === mark[0]) end++;
          if (end > i + mark.length && !/\s/.test(src[i + mark.length])) {
            flush();
            out.push({ t, c: parseInline(src.slice(i + mark.length, end), depth + 1) } as Inline);
            i = end + mark.length;
            continue outer;
          }
        }
      }
    }
    buf += src[i];
    i++;
  }
  flush();
  return out;
}

export function parseBlocks(src: string): Block[] {
  const blocks: Block[] = [];
  const lines = src.split("\n");
  let para: string[] = [];
  const flushPara = () => {
    if (para.length) blocks.push({ t: "p", c: parseInline(para.join("\n")) });
    para = [];
  };
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (line.startsWith("```")) {
      const end = lines.findIndex((l, j) => j > i && l.startsWith("```"));
      if (end > i) {
        flushPara();
        blocks.push({ t: "pre", lang: line.slice(3).trim().slice(0, 20), v: lines.slice(i + 1, end).join("\n") });
        i = end;
        continue;
      }
    }
    if (line.startsWith("> ")) {
      flushPara();
      blocks.push({ t: "quote", c: parseInline(line.slice(2)) });
      continue;
    }
    para.push(line);
  }
  flushPara();
  return blocks;
}

function Spoiler(props: { children: JSX.Element }) {
  const [shown, setShown] = createSignal(false);
  return (
    <span
      class="md-spoiler"
      classList={{ shown: shown() }}
      role="button"
      tabindex="0"
      aria-expanded={shown()}
      aria-label={shown() ? undefined : "Спойлер, нажмите чтобы показать"}
      onClick={() => setShown(true)}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          setShown(true);
        }
      }}
    >
      {props.children}
    </span>
  );
}

function renderInline(nodes: Inline[], r: MentionResolver): JSX.Element {
  return (
    <For each={nodes}>
      {(n) => {
        switch (n.t) {
          case "text":
            return n.v;
          case "b":
            return <strong>{renderInline(n.c, r)}</strong>;
          case "i":
            return <em>{renderInline(n.c, r)}</em>;
          case "u":
            return <u>{renderInline(n.c, r)}</u>;
          case "s":
            return <s>{renderInline(n.c, r)}</s>;
          case "spoiler":
            return <Spoiler>{renderInline(n.c, r)}</Spoiler>;
          case "code":
            return <code class="md-code">{n.v}</code>;
          case "link":
            return (
              <a href={n.href} target="_blank" rel="noopener noreferrer nofollow ugc">
                {n.href}
              </a>
            );
          case "user":
            return <span class="md-mention" classList={{ me: r.isMe(n.id) }}>@{r.user(n.id) ?? "неизвестный"}</span>;
          case "role": {
            const role = r.role(n.id);
            return (
              <span class="md-mention" style={role?.color ? { color: `#${role.color.toString(16).padStart(6, "0")}` } : undefined}>
                @{role?.name ?? "роль"}
              </span>
            );
          }
          case "everyone":
            return <span class="md-mention">{n.v}</span>;
          case "emoji": {
            const url = r.emoji?.(n.name);
            return url ? <img class="md-emoji" src={url} alt={`:${n.name}:`} title={`:${n.name}:`} loading="lazy" draggable={false} /> : `:${n.name}:`;
          }
        }
      }}
    </For>
  );
}

export function Markdown(props: { source: string; resolver: MentionResolver }): JSX.Element {
  return (
    <For each={parseBlocks(props.source)}>
      {(b) =>
        b.t === "pre" ? (
          <pre class="md-pre" data-lang={b.lang || undefined}>
            <code>{b.v}</code>
          </pre>
        ) : b.t === "quote" ? (
          <blockquote class="md-quote">{renderInline(b.c, props.resolver)}</blockquote>
        ) : (
          <div class="md-p">{renderInline(b.c, props.resolver)}</div>
        )
      }
    </For>
  );
}

/** One-line plain text for previews (reply quotes, notifications); spoilers stay hidden. */
export function plainText(src: string): string {
  const flat = (xs: Inline[]): string =>
    xs
      .map((x) => {
        switch (x.t) {
          case "text":
          case "code":
          case "everyone":
            return x.v;
          case "spoiler":
            return "▒▒▒";
          case "b":
          case "i":
          case "u":
          case "s":
            return flat(x.c);
          case "link":
            return x.href;
          case "user":
            return "@…";
          case "role":
            return "@роль";
          case "emoji":
            return `:${x.name}:`;
        }
      })
      .join("");
  return parseBlocks(src)
    .map((b) => (b.t === "pre" ? b.v : flat(b.c)))
    .join(" ")
    .replace(/\s+/g, " ")
    .trim();
}
