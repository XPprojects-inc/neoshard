// Thin adapter over Motion's vanilla `animate()` so primitives can be tested
// without a browser animation engine (tests install a synchronous animator).

import { animate } from "motion";

export interface Controls {
  stop(): void;
  finished: Promise<unknown>;
}

export type ElementAnimator = (el: Element, keyframes: Record<string, unknown>, options: Record<string, unknown>) => Controls;
export type ValueAnimator = (from: number, to: number, options: Record<string, unknown> & { onUpdate: (v: number) => void }) => Controls;

const motionElement: ElementAnimator = (el, keyframes, options) => {
  const a = animate(el as HTMLElement, keyframes as never, options as never);
  return { stop: () => a.stop(), finished: a.finished };
};

const motionValue: ValueAnimator = (from, to, options) => {
  const a = animate(from, to, options as never);
  return { stop: () => a.stop(), finished: a.finished };
};

let elementAnimator: ElementAnimator = motionElement;
let valueAnimator: ValueAnimator = motionValue;

export function animateElement(el: Element, keyframes: Record<string, unknown>, options: Record<string, unknown>): Controls {
  return elementAnimator(el, keyframes, options);
}

export function animateValue(from: number, to: number, options: Record<string, unknown> & { onUpdate: (v: number) => void }): Controls {
  return valueAnimator(from, to, options);
}

/** Replace the animation engine (tests). Pass nothing to restore Motion. */
export function setAnimator(el?: ElementAnimator, value?: ValueAnimator): void {
  elementAnimator = el ?? motionElement;
  valueAnimator = value ?? motionValue;
}

/** An animator that jumps to the end immediately; handy for tests and reduced motion. */
export const instantAnimator: { element: ElementAnimator; value: ValueAnimator } = {
  element: (el, keyframes) => {
    const style = (el as HTMLElement).style;
    for (const [k, v] of Object.entries(keyframes)) {
      const last = Array.isArray(v) ? v[v.length - 1] : v;
      if (k === "opacity") style.opacity = String(last);
    }
    return { stop() {}, finished: Promise.resolve() };
  },
  value: (_from, to, options) => {
    options.onUpdate(to);
    return { stop() {}, finished: Promise.resolve() };
  },
};
