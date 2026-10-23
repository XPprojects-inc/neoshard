import { describe, expect, it } from "vitest";

import { upsertMessage, type MessageView } from "./state";

const m = (id: bigint, nonce = ""): MessageView => ({ id, nonce, key: id ? String(id) : `n:${nonce}` }) as unknown as MessageView;

describe("upsertMessage", () => {
  it("keeps confirmed messages sorted and pending ones last", () => {
    const list: MessageView[] = [m(10n), m(20n)];
    upsertMessage(list, m(0n, "a")); // pending
    upsertMessage(list, m(30n)); // new from someone else
    upsertMessage(list, m(5n)); // older history page while a send is pending
    expect(list.map((x) => x.id)).toEqual([5n, 10n, 20n, 30n, 0n]);
  });

  it("replaces the pending copy when confirmed, keeping its key", () => {
    const list: MessageView[] = [m(10n), m(0n, "a")];
    upsertMessage(list, { ...m(40n), nonce: "a" });
    expect(list.map((x) => x.id)).toEqual([10n, 40n]);
    expect(list[1].key).toBe("n:a");
  });

  it("deduplicates by id", () => {
    const list: MessageView[] = [m(10n)];
    upsertMessage(list, m(10n));
    expect(list.length).toBe(1);
  });
});

describe("logout", () => {
  it("resets the state without cloning store proxies", async () => {
    const { create, UpdateSchema, AuthPhase } = await import("@neoshard/core");
    const { applyUpdates, state } = await import("./state");
    const auth = (phase: number) => create(UpdateSchema, { kind: { case: "auth", value: { phase, error: "" } } });
    applyUpdates([auth(AuthPhase.LOGGED_IN)]);
    expect(() => applyUpdates([auth(AuthPhase.LOGGED_OUT)])).not.toThrow();
    expect(state.auth.phase).toBe(AuthPhase.LOGGED_OUT);
    expect(state.me).toBeUndefined();
  });
});
