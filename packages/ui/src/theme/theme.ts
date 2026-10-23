// Themes are plain JSON → CSS custom properties (`--ns-<role>`). Forks ship
// their own JSON in packages/ui/themes/ or load one at runtime.

export interface Theme {
  name: string;
  scheme: "dark" | "light";
  colors: Record<string, string>;
}

import dark from "../../themes/neoshard-dark.json";
import light from "../../themes/neoshard-light.json";
import midnight from "../../themes/neoshard-midnight.json";

export const builtinThemes: Record<string, Theme> = {
  "neoshard-dark": dark as Theme,
  "neoshard-light": light as Theme,
  "neoshard-midnight": midnight as Theme,
};

const HEX = /^#(?:[0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})$/i;

/** Validate a theme from an untrusted source (user-installed JSON). */
export function parseTheme(raw: unknown): Theme | null {
  if (!raw || typeof raw !== "object") return null;
  const t = raw as Partial<Theme>;
  if (typeof t.name !== "string" || (t.scheme !== "dark" && t.scheme !== "light") || !t.colors || typeof t.colors !== "object") return null;
  const colors: Record<string, string> = {};
  for (const [k, v] of Object.entries(t.colors)) {
    // Only role names and hex colours: no url(), no expressions, no injection.
    if (/^[a-z][a-z0-9-]{0,31}$/.test(k) && typeof v === "string" && HEX.test(v)) colors[k] = v;
  }
  return { name: t.name.slice(0, 64), scheme: t.scheme, colors };
}

export function themeToCss(theme: Theme): Record<string, string> {
  const vars: Record<string, string> = {};
  for (const [k, v] of Object.entries(theme.colors)) vars[`--ns-${k}`] = v;
  return vars;
}

/** Apply a theme to the document root (or another element). */
export function applyTheme(theme: Theme, root: HTMLElement = document.documentElement): void {
  for (const [k, v] of Object.entries(themeToCss(theme))) root.style.setProperty(k, v);
  root.style.colorScheme = theme.scheme;
  root.dataset.theme = theme.name;
}
