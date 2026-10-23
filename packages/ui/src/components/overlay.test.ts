import { describe, expect, it } from "vitest";

import { place } from "./overlay";

const rect = (x: number, y: number, w: number, h: number) => new DOMRect(x, y, w, h);

describe("place", () => {
  it("puts a popover to the right of its anchor", () => {
    expect(place(rect(100, 100, 40, 40), 300, 200, "right", 1440, 900)).toEqual({ x: 148, y: 100 });
  });

  it("flips to the left near the right edge", () => {
    const p = place(rect(1300, 100, 40, 40), 300, 200, "right", 1440, 900);
    expect(p.x).toBe(1300 - 8 - 300);
  });

  it("flips above when there is no room below and clamps into the viewport", () => {
    const p = place(rect(20, 860, 100, 30), 280, 400, "bottom-start", 1440, 900);
    expect(p.y).toBe(860 - 8 - 400);
    const q = place(rect(-50, 10, 10, 10), 200, 100, "bottom", 1440, 900);
    expect(q.x).toBe(8);
  });
});
