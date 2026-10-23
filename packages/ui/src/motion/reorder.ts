// reorder(): sortable lists built on drag() + flip() (vanilla counterpart of
// Reorder.Group / Reorder.Item). Siblings glide out of the way with FLIP,
// the dragged item stays under the pointer, and Alt+↑/↓ reorders from the
// keyboard (announced via aria-live by the caller).

import { drag, type DragHandle } from "./drag";
import { flip } from "./layout";

export type Direction = "x" | "y";

/** Index the dragged item should occupy: how many other items' centers it has passed. */
export function targetIndex(otherCenters: number[], draggedCenter: number): number {
  return otherCenters.filter((c) => c < draggedCenter).length;
}

export function move<T>(list: readonly T[], from: number, to: number): T[] {
  const next = list.slice();
  const [item] = next.splice(from, 1);
  next.splice(Math.max(0, Math.min(to, next.length)), 0, item);
  return next;
}

export interface ReorderOptions<T> {
  container: () => HTMLElement | undefined;
  items: () => readonly T[];
  key: (item: T) => string;
  /** Live reorder while dragging (optimistic). */
  onReorder: (next: T[]) => void;
  /** Drag finished — persist the final order (e.g. PATCH positions). */
  onCommit?: (final: T[]) => void;
  axis?: Direction;
}

export function createReorder<T>(opts: ReorderOptions<T>) {
  const axis = opts.axis ?? "y";
  const center = (el: Element) => {
    const r = el.getBoundingClientRect();
    return axis === "y" ? r.top + r.height / 2 : r.left + r.width / 2;
  };
  const offset = (el: HTMLElement) => (axis === "y" ? el.offsetTop : el.offsetLeft);
  const indexOf = (item: T) => opts.items().findIndex((i) => opts.key(i) === opts.key(item));

  const reorderTo = (el: HTMLElement, item: T, to: number, handle?: DragHandle) => {
    const container = opts.container();
    const from = indexOf(item);
    if (!container || from < 0 || to === from) return;
    const before = offset(el);
    void flip(container, () => opts.onReorder(move(opts.items(), from, to)));
    if (handle) {
      // Our layout box moved; keep the element visually under the pointer.
      const d = offset(el) - before;
      handle.shift(axis === "x" ? -d : 0, axis === "y" ? -d : 0);
    }
  };

  /** Bind an item element. Use in a Solid ref: `ref={(el) => onCleanup(bind(el, item))}`. */
  return function bind(el: HTMLElement, item: T): () => void {
    el.setAttribute("data-layout", opts.key(item));
    el.setAttribute("aria-roledescription", "перетаскиваемый элемент");
    const handle = drag(el, {
      axis,
      momentum: false,
      elastic: 0.1,
      onStart: () => {
        el.style.zIndex = "10";
        el.setAttribute("aria-grabbed", "true");
      },
      onMove: () => {
        const container = opts.container();
        if (!container) return;
        const others = [...container.querySelectorAll<HTMLElement>("[data-layout]")].filter((x) => x !== el);
        reorderTo(el, item, targetIndex(others.map(center), center(el)), handle);
      },
      // Settle into the new slot.
      onEnd: () => ({ x: 0, y: 0 }),
    });
    const onUp = () => {
      el.style.zIndex = "";
      el.removeAttribute("aria-grabbed");
      opts.onCommit?.(opts.items().slice());
    };
    const onKey = (e: KeyboardEvent) => {
      if (!e.altKey) return;
      const dir = { ArrowUp: -1, ArrowLeft: -1, ArrowDown: 1, ArrowRight: 1 }[e.key];
      if (dir === undefined) return;
      e.preventDefault();
      reorderTo(el, item, indexOf(item) + dir);
      opts.onCommit?.(opts.items().slice());
      el.focus();
    };
    el.addEventListener("lostpointercapture", onUp);
    el.addEventListener("keydown", onKey);
    return () => {
      handle.dispose();
      el.removeEventListener("lostpointercapture", onUp);
      el.removeEventListener("keydown", onKey);
    };
  };
}
