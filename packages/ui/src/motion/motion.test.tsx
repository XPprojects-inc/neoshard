import { render } from "@solidjs/testing-library";
import { createSignal, For, Show } from "solid-js";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { instantAnimator, setAnimator, type ElementAnimator } from "./animator";
import { clamp, drag, project, rubberband, velocity } from "./drag";
import { invert, isIdentity } from "./layout";
import { mergeOrder, Presence } from "./presence";
import { move, targetIndex } from "./reorder";
import { motionConfig, setReducedMotion, springs } from "./tokens";

describe("tokens", () => {
  afterEach(() => setReducedMotion(undefined));

  it("reduced motion disables animation even if essential", () => {
    setReducedMotion(true);
    expect(motionConfig.shouldAnimate({ essential: true })).toBe(false);
    setReducedMotion(false);
    expect(motionConfig.shouldAnimate({ essential: true })).toBe(true);
  });

  it("springs come from the shared map", () => {
    expect(springs.snappy).toEqual({ type: "spring", stiffness: 300, damping: 30 });
    expect(springs.release.restDelta).toBe(0.001);
  });
});

describe("presence ordering", () => {
  it("keeps leaving items at their old index", () => {
    expect(mergeOrder(["a", "b", "c"], ["a", "c"], new Set(["b"]))).toEqual(["a", "b", "c"]);
    expect(mergeOrder(["a", "b"], ["c"], new Set(["a", "b"]))).toEqual(["a", "b", "c"]);
    expect(mergeOrder([], ["x"], new Set())).toEqual(["x"]);
  });
});

describe("<Presence>", () => {
  let pending: Array<() => void>;
  let calls: Array<Record<string, unknown>>;

  beforeEach(() => {
    setReducedMotion(false);
    pending = [];
    calls = [];
    const controllable: ElementAnimator = (_el, kf) => {
      calls.push(kf);
      let resolve!: () => void;
      const finished = new Promise<void>((r) => (resolve = r));
      pending.push(resolve);
      return { stop() {}, finished };
    };
    setAnimator(controllable, instantAnimator.value);
  });
  afterEach(() => {
    setAnimator();
    setReducedMotion(undefined);
  });

  it("removes a child only after its exit animation finishes", async () => {
    const [show, setShow] = createSignal(true);
    let exited = 0;
    const r = render(() => (
      <Presence onExitComplete={() => exited++}>
        <Show when={show()}>
          <div data-testid="modal">modal</div>
        </Show>
      </Presence>
    ));
    expect(r.queryByTestId("modal")).not.toBeNull();
    expect(calls.length).toBe(0); // no enter animation on first render
    setShow(false);
    expect(r.queryByTestId("modal")).not.toBeNull(); // still there while exiting
    expect(calls[0].opacity).toEqual([1, 0]);
    pending.forEach((p) => p());
    await new Promise((r) => setTimeout(r, 0));
    expect(r.queryByTestId("modal")).toBeNull();
    expect(exited).toBe(1);
    setShow(true);
    expect(r.queryByTestId("modal")).not.toBeNull();
    expect(calls[1].opacity).toEqual([0, 1]); // enter animation after first render
  });

  it("animates list removals independently", async () => {
    const [items, setItems] = createSignal(["a", "b", "c"]);
    const r = render(() => (
      <Presence>
        <For each={items()}>{(i) => <p data-testid={i}>{i}</p>}</For>
      </Presence>
    ));
    setItems(["a", "c"]);
    expect(r.queryByTestId("b")).not.toBeNull();
    const order = [...r.container.querySelectorAll("p")].map((p) => p.textContent);
    expect(order).toEqual(["a", "b", "c"]);
    pending.forEach((p) => p());
    await new Promise((r) => setTimeout(r, 0));
    expect(r.queryByTestId("b")).toBeNull();
  });

  it("reduced motion uses an opacity-only fade", () => {
    setReducedMotion(true);
    const [show, setShow] = createSignal(true);
    render(() => (
      <Presence exit={{ opacity: [1, 0], y: [0, 20], scale: [1, 0.9] }}>
        <Show when={show()}>
          <div>x</div>
        </Show>
      </Presence>
    ));
    setShow(false);
    expect(calls[0]).toEqual({ opacity: [1, 0] });
  });
});

describe("layout (FLIP)", () => {
  it("inverts boxes", () => {
    const d = invert({ x: 0, y: 100, width: 50, height: 50 }, { x: 0, y: 0, width: 50, height: 50 });
    expect(d).toEqual({ x: 0, y: 100, scaleX: 1, scaleY: 1 });
    const grow = invert({ x: 0, y: 0, width: 50, height: 20 }, { x: 0, y: 0, width: 100, height: 20 });
    expect(grow.scaleX).toBe(0.5);
    expect(isIdentity(invert({ x: 1, y: 1, width: 10, height: 10 }, { x: 1.2, y: 1, width: 10, height: 10 }))).toBe(true);
  });
});

describe("drag math", () => {
  it("rubber-bands past constraints", () => {
    expect(rubberband(50, 0, 100, 0.5)).toBe(50);
    expect(rubberband(120, 0, 100, 0.5)).toBe(110);
    expect(rubberband(-40, 0, 100, 0.25)).toBe(-10);
    expect(rubberband(500, undefined, undefined, 0.3)).toBe(500);
  });

  it("clamps and projects momentum", () => {
    expect(clamp(150, 0, 100)).toBe(100);
    expect(project(0, 1000)).toBeCloseTo(325);
  });

  it("measures velocity over the last 100 ms", () => {
    const s = [
      { t: 0, v: 0 },
      { t: 900, v: 0 },
      { t: 950, v: 50 },
      { t: 1000, v: 100 },
    ];
    expect(velocity(s)).toBeCloseTo(1000);
    expect(velocity([{ t: 0, v: 0 }])).toBe(0);
  });

  it("drags with pointer events and springs back", async () => {
    setAnimator(instantAnimator.element, instantAnimator.value);
    setReducedMotion(false);
    const el = document.createElement("div");
    document.body.append(el);
    const ends: number[] = [];
    const h = drag(el, { axis: "x", snapBack: true, constraints: { left: -100, right: 100 }, onEnd: (i) => void ends.push(i.x) });
    const pe = (type: string, x: number, t: number) => {
      const e = new MouseEvent(type, { clientX: x, clientY: 0, button: 0, bubbles: true });
      Object.defineProperty(e, "pointerId", { value: 1 });
      Object.defineProperty(e, "timeStamp", { value: t });
      el.dispatchEvent(e);
    };
    pe("pointerdown", 0, 0);
    pe("pointermove", 2, 10); // below threshold
    expect(h.position().x).toBe(0);
    pe("pointermove", 60, 20);
    expect(h.position().x).toBe(60);
    pe("pointermove", 300, 30); // beyond constraint → rubber band
    expect(h.position().x).toBeCloseTo(100 + 200 * 0.35);
    pe("pointerup", 300, 40);
    await Promise.resolve();
    expect(ends[0]).toBeGreaterThan(100);
    expect(h.position().x).toBe(0); // snapped back
    h.dispose();
    setAnimator();
    setReducedMotion(undefined);
  });
});

describe("reorder", () => {
  it("computes the slot from neighbour centers", () => {
    expect(targetIndex([10, 30, 50], 5)).toBe(0);
    expect(targetIndex([10, 30, 50], 35)).toBe(2);
    expect(targetIndex([10, 30, 50], 99)).toBe(3);
  });

  it("moves items", () => {
    expect(move(["a", "b", "c", "d"], 0, 2)).toEqual(["b", "c", "a", "d"]);
    expect(move(["a", "b", "c"], 2, 0)).toEqual(["c", "a", "b"]);
    expect(move(["a", "b"], 0, 10)).toEqual(["b", "a"]);
  });
});
