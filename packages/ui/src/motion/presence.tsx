// <Presence>: keeps removed children in the DOM until their exit animation
// finishes (the vanilla counterpart of AnimatePresence). Works for a single
// conditional child and for lists (<For> inside); elements are tracked by identity.

import { children as resolveChildren, createEffect, createSignal, on, onCleanup, untrack, type JSX } from "solid-js";

import { animateElement } from "./animator";
import { motionConfig, motionTokens, reducedFade, springs } from "./tokens";

export type Keyframes = Record<string, number | string | Array<number | string>>;

export interface PresenceProps {
  children: JSX.Element;
  /** Animate children present at first render. Default false (no flash on load). */
  initial?: boolean;
  enter?: Keyframes;
  exit?: Keyframes;
  onExitComplete?: (el: Element) => void;
}

export const presets = {
  fade: { enter: { opacity: [0, 1] }, exit: { opacity: [1, 0] } },
  pop: { enter: { opacity: [0, 1], scale: [0.96, 1] }, exit: { opacity: [1, 0], scale: [1, 0.96] } },
  rise: { enter: { opacity: [0, 1], y: [motionTokens.distance.md, 0] }, exit: { opacity: [1, 0], y: [0, motionTokens.distance.sm] } },
} satisfies Record<string, { enter: Keyframes; exit: Keyframes }>;

function elements(v: unknown): Element[] {
  const out: Element[] = [];
  const walk = (x: unknown) => {
    if (Array.isArray(x)) x.forEach(walk);
    else if (typeof Element !== "undefined" && x instanceof Element) out.push(x);
  };
  walk(v);
  return out;
}

/** Merge next children with leaving ones, keeping leaving elements at their old index. */
export function mergeOrder<T>(prev: T[], next: T[], leaving: Set<T>): T[] {
  const result = [...next];
  prev.forEach((el, i) => {
    if (leaving.has(el) && !result.includes(el)) result.splice(Math.min(i, result.length), 0, el);
  });
  return result;
}

function reduceKeyframes(k: Keyframes): Keyframes {
  // Reduced motion: opacity only.
  return k.opacity !== undefined ? { opacity: k.opacity } : {};
}

export function Presence(props: PresenceProps): JSX.Element {
  const resolved = resolveChildren(() => props.children);
  const [shown, setShown] = createSignal<Element[]>([]);
  const leaving = new Set<Element>();
  let first = true;

  const run = (el: Element, kf: Keyframes, done?: () => void) => {
    const reduce = !motionConfig.shouldAnimate({ essential: true });
    const keyframes = reduce ? reduceKeyframes(kf) : kf;
    const options = reduce ? reducedFade : { ...springs.snappy, duration: motionTokens.duration.fast };
    if (Object.keys(keyframes).length === 0) {
      done?.();
      return;
    }
    (el as HTMLElement).style?.setProperty("will-change", "transform, opacity");
    animateElement(el, keyframes, options).finished.then(
      () => {
        (el as HTMLElement).style?.removeProperty("will-change");
        done?.();
      },
      () => done?.(),
    );
  };

  createEffect(
    on(resolved, (value) => {
      const next = elements(value);
      const prev = untrack(shown);
      const entering = next.filter((e) => !prev.includes(e) || leaving.has(e));
      for (const e of prev) {
        if (!next.includes(e) && !leaving.has(e)) {
          leaving.add(e);
          run(e, props.exit ?? presets.pop.exit, () => {
            leaving.delete(e);
            setShown((s) => s.filter((x) => x !== e));
            props.onExitComplete?.(e);
          });
        }
      }
      for (const e of entering) leaving.delete(e);
      setShown(mergeOrder(prev, next, leaving));
      if (!first || props.initial) {
        for (const e of entering) run(e, props.enter ?? presets.pop.enter);
      }
      first = false;
    }),
  );
  onCleanup(() => leaving.clear());
  return <>{shown()}</>;
}
