// Desktop notifications for mentions and DMs while the tab is hidden.
// Uses the browser Notification API only: nothing leaves the device.

import type { Update } from "@neoshard/core";
import { RelationshipKind, Status } from "@neoshard/core";

import { parseManifest } from "./e2eeFiles";
import { prefs } from "./prefs";
import { sid, state } from "./state";

let ctx: AudioContext | undefined;

function ping() {
  if (!prefs.notificationSound) return;
  try {
    ctx ??= new AudioContext();
    const o = ctx.createOscillator();
    const g = ctx.createGain();
    o.frequency.value = 880;
    g.gain.setValueAtTime(0.0001, ctx.currentTime);
    g.gain.exponentialRampToValueAtTime(0.08, ctx.currentTime + 0.01);
    g.gain.exponentialRampToValueAtTime(0.0001, ctx.currentTime + 0.25);
    o.connect(g).connect(ctx.destination);
    o.start();
    o.stop(ctx.currentTime + 0.3);
  } catch {
    // audio is optional
  }
}

export function notify(updates: Update[]): void {
  if (document.visibilityState === "visible" || state.myPresence.status === Status.DND) return;
  for (const u of updates) {
    if (u.kind.case === "event" && u.kind.value.kind.case === "relationshipAdd" && u.kind.value.kind.value.kind === RelationshipKind.INCOMING) {
      const who = u.kind.value.kind.value.user;
      ping();
      if (prefs.desktopNotifications && typeof Notification !== "undefined" && Notification.permission === "granted")
        new Notification("Заявка в друзья", { body: `${who?.displayName || who?.username} хочет добавить вас в друзья`, silent: true });
      continue;
    }
    if (u.kind.case !== "event" || u.kind.value.kind.case !== "messageCreate") continue;
    const m = u.kind.value.kind.value;
    if (!m.author || m.author.id === state.me?.id) continue;
    const dm = m.guildId === 0n;
    const mention = m.mentionEveryone || m.mentionUserIds.some((x) => x === state.me?.id);
    if (!dm && !mention) continue;
    ping();
    if (!prefs.desktopNotifications || typeof Notification === "undefined" || Notification.permission !== "granted") continue;
    const who = m.author.displayName || m.author.username;
    const where = dm ? "" : ` · #${state.guilds[sid(m.guildId)]?.channels.find((c) => c.id === m.channelId)?.name ?? ""}`;
    const text = m.e2eePayload.length ? parseManifest(m.content).text : m.content;
    const n = new Notification(`${who}${where}`, { body: text ? text.slice(0, 180) : m.attachments.length ? "📎 файл" : "🔒 зашифрованное сообщение", tag: sid(m.channelId), silent: true });
    n.onclick = () => {
      window.focus();
      n.close();
    };
  }
}
