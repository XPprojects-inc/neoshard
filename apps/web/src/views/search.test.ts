import { describe, expect, it } from "vitest";

import { setState } from "../state";
import { parseQuery } from "./Search";

describe("parseQuery", () => {
  it("splits filters from words and resolves names locally", () => {
    setState("users", { "7": { id: 7n, username: "alice", displayName: "Алиса" } as never });
    setState("guilds", { "1": { id: "1", channels: [{ id: 10n, name: "общий" }] } as never });
    const p = parseQuery("релиз from:alice in:#общий has:файл", "1");
    expect(p).toEqual({ text: "релиз", authorId: "7", channelId: "10", has: "attachment", unknown: [] });
    expect(parseQuery("от:Алиса есть:ссылка", "1")).toMatchObject({ authorId: "7", has: "link" });
  });

  it("reports filters it cannot resolve instead of dropping them silently", () => {
    expect(parseQuery("from:nobody has:wat", "1").unknown).toEqual(["from:nobody", "has:wat"]);
  });
});
