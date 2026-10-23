// Calls in DMs and group DMs: the incoming-call card and its ringtone.
// A call is just the DM's voice room (LiveKit); the server tells every participant
// who is in it (VoiceStateUpdate with guild 0), and that is what rings here.

import { createEffect, createMemo, createSignal, onCleanup, Show } from "solid-js";

import { Phone, PhoneOff } from "../icons";
import { UserAvatar } from "../profile/components";
import { sid, state, type Id } from "../state";

/** Two soft alternating tones, synthesized (no audio files). Returns a stop function. */
function startRingtone(): () => void {
  const Ctx = window.AudioContext ?? (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
  if (!Ctx) return () => {};
  const ctx = new Ctx();
  const out = ctx.createGain();
  out.gain.value = 0.12;
  out.connect(ctx.destination);
  const ring = () => {
    const t0 = ctx.currentTime;
    [523.25, 659.25, 523.25, 659.25].forEach((f, i) => {
      const o = ctx.createOscillator();
      const g = ctx.createGain();
      o.type = "sine";
      o.frequency.value = f;
      const s = t0 + i * 0.22;
      g.gain.setValueAtTime(0, s);
      g.gain.linearRampToValueAtTime(1, s + 0.02);
      g.gain.exponentialRampToValueAtTime(0.001, s + 0.2);
      o.connect(g).connect(out);
      o.start(s);
      o.stop(s + 0.21);
    });
  };
  ring();
  const timer = setInterval(ring, 2200);
  return () => {
    clearInterval(timer);
    void ctx.close();
  };
}

/** The DM call we are being invited to: others are in it, we are not, and we have not declined it. */
export function IncomingCall(props: { activeCall?: Id; onAccept: (channelId: Id) => void }) {
  const [declined, setDeclined] = createSignal<Set<Id>>(new Set());
  const me = () => state.me?.id;
  const incoming = createMemo(() => {
    for (const [cid, people] of Object.entries(state.dmVoice)) {
      if (cid === props.activeCall || declined().has(cid) || !state.dms[cid]) continue;
      if (people.some((v) => v.userId === me())) continue;
      if (people.some((v) => v.userId !== me())) return cid;
    }
    return undefined;
  });
  // A declined call can ring again once it has ended and someone calls anew.
  createEffect(() => {
    const live = new Set(Object.keys(state.dmVoice));
    const d = declined();
    if ([...d].some((cid) => !live.has(cid))) setDeclined(new Set([...d].filter((cid) => live.has(cid))));
  });
  createEffect(() => {
    const cid = incoming();
    if (!cid) return;
    const stop = startRingtone();
    // Stop ringing after 45 s even if nobody answers.
    const giveUp = setTimeout(() => decline(cid), 45_000);
    onCleanup(() => {
      clearTimeout(giveUp);
      stop();
    });
  });
  const decline = (cid: Id) => setDeclined(new Set([...declined(), cid]));
  const caller = () => {
    const cid = incoming();
    const v = cid ? state.dmVoice[cid]?.find((x) => x.userId !== me()) : undefined;
    return v ? state.users[sid(v.userId)] : undefined;
  };
  const title = () => {
    const ch = state.dms[incoming() ?? ""];
    if (ch && ch.recipientIds.length > 2) return ch.name || "Групповой звонок";
    return caller()?.displayName || caller()?.username || "Звонок";
  };
  return (
    <Show when={incoming()}>
      {(cid) => (
        <div class="call-incoming" role="alertdialog" aria-live="assertive" aria-label={`Входящий звонок: ${title()}`}>
          <div class="call-incoming-avatar">
            <UserAvatar user={caller()} size={56} noDeco />
          </div>
          <div class="call-incoming-text">
            <b>{title()}</b>
            <span>Входящий звонок…</span>
          </div>
          <button class="call-btn call-btn--decline" aria-label="Отклонить" title="Отклонить" onClick={() => decline(cid())}>
            <PhoneOff size={20} />
          </button>
          <button class="call-btn call-btn--accept" aria-label="Принять" title="Принять" onClick={() => props.onAccept(cid())}>
            <Phone size={20} />
          </button>
        </div>
      )}
    </Show>
  );
}
