// Anchored overlays: Popover, Menu (also as a context menu), Tooltip.
// Positioned with plain geometry (no layout library), kept inside the viewport,
// Esc / outside click close them and focus returns to where it was.

import { createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { Portal } from "solid-js/web";

import { Presence, presets } from "../motion/presence";

export type Placement = "right" | "left" | "bottom" | "top" | "bottom-start" | "bottom-end" | "top-start";
export type Anchor = HTMLElement | DOMRect | { x: number; y: number };

const GAP = 8;
const MARGIN = 8;

function rectOf(a: Anchor): DOMRect {
  if (a instanceof HTMLElement) return a.getBoundingClientRect();
  if (a instanceof DOMRect) return a;
  return new DOMRect(a.x, a.y, 0, 0);
}

/** Top-left for a box of size w×h next to `r`, flipped/clamped into the viewport. */
export function place(r: DOMRect, w: number, h: number, p: Placement, vw = window.innerWidth, vh = window.innerHeight): { x: number; y: number } {
  let x: number;
  let y: number;
  switch (p) {
    case "right":
      x = r.right + GAP;
      y = r.top;
      if (x + w > vw - MARGIN) x = r.left - GAP - w;
      break;
    case "left":
      x = r.left - GAP - w;
      y = r.top;
      if (x < MARGIN) x = r.right + GAP;
      break;
    case "top":
    case "top-start":
      x = p === "top" ? r.left + r.width / 2 - w / 2 : r.left;
      y = r.top - GAP - h;
      if (y < MARGIN) y = r.bottom + GAP;
      break;
    case "bottom-end":
      x = r.right - w;
      y = r.bottom + GAP;
      if (y + h > vh - MARGIN) y = r.top - GAP - h;
      break;
    case "bottom-start":
      x = r.left;
      y = r.bottom + GAP;
      if (y + h > vh - MARGIN) y = r.top - GAP - h;
      break;
    default:
      x = r.left + r.width / 2 - w / 2;
      y = r.bottom + GAP;
      if (y + h > vh - MARGIN) y = r.top - GAP - h;
  }
  return {
    x: Math.round(Math.min(Math.max(MARGIN, x), Math.max(MARGIN, vw - w - MARGIN))),
    y: Math.round(Math.min(Math.max(MARGIN, y), Math.max(MARGIN, vh - h - MARGIN))),
  };
}

const FOCUSABLE = 'button:not([disabled]), [href], input:not([disabled]), select, textarea, [tabindex]:not([tabindex="-1"])';

export function Popover(props: {
  open: boolean;
  anchor?: Anchor;
  placement?: Placement;
  onClose: () => void;
  label: string;
  class?: string;
  role?: "dialog" | "menu";
  children: JSX.Element;
}) {
  return (
    <Portal>
      <Presence enter={presets.pop.enter} exit={presets.pop.exit}>
        <Show when={props.open && props.anchor}>
          <PopoverBody {...props} anchor={props.anchor!} />
        </Show>
      </Presence>
    </Portal>
  );
}

function PopoverBody(props: {
  anchor: Anchor;
  placement?: Placement;
  onClose: () => void;
  label: string;
  class?: string;
  role?: "dialog" | "menu";
  children: JSX.Element;
}) {
  let el!: HTMLDivElement;
  const previous = document.activeElement as HTMLElement | null;
  const [pos, setPos] = createSignal({ x: -9999, y: -9999 });
  const reposition = () => {
    if (!el) return;
    setPos(place(rectOf(props.anchor), el.offsetWidth, el.offsetHeight, props.placement ?? "bottom"));
  };
  const onDown = (e: PointerEvent) => {
    if (!el.contains(e.target as Node) && !(props.anchor instanceof HTMLElement && props.anchor.contains(e.target as Node))) props.onClose();
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      props.onClose();
    }
  };
  onMount(() => {
    reposition();
    const ro = new ResizeObserver(reposition);
    ro.observe(el);
    window.addEventListener("resize", reposition);
    document.addEventListener("pointerdown", onDown, true);
    requestAnimationFrame(() => {
      reposition();
      if (props.role !== "menu") (el.querySelector<HTMLElement>("[data-autofocus]") ?? el).focus({ preventScroll: true });
    });
    onCleanup(() => {
      ro.disconnect();
      window.removeEventListener("resize", reposition);
      document.removeEventListener("pointerdown", onDown, true);
      if (previous?.isConnected) previous.focus({ preventScroll: true });
    });
  });
  return (
    <div
      ref={el}
      class={`ns-popover ${props.class ?? ""}`}
      role={props.role ?? "dialog"}
      aria-label={props.label}
      tabindex="-1"
      style={{ left: `${pos().x}px`, top: `${pos().y}px` }}
      onKeyDown={onKey}
    >
      {props.children}
    </div>
  );
}

export interface MenuItem {
  label?: string;
  icon?: JSX.Element;
  hint?: string;
  danger?: boolean;
  disabled?: boolean;
  checked?: boolean;
  separator?: boolean;
  onSelect?: () => void;
}

/** Menu (role=menu): ↑/↓/Home/End move, Enter/Space activate, Esc closes. */
export function Menu(props: { open: boolean; anchor?: Anchor; placement?: Placement; onClose: () => void; label: string; items: MenuItem[] }) {
  let list!: HTMLDivElement;
  const items = () => [...(list?.querySelectorAll<HTMLElement>('[role^="menuitem"]:not([aria-disabled="true"])') ?? [])];
  const move = (d: number) => {
    const all = items();
    if (!all.length) return;
    const i = all.indexOf(document.activeElement as HTMLElement);
    all[(i + d + all.length) % all.length].focus();
  };
  return (
    <Popover open={props.open} anchor={props.anchor} placement={props.placement ?? "bottom-start"} onClose={props.onClose} label={props.label} role="menu" class="ns-menu">
      <div
        ref={(e) => {
          list = e;
          requestAnimationFrame(() => items()[0]?.focus({ preventScroll: true }));
        }}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown") (e.preventDefault(), move(1));
          else if (e.key === "ArrowUp") (e.preventDefault(), move(-1));
          else if (e.key === "Home") (e.preventDefault(), items()[0]?.focus());
          else if (e.key === "End") (e.preventDefault(), items().at(-1)?.focus());
          else if (e.key === "Tab") props.onClose();
        }}
      >
        <For each={props.items}>
          {(it) =>
            it.separator ? (
              <div class="ns-menu-sep" role="separator" />
            ) : (
              <button
                type="button"
                role={it.checked === undefined ? "menuitem" : "menuitemcheckbox"}
                aria-checked={it.checked === undefined ? undefined : it.checked}
                aria-disabled={it.disabled ? "true" : undefined}
                class="ns-menu-item"
                classList={{ danger: !!it.danger }}
                tabindex="-1"
                onClick={() => {
                  if (it.disabled) return;
                  props.onClose();
                  it.onSelect?.();
                }}
              >
                <span class="ns-menu-label">{it.label}</span>
                <Show when={it.hint}>
                  <small>{it.hint}</small>
                </Show>
                <Show when={it.icon}>
                  <span class="ns-menu-icon" aria-hidden="true">{it.icon}</span>
                </Show>
              </button>
            )
          }
        </For>
      </div>
    </Popover>
  );
}

/** Shared state for one context menu per screen. */
export function createContextMenu() {
  const [state, setState] = createSignal<{ at: { x: number; y: number }; items: MenuItem[]; label: string }>();
  return {
    open: (e: MouseEvent | KeyboardEvent, items: MenuItem[], label = "Действия") => {
      e.preventDefault();
      const at =
        e instanceof MouseEvent && e.clientX + e.clientY > 0
          ? { x: e.clientX, y: e.clientY }
          : (() => {
              const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
              return { x: r.left + 16, y: r.top + 16 };
            })();
      setState({ at, items, label });
    },
    view: () => (
      <Menu open={!!state()} anchor={state()?.at} placement="bottom-start" onClose={() => setState(undefined)} label={state()?.label ?? ""} items={state()?.items ?? []} />
    ),
  };
}

let tipSeq = 0;
/** Tooltip on hover and keyboard focus; the label is also the accessible description. */
export function Tooltip(props: { label: string; placement?: Placement; children: JSX.Element }) {
  const id = `ns-tip-${++tipSeq}`;
  let host!: HTMLSpanElement;
  const [open, setOpen] = createSignal(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  const show = () => {
    clearTimeout(timer);
    timer = setTimeout(() => setOpen(true), 350);
  };
  const hide = () => {
    clearTimeout(timer);
    setOpen(false);
  };
  onCleanup(hide);
  return (
    <span ref={host} class="ns-tip-host" aria-describedby={open() ? id : undefined} onPointerEnter={show} onPointerLeave={hide} onFocusIn={show} onFocusOut={hide}>
      {props.children}
      <Show when={open()}>
        <Portal>
          <TipBubble id={id} label={props.label} anchor={host} placement={props.placement ?? "top"} />
        </Portal>
      </Show>
    </span>
  );
}

function TipBubble(props: { id: string; label: string; anchor: HTMLElement; placement: Placement }) {
  let el!: HTMLDivElement;
  const [pos, setPos] = createSignal({ x: -9999, y: -9999 });
  onMount(() => setPos(place(props.anchor.getBoundingClientRect(), el.offsetWidth, el.offsetHeight, props.placement)));
  return (
    <div ref={el} id={props.id} role="tooltip" class="ns-tooltip" style={{ left: `${pos().x}px`, top: `${pos().y}px` }}>
      {props.label}
    </div>
  );
}

export function Switch(props: { checked: boolean; onChange: (v: boolean) => void; label: string; description?: string; disabled?: boolean }) {
  return (
    <button type="button" role="switch" aria-checked={props.checked} class="ns-switch-row" disabled={props.disabled} onClick={() => props.onChange(!props.checked)}>
      <span class="ns-switch-text">
        <span>{props.label}</span>
        <Show when={props.description}>
          <small>{props.description}</small>
        </Show>
      </span>
      <span class="ns-switch" aria-hidden="true">
        <span />
      </span>
    </button>
  );
}
