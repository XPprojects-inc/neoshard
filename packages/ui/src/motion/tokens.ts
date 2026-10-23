// Motion tokens (ported from the ecc motion-foundations skill to vanilla Motion).
// Every duration, easing and spring in NeoShard comes from here.

export const motionTokens = {
  /** seconds */
  duration: { instant: 0.08, fast: 0.18, normal: 0.35, slow: 0.6, crawl: 1.0 },
  easing: {
    smooth: [0.22, 1, 0.36, 1] as [number, number, number, number],
    sharp: [0.4, 0, 0.2, 1] as [number, number, number, number],
    bounce: [0.34, 1.56, 0.64, 1] as [number, number, number, number],
    linear: [0, 0, 1, 1] as [number, number, number, number],
  },
  distance: { xs: 4, sm: 8, md: 16, lg: 24, xl: 48 },
  scale: { subtle: 0.98, press: 0.95, pop: 1.04 },
} as const;

export interface SpringConfig {
  type: "spring";
  stiffness: number;
  damping: number;
  mass?: number;
  restDelta?: number;
}

export const springs = {
  snappy: { type: "spring", stiffness: 300, damping: 30 },
  gentle: { type: "spring", stiffness: 120, damping: 14 },
  bouncy: { type: "spring", stiffness: 400, damping: 10 },
  instant: { type: "spring", stiffness: 600, damping: 35 },
  release: { type: "spring", stiffness: 200, damping: 20, restDelta: 0.001 },
} as const satisfies Record<string, SpringConfig>;

export type SpringName = keyof typeof springs;

let forcedReduced: boolean | undefined;

/** Tests and the "reduce motion" user setting can force the preference. */
export function setReducedMotion(value: boolean | undefined): void {
  forcedReduced = value;
}

export const motionConfig = {
  prefersReduced(): boolean {
    if (forcedReduced !== undefined) return forcedReduced;
    return typeof window !== "undefined" && typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  },
  isLowEnd(): boolean {
    return typeof navigator !== "undefined" && (navigator.hardwareConcurrency ?? 8) <= 4;
  },
  /** Gate for every animation. Essential = conveys state (keep on low-end devices). */
  shouldAnimate({ essential = false } = {}): boolean {
    if (this.prefersReduced()) return false;
    if (!essential && this.isLowEnd()) return false;
    return true;
  },
};

/** Opacity-only fallback allowed under reduced motion (≤ 0.2 s). */
export const reducedFade = { duration: 0.15, ease: motionTokens.easing.linear };
