// One profile popout / full profile for the whole app, opened with openProfile().

import { Popover, IconButton } from "@neoshard/ui";
import { createEffect, createResource, on, onCleanup, onMount, Show } from "solid-js";
import { Portal } from "solid-js/web";

import { activeCustom, openDm, statusName } from "../actions";
import { Check, Clock, Ellipsis, UserPlus, X } from "../icons";
import { RelationshipKind } from "@neoshard/core";
import { acceptFriend, addFriend } from "../views/Friends";
import { core, sid, state } from "../state";
import { ProfileCard } from "./components";
import { closeProfile, fetchProfile, invalidateProfile, openProfile, profileTarget, viewFromJson, viewFromUser } from "./data";

export function ProfileLayer() {
  const t = profileTarget;
  const [json] = createResource(
    () => t() && { id: t()!.userId, gid: t()!.guildId },
    (k) => fetchProfile(k.id, k.gid, true),
  );
  // Cosmetics change live (UserUpdate): drop the cached profile.
  createEffect(on(() => (t() ? state.users[t()!.userId] : undefined), () => t() && invalidateProfile(t()!.userId), { defer: true }));
  const view = () => {
    const id = t()?.userId;
    if (!id) return undefined;
    const u = state.users[id];
    return json() ? viewFromJson(json()!, u) : viewFromUser(u, id);
  };
  const isMe = () => t()?.userId === sid(state.me?.id ?? 0n);
  const presence = () => (isMe() ? state.myPresence : state.presence[t()?.userId ?? ""]);
  const status = () => (isMe() ? statusName(state.myPresence.status) : statusName(presence()?.status));
  const message = async (text: string) => {
    const id = t()?.userId;
    if (!id) return;
    closeProfile();
    const cid = await openDm(id);
    if (cid) core().send({ case: "sendMessage", value: { channelId: BigInt(cid), content: text, replyToId: 0n } });
  };
  const card = (full: boolean) => (
    <Show when={view()}>
      <ProfileCard
        view={view()!}
        full={full}
        status={status()}
        custom={activeCustom(presence())}
        onMessage={message}
        onOpenFull={() => openProfile({ ...t()!, full: true, anchor: undefined })}
        actions={
          full ? (
            <IconButton label="Закрыть" onClick={closeProfile}>
              <X size={18} />
            </IconButton>
          ) : (
            <>
              <Show when={!isMe()}>
                <FriendButton id={t()!.userId} />
              </Show>
              <IconButton label="Полный профиль" onClick={() => openProfile({ ...t()!, full: true, anchor: undefined })}>
                <Ellipsis size={18} />
              </IconButton>
            </>
          )
        }
      />
    </Show>
  );
  return (
    <>
      <Popover open={!!t() && !t()!.full} anchor={t()?.anchor} placement="right" onClose={closeProfile} label="Профиль пользователя" class="pcard-pop">
        {card(false)}
      </Popover>
      <Show when={t()?.full}>
        <FullProfile onClose={closeProfile}>{card(true)}</FullProfile>
      </Show>
    </>
  );
}

/** Add / accept / pending indicator for a profile card. */
function FriendButton(props: { id: string }) {
  const kind = () => state.relationships[props.id]?.kind ?? RelationshipKind.NONE;
  return (
    <Show when={kind() !== RelationshipKind.FRIEND && kind() !== RelationshipKind.BLOCKED}>
      <Show
        when={kind() === RelationshipKind.NONE}
        fallback={
          <Show
            when={kind() === RelationshipKind.INCOMING}
            fallback={
              <IconButton label="Заявка в друзья отправлена" disabled>
                <Clock size={18} />
              </IconButton>
            }
          >
            <IconButton label="Принять заявку в друзья" onClick={() => void acceptFriend(props.id)}>
              <Check size={18} />
            </IconButton>
          </Show>
        }
      >
        <IconButton label="Добавить в друзья" onClick={() => void addFriend(state.users[props.id]?.username ?? "")}>
          <UserPlus size={18} />
        </IconButton>
      </Show>
    </Show>
  );
}

function FullProfile(props: { onClose: () => void; children: any }) {
  let box!: HTMLDivElement;
  const prev = document.activeElement as HTMLElement | null;
  const onKey = (e: KeyboardEvent) => e.key === "Escape" && props.onClose();
  onMount(() => {
    document.addEventListener("keydown", onKey);
    requestAnimationFrame(() => box?.focus());
  });
  onCleanup(() => {
    document.removeEventListener("keydown", onKey);
    prev?.focus?.();
  });
  return (
    <Portal>
      <div class="ns-backdrop" onClick={(e) => e.target === e.currentTarget && props.onClose()}>
        <div ref={box} role="dialog" aria-modal="true" aria-label="Профиль пользователя" tabindex="-1" style={{ outline: "none", "max-height": "calc(100vh - 32px)", overflow: "auto", "border-radius": "var(--ns-radius-lg)" }}>
          {props.children}
        </div>
      </div>
    </Portal>
  );
}
