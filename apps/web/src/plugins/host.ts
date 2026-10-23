// Client plugins (ADR-0015), exteraGram-style but sandboxed:
// - each plugin runs in <iframe sandbox="allow-scripts" src="/plugin-sandbox.html">:
//   an opaque origin with no access to the token, storage or the app's DOM,
//   and a CSP with no network access at all;
// - the plugin code is pinned by SHA-256 at install time and verified before
//   every start, so it cannot change after the user approved its permissions;
// - the host only forwards what the manifest's permissions allow.

import type { Update } from "@neoshard/core";

import { core, sid, state } from "../state";

export type PluginPermission = "messages.read" | "messages.send";

export interface PluginManifest {
  name: string;
  version: string;
  permissions: PluginPermission[];
}

interface Installed {
  url: string;
  sha256: string;
  manifest: PluginManifest;
}

interface Running extends Installed {
  frame: HTMLIFrameElement;
  sent: number[];
}

const KEY = "neoshard.plugins";
const ALLOWED: PluginPermission[] = ["messages.read", "messages.send"];
const SEND_LIMIT = 5; // messages per 10 s per plugin
const running = new Map<string, Running>();
let listening = false;

/** Validate a manifest from an untrusted plugin. */
export function parseManifest(raw: unknown): PluginManifest | null {
  if (!raw || typeof raw !== "object") return null;
  const m = raw as Record<string, unknown>;
  if (typeof m.name !== "string" || !m.name.trim() || m.name.length > 64) return null;
  const perms = Array.isArray(m.permissions) ? m.permissions : [];
  if (!perms.every((p) => ALLOWED.includes(p as PluginPermission))) return null;
  return { name: m.name.trim(), version: typeof m.version === "string" ? m.version.slice(0, 32) : "0.0.0", permissions: [...new Set(perms)] as PluginPermission[] };
}

function validInstalled(x: unknown): x is Installed {
  if (!x || typeof x !== "object") return false;
  const i = x as Installed;
  return typeof i.url === "string" && /^https:\/\/\S+\.js$/.test(i.url) && typeof i.sha256 === "string" && /^[0-9a-f]{64}$/.test(i.sha256) && parseManifest(i.manifest) !== null;
}

export function listPlugins(): Installed[] {
  try {
    const raw = JSON.parse(localStorage.getItem(KEY) ?? "[]");
    return Array.isArray(raw) ? raw.filter(validInstalled) : [];
  } catch {
    return [];
  }
}

function save(list: Installed[]) {
  localStorage.setItem(KEY, JSON.stringify(list));
}

export async function sha256Hex(text: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

async function fetchText(url: string): Promise<string> {
  const res = await fetch(url, { credentials: "omit", referrerPolicy: "no-referrer" });
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  const text = await res.text();
  if (text.length > 512 * 1024) throw new Error("плагин слишком большой");
  return text;
}

export async function installPlugin(url: string): Promise<void> {
  if (!/^https:\/\/\S+\.js$/.test(url)) throw new Error("Нужен https-адрес .js файла");
  const manifest = parseManifest(JSON.parse(await fetchText(url.replace(/\.js$/, ".json"))));
  if (!manifest) throw new Error("Некорректный манифест плагина");
  const code = await fetchText(url);
  const sha256 = await sha256Hex(code);
  if (!confirm(`Установить «${manifest.name}» ${manifest.version}?\nРазрешения: ${manifest.permissions.join(", ") || "нет"}\nSHA-256: ${sha256.slice(0, 16)}…`)) return;
  save([...listPlugins().filter((p) => p.url !== url), { url, sha256, manifest }]);
  await start({ url, sha256, manifest }, code);
}

export function removePlugin(url: string): void {
  save(listPlugins().filter((p) => p.url !== url));
  running.get(url)?.frame.remove();
  running.delete(url);
}

async function start(p: Installed, preloaded?: string): Promise<void> {
  if (running.has(p.url)) return;
  const code = preloaded ?? (await fetchText(p.url));
  if ((await sha256Hex(code)) !== p.sha256) {
    console.warn(`plugin ${p.manifest.name}: code changed since installation, not started`);
    return;
  }
  const frame = document.createElement("iframe");
  frame.setAttribute("sandbox", "allow-scripts"); // no allow-same-origin
  frame.setAttribute("referrerpolicy", "no-referrer");
  frame.setAttribute("aria-hidden", "true");
  frame.style.display = "none";
  frame.src = `${import.meta.env.BASE_URL}plugin-sandbox.html`;
  const r: Running = { ...p, frame, sent: [] };
  running.set(p.url, r);
  pendingCode.set(frame, code);
  document.body.append(frame);
}

const pendingCode = new WeakMap<HTMLIFrameElement, string>();

export function startInstalledPlugins(): void {
  if (!listening) {
    window.addEventListener("message", onMessage);
    listening = true;
  }
  for (const p of listPlugins()) void start(p).catch((e) => console.warn("plugin start failed", e));
}

/** Tear everything down (logout). */
export function stopAllPlugins(): void {
  for (const r of running.values()) r.frame.remove();
  running.clear();
}

function pluginFor(source: MessageEventSource | null): Running | undefined {
  for (const r of running.values()) if (r.frame.contentWindow === source) return r;
  return undefined;
}

function onMessage(e: MessageEvent) {
  const p = pluginFor(e.source);
  if (!p || typeof e.data !== "object" || e.data === null) return;
  const msg = e.data as { type?: unknown; channelId?: unknown; content?: unknown };
  if (msg.type === "ready") {
    const code = pendingCode.get(p.frame);
    pendingCode.delete(p.frame);
    if (code) p.frame.contentWindow?.postMessage({ type: "load", code }, "*");
    return;
  }
  if (msg.type === "send-message" && p.manifest.permissions.includes("messages.send")) {
    const now = Date.now();
    p.sent = p.sent.filter((t) => now - t < 10_000);
    if (p.sent.length >= SEND_LIMIT) return;
    const cid = typeof msg.channelId === "string" && /^\d{1,20}$/.test(msg.channelId) ? msg.channelId : state.selection.channel;
    if (!cid || typeof msg.content !== "string" || !msg.content.trim()) return;
    p.sent.push(now);
    // Length limits and permissions are enforced by the core and the server.
    core().send({ case: "sendMessage", value: { channelId: BigInt(cid), content: msg.content, replyToId: 0n } });
  }
}

/** Forward permitted events to plugins (called with each update batch). */
export function forwardToPlugins(updates: Update[]): void {
  if (running.size === 0) return;
  for (const u of updates) {
    if (u.kind.case !== "event" || u.kind.value.kind.case !== "messageCreate") continue;
    const m = u.kind.value.kind.value;
    const payload = { type: "message", id: sid(m.id), channelId: sid(m.channelId), author: m.author?.username ?? "", content: m.content };
    for (const r of running.values()) {
      if (r.manifest.permissions.includes("messages.read")) r.frame.contentWindow?.postMessage(payload, "*");
    }
  }
}
