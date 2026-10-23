import { describe, expect, it } from "vitest";

import { emojiName, PERMISSION_GROUPS } from "./ServerSettings";
import { Permission } from "@neoshard/core";

describe("emojiName", () => {
  it("makes server-valid names ([A-Za-z0-9_]{2,32})", () => {
    expect(emojiName("Котик")).toBe("kotik");
    expect(emojiName("party cat!")).toBe("party_cat");
    expect(emojiName("ж")).toBe("zh");
    expect(emojiName("!")).toMatch(/^emoji_/);
    expect(emojiName("a".repeat(50))).toHaveLength(32);
    for (const n of ["Котик", "party cat!", "ж", "!", "x"]) expect(emojiName(n)).toMatch(/^[a-z0-9_]{2,32}$/);
  });
});

describe("permission groups", () => {
  it("cover every permission bit exactly once", () => {
    const keys = PERMISSION_GROUPS.flatMap((g) => g.items.map(([k]) => k));
    expect(new Set(keys).size).toBe(keys.length);
    expect(keys.sort()).toEqual(Object.keys(Permission).sort());
  });
});
