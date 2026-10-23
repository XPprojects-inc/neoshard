import { beforeEach, describe, expect, it } from "vitest";
import { deviceE2ee } from "../index.js";

describe("deviceE2ee", () => {
  beforeEach(() => {
    const data = new Map<string, string>();
    globalThis.localStorage = {
      getItem: (k: string) => data.get(k) ?? null,
      setItem: (k: string, v: string) => void data.set(k, v),
      removeItem: (k: string) => void data.delete(k),
    } as Storage;
  });

  it("is off by default and toggles", () => {
    expect(deviceE2ee.get()).toBe(false);
    deviceE2ee.set(true);
    expect(deviceE2ee.get()).toBe(true);
    deviceE2ee.set(false);
    expect(deviceE2ee.get()).toBe(false);
  });

  it("survives a storage that throws (private mode)", () => {
    globalThis.localStorage = {
      getItem: () => {
        throw new Error("denied");
      },
      setItem: () => {
        throw new Error("denied");
      },
      removeItem: () => {},
    } as unknown as Storage;
    expect(() => deviceE2ee.set(true)).not.toThrow();
    expect(deviceE2ee.get()).toBe(false);
  });
});
