import { describe, expect, it } from "vitest";

import { activeCustom } from "../actions";
import { setState } from "../state";
import { cssUrl } from "./components";
import { nameStyle } from "./cosmetics";
import { viewFromJson } from "./data";

describe("profile view", () => {
  it("lets the server profile override the user profile, field by field", () => {
    setState("filesBase", "https://files.example.ru");
    const v = viewFromJson(
      {
        user: { id: "7", username: "alice", display_name: "Алиса", avatar: "avatars/7/1" },
        pronouns: "она",
        bio: "общий профиль",
        banner: "banners/7/2",
        theme_colors: [1, 2],
        member: { guild_id: "9", nick: "Модератор", bio: "серверный профиль", avatar: "members/9/7/3", theme_colors: [] },
        mutual_guild_ids: ["9"],
      },
      undefined,
    );
    expect(v.displayName).toBe("Модератор");
    expect(v.bio).toBe("серверный профиль");
    expect(v.pronouns).toBe("она"); // empty server pronouns fall back
    expect(v.avatar).toBe("https://files.example.ru/members/9/7/3");
    expect(v.banner).toBe("https://files.example.ru/banners/7/2");
    expect(v.theme).toEqual([1, 2]);
    expect(v.serverProfile).toBe(true);
  });

  it("keeps absolute URLs of federated users as they are", () => {
    const v = viewFromJson({ user: { id: "1", username: "bob", avatar: "https://other.example/a.png" } }, undefined);
    expect(v.avatar).toBe("https://other.example/a.png");
  });
});

describe("cssUrl", () => {
  it("passes plain https URLs and blocks anything that could break out of url()", () => {
    expect(cssUrl("https://x.ru/a.png")).toBe('url("https://x.ru/a.png")');
    expect(cssUrl('https://x.ru/a.png") , url("https://evil')).toBe('url("about:invalid")');
    expect(cssUrl("javascript:alert(1)")).toBe('url("about:invalid")');
    expect(cssUrl("https://x.ru/a b.png")).toBe('url("about:invalid")');
  });
});

describe("name styles", () => {
  it("maps font and effect to a class and colour variables", () => {
    const s = nameStyle({ font: "serif", effect: "gradient", colors: [0xff0000, 0x0000ff] });
    expect(s.class).toBe("nm nm-gradient");
    expect(s.style["font-family"]).toContain("Georgia");
    expect((s.style as Record<string, string>)["--n1"]).toBe("#ff0000");
    expect(nameStyle(undefined).class).toBe("");
    expect(nameStyle({ font: "", effect: "", colors: [0x00ff00] }).class).toBe("nm nm-solid");
  });
});

describe("custom status", () => {
  it("hides an expired status", () => {
    expect(activeCustom({ custom: "обед", emoji: "🍕", expires: Date.now() + 60_000 })).toEqual({ text: "обед", emoji: "🍕" });
    expect(activeCustom({ custom: "обед", emoji: "🍕", expires: Date.now() - 1 })).toBeUndefined();
    expect(activeCustom({ custom: "", emoji: "", expires: 0 })).toBeUndefined();
  });
});
