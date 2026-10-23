import { describe, expect, it } from "vitest";

import { inviteCode } from "./instance";

describe("inviteCode", () => {
  it("takes the code from links and bare codes", () => {
    expect(inviteCode("https://chat.example.ru/invite/AbC_12-x")).toBe("AbC_12-x");
    expect(inviteCode("  https://chat.example.ru/app/?invite=Q9w8  ")).toBe("Q9w8");
    expect(inviteCode("http://tauri.localhost/invite/Zz1?utm=1")).toBe("Zz1");
    expect(inviteCode("Kq3Lm")).toBe("Kq3Lm");
    expect(inviteCode("")).toBe("");
  });
});
