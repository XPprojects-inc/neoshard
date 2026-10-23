// drag(): Pointer-Events dragging with axis lock, constraints, rubber-band
// overscroll, momentum and a spring back (vanilla counterpart of `drag`).
// Uses transforms only; works with mouse, pen and touch.

import { animateValue, type Controls } from "./animator";
import { motionConfig, springs } from "./tokens";

export type Axis = "x" | "y" | "both";

export interface Constraints {
  left?: number;
  right?: number;
  top?: number;
  bottom?: number;
}

export interface DragInfo {
  x: number;
  y: number;
  vx: number;
  vy: number;
  dx: number;
  dy: number;
}

export interface DragOptions {
  axis?: Axis;
  constraints?: Constraints | (() => Constraints);
  /** 0 = hard stop at constraints, 1 = free; default 0.35 */
  elastic?: number;
  /** Continue with inertia after release (projected, then clamped). Default true. */
  momentum?: boolean;
  /** Return to origin on release (sheets/swipes that commit elsewhere). */
  snapBack?: boolean;
  /** Pixels before a drag starts (lets clicks through). Default 4. */
  threshold?: number;
  onStart?(info: DragInfo): void;
  onMove?(info: DragInfo): void;
  /** Return a target {x,y} to settle there instead of the default. */
  onEnd?(info: DragInfo): { x?: number; y?: number } | void;
}

/** Rubber band: resistance grows with distance beyond the limit. */
export function rubberband(value: number, min: number | undefined, max: number | undefined, elastic: number): number {
  if (min !== undefined && value < min) return min - (min - value) * elastic;
  if (max !== undefined && value > max) return max + (value - max) * elastic;
  return value;
}

export function clamp(value: number, min?: number, max?: number): number {
  if (min !== undefined && value < min) return min;
  if (max !== undefined && value > max) return max;
  return value;
}

/** Where momentum would carry a value (exponential decay, time constant 0.325 s). */
export function project(value: number, velocity: number, power = 0.325): number {
  return value + velocity * power;
}

/** Velocity (px/s) from recent samples (last ~100 ms). */
export function velocity(samples: Array<{ t: number; v: number }>): number {
  if (samples.length < 2) return 0;
  const last = samples[samples.length - 1];
  let first = samples[0];
  for (const s of samples) {
    if (last.t - s.t <= 100) {
      first = s;
      break;
    }
  }
  const dt = (last.t - first.t) / 1000;
  return dt > 0 ? (last.v - first.v) / dt : 0;
}

export interface DragHandle {
  /** Current offset. */
  position(): { x: number; y: number };
  /** Move without animation. */
  set(x: number, y: number): void;
  /** Shift offset and drag origin together (the layout box moved under the pointer, e.g. reorder). */
  shift(dx: number, dy: number): void;
  /** Animate to a position with a spring. */
  to(x: number, y: number): Promise<unknown>;
  dispose(): void;
}

export function drag(el: HTMLElement, opts: DragOptions = {}): DragHandle {
  const axis = opts.axis ?? "both";
  const elastic = opts.elastic ?? 0.35;
  const threshold = opts.threshold ?? 4;
  let x = 0;
  let y = 0;
  let originX = 0;
  let originY = 0;
  let startX = 0;
  let startY = 0;
  let active = false;
  let pointerId: number | null = null;
  let samplesX: Array<{ t: number; v: number }> = [];
  let samplesY: Array<{ t: number; v: number }> = [];
  let running: Controls[] = [];
  let frame = 0;

  const limits = () => (typeof opts.constraints === "function" ? opts.constraints() : opts.constraints ?? {});
  const apply = () => {
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(() => {
      el.style.transform = `translate3d(${x}px, ${y}px, 0)`;
    });
  };
  const stopAll = () => {
    running.forEach((r) => r.stop());
    running = [];
  };
  const info = (): DragInfo => ({ x, y, vx: velocity(samplesX), vy: velocity(samplesY), dx: x - originX, dy: y - originY });

  const onDown = (e: PointerEvent) => {
    if (e.button !== 0 || pointerId !== null) return;
    pointerId = e.pointerId;
    stopAll();
    startX = e.clientX;
    startY = e.clientY;
    originX = x;
    originY = y;
    samplesX = [{ t: e.timeStamp, v: x }];
    samplesY = [{ t: e.timeStamp, v: y }];
  };

  const onMove = (e: PointerEvent) => {
    if (e.pointerId !== pointerId) return;
    const mx = e.clientX - startX;
    const my = e.clientY - startY;
    if (!active) {
      if (Math.hypot(mx, my) < threshold) return;
      // Axis lock: ignore drags mostly along the other axis (lets page scroll win).
      if (axis === "x" && Math.abs(my) > Math.abs(mx)) return release(e, false);
      if (axis === "y" && Math.abs(mx) > Math.abs(my)) return release(e, false);
      active = true;
      el.setPointerCapture?.(e.pointerId);
      el.style.willChange = "transform";
      el.dataset.dragging = "";
      opts.onStart?.(info());
    }
    const c = limits();
    if (axis !== "y") x = rubberband(originX + mx, c.left, c.right, elastic);
    if (axis !== "x") y = rubberband(originY + my, c.top, c.bottom, elastic);
    samplesX.push({ t: e.timeStamp, v: x });
    samplesY.push({ t: e.timeStamp, v: y });
    if (samplesX.length > 20) {
      samplesX.shift();
      samplesY.shift();
    }
    apply();
    opts.onMove?.(info());
  };

  const release = (e: PointerEvent, wasDrag = active) => {
    if (e.pointerId !== pointerId) return;
    pointerId = null;
    el.releasePointerCapture?.(e.pointerId);
    if (!wasDrag) return;
    active = false;
    delete el.dataset.dragging;
    const i = info();
    const custom = opts.onEnd?.(i) ?? {};
    const c = limits();
    let tx: number;
    let ty: number;
    if (opts.snapBack) {
      tx = originX;
      ty = originY;
    } else {
      const momentum = opts.momentum !== false && motionConfig.shouldAnimate();
      tx = clamp(momentum ? project(x, i.vx) : x, c.left, c.right);
      ty = clamp(momentum ? project(y, i.vy) : y, c.top, c.bottom);
    }
    if (custom.x !== undefined) tx = custom.x;
    if (custom.y !== undefined) ty = custom.y;
    void settle(tx, ty, i.vx, i.vy).then(() => {
      el.style.willChange = "";
    });
  };

  const settle = (tx: number, ty: number, vx = 0, vy = 0): Promise<unknown> => {
    stopAll();
    if (!motionConfig.shouldAnimate({ essential: true })) {
      x = tx;
      y = ty;
      apply();
      return Promise.resolve();
    }
    const ax = animateValue(x, tx, { ...springs.release, velocity: vx, onUpdate: (v: number) => ((x = v), apply()) });
    const ay = animateValue(y, ty, { ...springs.release, velocity: vy, onUpdate: (v: number) => ((y = v), apply()) });
    running = [ax, ay];
    return Promise.all([ax.finished, ay.finished]);
  };

  const onUp = (e: PointerEvent) => {
    if (active) markDragged();
    release(e);
  };
  const onClickCapture = (e: MouseEvent) => {
    // A drag must not also trigger a click on release.
    if (el.dataset.justDragged !== undefined) {
      e.stopPropagation();
      e.preventDefault();
      delete el.dataset.justDragged;
    }
  };
  const markDragged = () => {
    el.dataset.justDragged = "";
    setTimeout(() => delete el.dataset.justDragged, 0);
  };

  el.style.touchAction = axis === "x" ? "pan-y" : axis === "y" ? "pan-x" : "none";
  el.addEventListener("pointerdown", onDown);
  el.addEventListener("pointermove", onMove);
  el.addEventListener("pointerup", onUp);
  el.addEventListener("pointercancel", onUp);
  el.addEventListener("click", onClickCapture, true);

  return {
    position: () => ({ x, y }),
    set(nx, ny) {
      stopAll();
      x = nx;
      y = ny;
      el.style.transform = `translate3d(${x}px, ${y}px, 0)`;
    },
    shift(dx, dy) {
      x += dx;
      y += dy;
      originX += dx;
      originY += dy;
      el.style.transform = `translate3d(${x}px, ${y}px, 0)`;
    },
    to: (nx, ny) => settle(nx, ny),
    dispose() {
      stopAll();
      cancelAnimationFrame(frame);
      el.removeEventListener("pointerdown", onDown);
      el.removeEventListener("pointermove", onMove);
      el.removeEventListener("pointerup", onUp);
      el.removeEventListener("pointercancel", onUp);
      el.removeEventListener("click", onClickCapture, true);
    },
  };
}
