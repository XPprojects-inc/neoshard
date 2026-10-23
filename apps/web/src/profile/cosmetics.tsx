// Profile cosmetics (ADR-0020): NeoShard's own SVG/CSS art for the catalog ids
// in neoshard_proto::profile. Nothing here is copied from another product, and
// nothing is paywalled. Every animation stops with prefers-reduced-motion.

import { For, Show, type JSX } from "solid-js";

import { Flame, Heart, Leaf, Moon, Star, Waves, Zap } from "../icons";
import { DecorationArt } from "./decorations";

export interface CatalogItem {
  id: string;
  label: string;
}

export const DECORATIONS: CatalogItem[] = [
  { id: "orbit", label: "Орбита" },
  { id: "petals", label: "Сакура" },
  { id: "frost", label: "Иней" },
  { id: "flame", label: "Пламя" },
  { id: "circuit", label: "Схема" },
  { id: "stardust", label: "Звёздная пыль" },
  { id: "leaves", label: "Лавры" },
  { id: "crown", label: "Корона" },
  { id: "neon", label: "Неон" },
  { id: "shards", label: "Осколки" },
  { id: "cat_ears", label: "Ушки" },
  { id: "headphones", label: "Наушники" },
  { id: "halo", label: "Нимб" },
  { id: "hearts", label: "Сердечки" },
];

export const EFFECTS: CatalogItem[] = [
  { id: "snowfall", label: "Снегопад" },
  { id: "sparkles", label: "Искры" },
  { id: "aurora", label: "Сияние" },
  { id: "bubbles", label: "Пузыри" },
  { id: "embers", label: "Угольки" },
  { id: "rain", label: "Дождь" },
  { id: "confetti", label: "Конфетти" },
  { id: "starfield", label: "Звёздное небо" },
];

export const NAMEPLATES: CatalogItem[] = [
  { id: "dusk", label: "Сумерки" },
  { id: "lagoon", label: "Лагуна" },
  { id: "ember", label: "Жар" },
  { id: "forest", label: "Лес" },
  { id: "nebula", label: "Туманность" },
  { id: "paper", label: "Бумага" },
  { id: "grid", label: "Сетка" },
  { id: "waves", label: "Волны" },
];

export const NAME_FONTS: CatalogItem[] = [
  { id: "sans", label: "Обычный" },
  { id: "serif", label: "С засечками" },
  { id: "mono", label: "Моноширинный" },
  { id: "rounded", label: "Округлый" },
  { id: "slab", label: "Брусковый" },
  { id: "hand", label: "Рукописный" },
];

export const NAME_EFFECTS: CatalogItem[] = [
  { id: "solid", label: "Цвет" },
  { id: "gradient", label: "Градиент" },
  { id: "neon", label: "Неон" },
  { id: "toon", label: "Контур" },
  { id: "pop", label: "Тень" },
  { id: "shimmer", label: "Перелив" },
];

export const TAG_BADGES: CatalogItem[] = [
  { id: "shard", label: "Осколок" },
  { id: "star", label: "Звезда" },
  { id: "leaf", label: "Лист" },
  { id: "flame", label: "Огонь" },
  { id: "wave", label: "Волна" },
  { id: "bolt", label: "Молния" },
  { id: "heart", label: "Сердце" },
  { id: "moon", label: "Луна" },
];

export const hex = (c: number) => `#${(c >>> 0).toString(16).padStart(6, "0").slice(-6)}`;

let seq = 0;
const uid = (p: string) => `${p}${++seq}`;

/** Deterministic pseudo-random sequence: the same id always draws the same picture. */
function rng(seed: number) {
  let s = seed >>> 0 || 1;
  return () => {
    s ^= s << 13;
    s ^= s >>> 17;
    s ^= s << 5;
    return (s >>> 0) / 4294967296;
  };
}

// Avatar frames. The avatar is a circle of radius ≈37 in the 100×100 viewBox
// (the frame box is 134% of the avatar); frames are drawn around radius 40–48.
function ring(cx: number, cy: number, r: number, i: number, n: number) {
  const a = (i / n) * Math.PI * 2 - Math.PI / 2;
  return { x: cx + Math.cos(a) * r, y: cy + Math.sin(a) * r, a: (a * 180) / Math.PI + 90 };
}

/** Avatar frame art (see decorations.tsx). */
export function Decoration(props: { id: string }): JSX.Element {
  return <DecorationArt id={props.id} />;
}

/** Animated overlay for a profile card. */
export function ProfileEffect(props: { id: string }): JSX.Element {
  const r = rng(props.id.length * 97 + props.id.charCodeAt(0));
  const parts = (n: number) =>
    Array.from({ length: n }, () => ({ x: r() * 100, y: r() * 100, s: r(), d: r() * 6, k: Math.floor(r() * 5) }));
  return (
    <Show when={props.id}>
      <div class={`fx fx-${props.id}`} aria-hidden="true">
        <Show when={props.id === "aurora"}>
          <span class="fx-aurora-a" />
          <span class="fx-aurora-b" />
        </Show>
        <Show when={props.id !== "aurora"}>
          <For each={parts(props.id === "starfield" ? 34 : props.id === "rain" ? 40 : 26)}>
            {(p) => (
              <i
                class={`fx-p fx-k${p.k}`}
                style={{
                  left: `${p.x}%`,
                  top: props.id === "starfield" || props.id === "sparkles" ? `${p.y}%` : undefined,
                  "--s": p.s.toFixed(2),
                  "animation-delay": `${-p.d.toFixed(2)}s`,
                }}
              />
            )}
          </For>
        </Show>
      </div>
    </Show>
  );
}

export function TagBadge(props: { id: string; size?: number }): JSX.Element {
  const size = () => props.size ?? 12;
  switch (props.id) {
    case "star":
      return <Star size={size()} fill="currentColor" />;
    case "leaf":
      return <Leaf size={size()} />;
    case "flame":
      return <Flame size={size()} />;
    case "wave":
      return <Waves size={size()} />;
    case "bolt":
      return <Zap size={size()} fill="currentColor" />;
    case "heart":
      return <Heart size={size()} fill="currentColor" />;
    case "moon":
      return <Moon size={size()} fill="currentColor" />;
    default:
      return (
        <svg width={size()} height={size()} viewBox="0 0 24 24" aria-hidden="true">
          <path d="M12 2l7 6-3 14H8L5 8z" fill="currentColor" />
        </svg>
      );
  }
}

const FONT_STACK: Record<string, string> = {
  serif: 'Georgia, "Times New Roman", "PT Serif", serif',
  mono: "var(--ns-font-mono)",
  rounded: 'ui-rounded, "SF Pro Rounded", "Nunito", "Segoe UI Variable Display", system-ui, sans-serif',
  slab: 'Rockwell, "Roboto Slab", "Courier New", serif',
  hand: '"Segoe Print", "Bradley Hand", "Comic Sans MS", cursive',
};

/** Inline style + class for a display name in a given style. */
export function nameStyle(s: { font: string; effect: string; colors: number[] } | undefined): { class: string; style: JSX.CSSProperties } {
  if (!s) return { class: "", style: {} };
  const [c1, c2] = [s.colors[0] ?? 0x2ec4a6, s.colors[1] ?? s.colors[0] ?? 0x9d7bff].map(hex);
  const style: JSX.CSSProperties = { "--n1": c1, "--n2": c2 } as JSX.CSSProperties;
  if (FONT_STACK[s.font]) style["font-family"] = FONT_STACK[s.font];
  const effect = s.effect || (s.colors.length ? "solid" : "");
  return { class: effect ? `nm nm-${effect}` : "", style };
}
