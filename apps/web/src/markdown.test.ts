import { describe, expect, it } from "vitest";

import { parseBlocks, parseInline, plainText } from "./markdown";

describe("markdown", () => {
  it("parses nested emphasis and code", () => {
    expect(parseInline("**жирный *курсив*** и `код`")).toEqual([
      { t: "b", c: [{ t: "text", v: "жирный " }, { t: "i", c: [{ t: "text", v: "курсив" }] }] },
      { t: "text", v: " и " },
      { t: "code", v: "код" },
    ]);
  });

  it("does not interpret markup inside code", () => {
    expect(parseInline("`**x** <@1>`")).toEqual([{ t: "code", v: "**x** <@1>" }]);
  });

  it("mentions, links and spoilers", () => {
    expect(parseInline("hi <@123> <@&9> @here https://a.ru/x. ||тайна||")).toEqual([
      { t: "text", v: "hi " },
      { t: "user", id: "123" },
      { t: "text", v: " " },
      { t: "role", id: "9" },
      { t: "text", v: " " },
      { t: "everyone", v: "@here" },
      { t: "text", v: " " },
      { t: "link", href: "https://a.ru/x" },
      { t: "text", v: ". " },
      { t: "spoiler", c: [{ t: "text", v: "тайна" }] },
    ]);
  });

  it("never produces javascript: links or raw HTML", () => {
    const nodes = parseInline('<img src=x onerror=alert(1)> javascript:alert(1) [x](javascript:alert(1))');
    expect(nodes.every((n) => n.t === "text")).toBe(true);
  });

  it("blocks: code fences and quotes", () => {
    const b = parseBlocks("текст\n```rust\nfn main() {}\n```\n> цитата");
    expect(b.map((x) => x.t)).toEqual(["p", "pre", "quote"]);
    expect(b[1]).toEqual({ t: "pre", lang: "rust", v: "fn main() {}" });
  });

  it("unclosed markers stay literal", () => {
    expect(parseInline("2 * 3 = 6 и **не закрыто")).toEqual([{ t: "text", v: "2 * 3 = 6 и **не закрыто" }]);
  });
});

describe("custom emoji", () => {
  it("parses :name: tokens and leaves other colons alone", async () => {
    const { parseInline } = await import("./markdown");
    expect(parseInline("привет :party_cat: ура")).toEqual([
      { t: "text", v: "привет " },
      { t: "emoji", name: "party_cat" },
      { t: "text", v: " ура" },
    ]);
    expect(parseInline("время: 12 :x")).toEqual([{ t: "text", v: "время: 12 :x" }]);
  });

  it("flattens to plain text for previews", () => {
    expect(plainText("Нужен фронтендер **на SolidJS**")).toBe("Нужен фронтендер на SolidJS");
    expect(plainText("> цитата\nи `код` с ||секретом||")).toBe("цитата и код с ▒▒▒");
    expect(plainText("```rust\nfn main() {}\n```")).toBe("fn main() {}");
  });
});
