// layout(): FLIP animations for position/size changes (the vanilla counterpart
// of `layout` / `layoutId`). Only transforms are animated — never width/height.
//
//  flip(container, () => setState(...))   — reorder/expand: measure, mutate, invert, play
//  layoutId(el, "avatar-42")               — shared element: a new element with the same id
//                                            animates from where the previous one was

import { animateElement } from "./animator";
import { motionConfig, springs } from "./tokens";

export interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface Delta {
  x: number;
  y: number;
  scaleX: number;
  scaleY: number;
}

/** Transform that maps the `to` box back onto the `from` box (the "invert" step). */
export function invert(from: Box, to: Box): Delta {
  return {
    x: from.x - to.x + (from.width - to.width) / 2,
    y: from.y - to.y + (from.height - to.height) / 2,
    scaleX: to.width === 0 ? 1 : from.width / to.width,
    scaleY: to.height === 0 ? 1 : from.height / to.height,
  };
}

export function isIdentity(d: Delta, epsilon = 0.5): boolean {
  return Math.abs(d.x) < epsilon && Math.abs(d.y) < epsilon && Math.abs(d.scaleX - 1) < 0.01 && Math.abs(d.scaleY - 1) < 0.01;
}

function box(el: Element): Box {
  const r = el.getBoundingClientRect();
  return { x: r.left, y: r.top, width: r.width, height: r.height };
}

function play(el: Element, d: Delta, scale = true): Promise<unknown> {
  if (isIdentity(d)) return Promise.resolve();
  const kf: Record<string, number[]> = { x: [d.x, 0], y: [d.y, 0] };
  if (scale && (d.scaleX !== 1 || d.scaleY !== 1)) {
    kf.scaleX = [d.scaleX, 1];
    kf.scaleY = [d.scaleY, 1];
  }
  (el as HTMLElement).style.transformOrigin = "50% 50%";
  return animateElement(el, kf, { ...springs.snappy }).finished;
}

const KEY = "[data-layout]";

/**
 * Measure every `[data-layout]` element under `container` (and the container
 * itself if marked), run `mutate` (a synchronous Solid state change), then
 * animate each element from its old box to its new one.
 */
export function flip(container: Element, mutate: () => void, opts: { scale?: boolean } = {}): Promise<unknown> {
  if (!motionConfig.shouldAnimate()) {
    mutate();
    return Promise.resolve();
  }
  const keyOf = (el: Element) => el.getAttribute("data-layout") || el;
  const before = new Map<unknown, Box>();
  const els = [...(container.matches(KEY) ? [container] : []), ...container.querySelectorAll(KEY)];
  for (const el of els) before.set(keyOf(el), box(el));
  mutate();
  const after = [...(container.matches(KEY) ? [container] : []), ...container.querySelectorAll(KEY)];
  return Promise.all(
    after.map((el) => {
      // The element under the pointer is positioned by drag(), not by FLIP.
      if (el.hasAttribute("data-dragging")) return Promise.resolve();
      const from = before.get(keyOf(el));
      return from ? play(el, invert(from, box(el)), opts.scale ?? false) : Promise.resolve();
    }),
  );
}

// ---------------------------------------------------------------- shared elements

const lastBoxes = new Map<string, { box: Box; at: number }>();
const SHARED_TTL_MS = 600;

/**
 * Register an element under a shared `id`. If another element with this id
 * disappeared recently, this one animates from the old box (avatar in the
 * member list → avatar in the profile card). Call the returned function when
 * the element is removed (Solid: in onCleanup).
 */
export function layoutId(el: Element, id: string): () => void {
  const prev = lastBoxes.get(id);
  if (prev && performance.now() - prev.at < SHARED_TTL_MS && motionConfig.shouldAnimate()) {
    requestAnimationFrame(() => void play(el, invert(prev.box, box(el)), true));
  }
  lastBoxes.delete(id);
  return () => {
    if (el.isConnected) lastBoxes.set(id, { box: box(el), at: performance.now() });
  };
}

/** Snapshot a box for `id` explicitly (before navigating away). */
export function rememberLayout(id: string, el: Element): void {
  lastBoxes.set(id, { box: box(el), at: performance.now() });
}
