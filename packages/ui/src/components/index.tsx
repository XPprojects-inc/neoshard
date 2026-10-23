// Primitive components. Accessible by default: real buttons, labelled fields,
// focus-trapped modals, polite toasts.

import { createEffect, createSignal, For, on, onCleanup, onMount, Show, splitProps, type JSX } from "solid-js";
import { Portal } from "solid-js/web";

import { Presence, presets } from "../motion/presence";

type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";

export function Button(props: JSX.ButtonHTMLAttributes<HTMLButtonElement> & { variant?: ButtonVariant; size?: "sm" | "md"; loading?: boolean }) {
  const [local, rest] = splitProps(props, ["variant", "size", "loading", "class", "children", "disabled"]);
  return (
    <button
      type="button"
      {...rest}
      class={`ns-btn ns-btn--${local.variant ?? "primary"} ${local.size === "sm" ? "ns-btn--sm" : ""} ${local.class ?? ""}`}
      disabled={local.disabled || local.loading}
      aria-busy={local.loading ? "true" : undefined}
    >
      <Show when={local.loading}>
        <span class="ns-spinner" aria-hidden="true" />
      </Show>
      {local.children}
    </button>
  );
}

export function IconButton(props: JSX.ButtonHTMLAttributes<HTMLButtonElement> & { label: string }) {
  const [local, rest] = splitProps(props, ["label", "children", "class"]);
  return (
    <button type="button" {...rest} class={`ns-icon-btn ${local.class ?? ""}`} aria-label={local.label} title={local.label}>
      {local.children}
    </button>
  );
}

let fieldSeq = 0;

/** Multi-line field with a live character counter. */
export function TextArea(props: { label: string; value: string; onInput: (v: string) => void; max: number; rows?: number; placeholder?: string; hint?: string }) {
  const id = `ns-f-${++fieldSeq}`;
  const left = () => props.max - [...props.value].length;
  return (
    <div class="ns-field">
      <label class="ns-label" for={id}>
        {props.label}
      </label>
      <textarea
        id={id}
        class="ns-input ns-textarea"
        rows={props.rows ?? 4}
        value={props.value}
        placeholder={props.placeholder}
        aria-describedby={`${id}-count`}
        onInput={(e) => props.onInput(e.currentTarget.value)}
      />
      <span id={`${id}-count`} class="ns-field-hint" classList={{ over: left() < 0 }}>
        <Show when={props.hint}>{props.hint} · </Show>
        {left() >= 0 ? `осталось ${left()}` : `лишних символов: ${-left()}`}
      </span>
    </div>
  );
}

/** Colour input with a hex label; value is 0xRRGGBB. */
export function ColorField(props: { label: string; value: number; onInput: (v: number) => void }) {
  const id = `ns-f-${++fieldSeq}`;
  const hex = () => `#${props.value.toString(16).padStart(6, "0")}`;
  return (
    <label class="ns-color" for={id}>
      <input id={id} type="color" value={hex()} onInput={(e) => props.onInput(parseInt(e.currentTarget.value.slice(1), 16))} />
      <span>
        {props.label}
        <small>{hex()}</small>
      </span>
    </label>
  );
}
export function TextField(props: {
  label: string;
  value: string;
  onInput: (v: string) => void;
  type?: string;
  error?: string;
  autocomplete?: string;
  required?: boolean;
  autofocus?: boolean;
  inputmode?: "text" | "numeric" | "email";
  placeholder?: string;
}) {
  const id = `ns-f-${++fieldSeq}`;
  return (
    <div class="ns-field">
      <label class="ns-label" for={id}>
        {props.label}
      </label>
      <input
        id={id}
        class="ns-input"
        type={props.type ?? "text"}
        value={props.value}
        onInput={(e) => props.onInput(e.currentTarget.value)}
        aria-invalid={props.error ? "true" : undefined}
        aria-describedby={props.error ? `${id}-err` : undefined}
        autocomplete={props.autocomplete}
        required={props.required}
        autofocus={props.autofocus}
        inputmode={props.inputmode}
        placeholder={props.placeholder}
      />
      <Show when={props.error}>
        <span id={`${id}-err`} class="ns-error" role="alert">
          {props.error}
        </span>
      </Show>
    </div>
  );
}

const AVATAR_COLORS = ["#2a7f6f", "#7b5ea7", "#b5563a", "#3d6fb6", "#8a6d1f", "#a64570", "#4a7d3b"];

export function avatarColor(seed: string): string {
  let h = 0;
  for (const ch of seed) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  return AVATAR_COLORS[h % AVATAR_COLORS.length];
}

export type StatusName = "online" | "idle" | "dnd" | "offline";
const STATUS_LABEL: Record<StatusName, string> = { online: "в сети", idle: "неактивен", dnd: "не беспокоить", offline: "не в сети" };

export function Avatar(props: { name: string; src?: string; size?: number; status?: StatusName; decoration?: JSX.Element; seed?: string }) {
  const size = () => props.size ?? 40;
  const [broken, setBroken] = createSignal(false);
  createEffect(on(() => props.src, () => setBroken(false), { defer: true }));
  const src = () => (broken() ? undefined : props.src);
  return (
    <span
      class="ns-avatar"
      style={{
        width: `${size()}px`,
        height: `${size()}px`,
        background: src() ? "transparent" : avatarColor(props.seed ?? props.name),
        "font-size": `${size() * 0.4}px`,
        "--ns-avatar-size": `${size()}px`,
      }}
    >
      <Show when={src()} fallback={<span aria-hidden="true">{(props.name[0] ?? "?").toUpperCase()}</span>}>
        <img src={src()} alt="" loading="lazy" decoding="async" referrerpolicy="no-referrer" onError={() => setBroken(true)} />
      </Show>
      <Show when={props.decoration}>
        <span class="ns-avatar-deco" aria-hidden="true">{props.decoration}</span>
      </Show>
      <Show when={props.status}>
        {(s) => (
          <>
            <span class={`ns-status ns-status--${s()}`} aria-hidden="true" />
            <span class="ns-sr-only">{STATUS_LABEL[s()]}</span>
          </>
        )}
      </Show>
    </span>
  );
}

/** Modal dialog: Presence enter/exit, focus trap, Esc to close, focus restore. */
export function Modal(props: { open: boolean; title: string; onClose: () => void; children: JSX.Element }) {
  return (
    <Portal>
      <Presence enter={presets.pop.enter} exit={presets.pop.exit}>
        <Show when={props.open}>
          <ModalBody title={props.title} onClose={props.onClose}>
            {props.children}
          </ModalBody>
        </Show>
      </Presence>
    </Portal>
  );
}

function ModalBody(props: { title: string; onClose: () => void; children: JSX.Element }) {
  let dialog!: HTMLDivElement;
  const previous = document.activeElement as HTMLElement | null;
  const focusables = () => [...dialog.querySelectorAll<HTMLElement>('button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])')].filter((e) => !e.hasAttribute("disabled"));
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      props.onClose();
    } else if (e.key === "Tab") {
      const f = focusables();
      if (f.length === 0) return;
      const [first, last] = [f[0], f[f.length - 1]];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    }
  };
  // Next frame: inside <Presence> the node is attached after mount.
  onMount(() => requestAnimationFrame(() => (focusables()[0] ?? dialog).focus()));
  onCleanup(() => previous?.focus?.());
  return (
    <div class="ns-backdrop" onClick={(e) => e.target === e.currentTarget && props.onClose()}>
      <div ref={dialog} class="ns-modal" role="dialog" aria-modal="true" aria-labelledby="ns-modal-title" tabindex="-1" onKeyDown={onKey}>
        <h2 id="ns-modal-title">{props.title}</h2>
        {props.children}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------- toasts

export interface ToastItem {
  id: number;
  text: string;
  kind: "info" | "error";
}

const [toasts, setToasts] = createSignal<ToastItem[]>([]);
let toastSeq = 0;

export function toast(text: string, kind: ToastItem["kind"] = "info", ms = 4000): void {
  const id = ++toastSeq;
  setToasts((t) => [...t.slice(-4), { id, text, kind }]);
  setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), ms);
}

export function Toasts() {
  return (
    <Portal>
      <div class="ns-toasts" role="status" aria-live="polite">
        <Presence enter={presets.rise.enter} exit={presets.rise.exit}>
          <For each={toasts()}>{(t) => <div class={`ns-toast ns-toast--${t.kind}`}>{t.text}</div>}</For>
        </Presence>
      </div>
    </Portal>
  );
}

export * from "./overlay";

export function Spinner(props: { label?: string }) {
  return (
    <span role="status" style={{ display: "inline-flex", gap: "8px", "align-items": "center" }}>
      <span class="ns-spinner" aria-hidden="true" />
      <span class="ns-sr-only">{props.label ?? "Загрузка"}</span>
    </span>
  );
}
