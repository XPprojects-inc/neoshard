// Which server this client talks to. In the desktop and Android apps the page
// origin is the app itself (tauri.localhost), never a server, so links that
// leave the app (invites) must use this instead of location.origin.

import { isTauri } from "@neoshard/core";

import config from "../neoshard.config.json";

export const INSTANCE_KEY = "neoshard.instance";

/** Server the build is tied to (VITE_NEOSHARD_INSTANCE); when set, the server field is hidden. */
export const FIXED_INSTANCE = (import.meta.env.VITE_NEOSHARD_INSTANCE ?? "").trim().replace(/\/+$/, "");

/** The server picked at sign-in (or the build's), "" if unknown yet. */
export function instanceUrl(): string {
  if (FIXED_INSTANCE) return FIXED_INSTANCE;
  try {
    const saved = localStorage.getItem(INSTANCE_KEY);
    if (saved) return saved.replace(/\/+$/, "");
  } catch {
    // ignore
  }
  return config.defaultInstance || (isTauri() ? "" : location.origin);
}

/** A link others can open: https://server/invite/CODE (the server redirects it into the web client). */
export function inviteLink(code: string): string {
  return `${instanceUrl() || location.origin}/invite/${code}`;
}

/** Invite code from a pasted link or a bare code. */
export function inviteCode(text: string): string {
  const t = text.trim();
  const m = /(?:\/invite\/|[?&]invite=)([A-Za-z0-9_-]+)/.exec(t);
  return m ? m[1] : t.split("/").pop() ?? "";
}
