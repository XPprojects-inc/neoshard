// Phone layout state and Material-style touch feedback (ripples).

import { createSignal, onCleanup } from "solid-js";

const query = typeof matchMedia === "function" ? matchMedia("(max-width: 767px)") : undefined;
const [mobile, setMobile] = createSignal(query?.matches ?? false);
query?.addEventListener("change", (e) => setMobile(e.matches));

/** True on phone-sized screens (Material layout). */
export const isMobile = mobile;

export type MobileTab = "chats" | "friends" | "you";
const [tab, setTab] = createSignal<MobileTab>("chats");
export const mobileTab = tab;
export const setMobileTab = setTab;

const RIPPLE_TARGETS = "button, a, .member, .channel, .friend-row, .m3-tap";

/** Material ripple on press for the subtree of `root` (pointer only, honours reduced motion). */
export function installRipples(root: HTMLElement): void {
  if (matchMedia("(prefers-reduced-motion: reduce)").matches) return;
  const onDown = (e: PointerEvent) => {
    const el = (e.target as HTMLElement).closest<HTMLElement>(RIPPLE_TARGETS);
    if (!el || !root.contains(el) || el.matches(":disabled")) return;
    const r = el.getBoundingClientRect();
    const size = Math.max(r.width, r.height) * 2;
    const dot = document.createElement("span");
    dot.className = "m3-ripple";
    dot.style.width = dot.style.height = `${size}px`;
    dot.style.left = `${e.clientX - r.left - size / 2}px`;
    dot.style.top = `${e.clientY - r.top - size / 2}px`;
    el.classList.add("m3-ripple-host");
    el.appendChild(dot);
    dot.addEventListener("animationend", () => dot.remove(), { once: true });
  };
  root.addEventListener("pointerdown", onDown);
  onCleanup(() => root.removeEventListener("pointerdown", onDown));
}
