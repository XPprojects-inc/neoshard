// Voice / video / screen sharing over LiveKit (self-hosted SFU, ADR-0012).
// This module (and livekit-client) is a lazy chunk: it loads only when a
// user joins a voice channel. Media never goes through the NeoShard server.

import { Button, toast } from "@neoshard/ui";
import { Monitor, MonitorOff, Mic, MicOff, PhoneOff, Video, VideoOff } from "../icons";
import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { Room, RoomEvent, Track, type Participant, type RemoteTrack } from "livekit-client";

import { UserAvatar } from "../profile/components";
import { api, core, state } from "../state";
import { prefs } from "../prefs";

interface VoiceGrant {
  url: string;
  token: string;
  turn?: { urls: string[]; username: string; credential: string } | null;
  /** LiveKit grant derived from channel permissions (SPEAK → microphone, VIDEO → camera, STREAM → screen). */
  grants?: { canPublishSources?: string[] };
}

interface Tile {
  id: string;
  name: string;
  speaking: boolean;
  muted: boolean;
  video?: MediaStreamTrack;
  screen?: MediaStreamTrack;
}

function VideoEl(props: { track: MediaStreamTrack }) {
  let el!: HTMLVideoElement;
  onMount(() => {
    el.srcObject = new MediaStream([props.track]);
  });
  return <video ref={el} autoplay playsinline muted />;
}

export default function Voice(props: { channelId: string; guildId: string; onLeave: () => void }) {
  const [tiles, setTiles] = createSignal<Tile[]>([]);
  const [mic, setMic] = createSignal(true);
  const [cam, setCam] = createSignal(false);
  const [screen, setScreen] = createSignal(false);
  const [error, setError] = createSignal("");
  let room: Room | undefined;

  const voiceState = () =>
    core().send({
      case: "voice",
      value: { guildId: props.guildId === "@me" ? 0n : BigInt(props.guildId), channelId: BigInt(props.channelId), selfMute: !mic(), selfDeaf: false, video: cam(), streaming: screen() },
    });

  const refresh = () => {
    if (!room) return;
    const all: Participant[] = [room.localParticipant, ...room.remoteParticipants.values()];
    setTiles(
      all.map((p) => {
        const cameraPub = p.getTrackPublication(Track.Source.Camera);
        const screenPub = p.getTrackPublication(Track.Source.ScreenShare);
        return {
          id: p.identity,
          name: p.name || state.users[p.identity]?.username || p.identity,
          speaking: p.isSpeaking,
          muted: !p.isMicrophoneEnabled,
          video: cameraPub?.track?.mediaStreamTrack,
          screen: screenPub?.track?.mediaStreamTrack,
        };
      }),
    );
  };

  onMount(async () => {
    const r = await api<VoiceGrant>("POST", `/channels/${props.channelId}/voice`, {});
    if (r.status !== 200) {
      setError(r.status === 503 ? "Голосовая связь не настроена на этом сервере" : r.status === 403 ? "Нет права подключаться к этому каналу" : "Не удалось подключиться");
      return;
    }
    const iceServers = r.body.turn ? [{ urls: r.body.turn.urls, username: r.body.turn.username, credential: r.body.turn.credential }] : undefined;
    room = new Room({
      adaptiveStream: true,
      dynacast: true,
      // Browser-level noise suppression + echo cancellation; RNNoise (neoshard-core `audio`) is applied in the desktop app.
      audioCaptureDefaults: {
        noiseSuppression: prefs.noiseSuppression,
        echoCancellation: true,
        autoGainControl: true,
        deviceId: prefs.micId || undefined,
      },
      videoCaptureDefaults: { deviceId: prefs.camId || undefined },
      audioOutput: { deviceId: prefs.speakerId || undefined },
    });
    room
      .on(RoomEvent.ParticipantConnected, refresh)
      .on(RoomEvent.ParticipantDisconnected, refresh)
      .on(RoomEvent.TrackSubscribed, (track: RemoteTrack) => {
        if (track.kind === Track.Kind.Audio) document.body.append(track.attach());
        refresh();
      })
      .on(RoomEvent.TrackUnsubscribed, (track: RemoteTrack) => {
        track.detach().forEach((el) => el.remove());
        refresh();
      })
      .on(RoomEvent.ActiveSpeakersChanged, refresh)
      .on(RoomEvent.LocalTrackPublished, refresh)
      .on(RoomEvent.LocalTrackUnpublished, refresh)
      .on(RoomEvent.Disconnected, () => toast("Голосовое соединение разорвано", "error"));
    try {
      await room.connect(r.body.url, r.body.token, iceServers ? { rtcConfig: { iceServers, iceTransportPolicy: "all" } } : undefined);
    } catch (e) {
      setError(`Не удалось подключиться: ${String(e)}`);
      return;
    }
    // Listening works without a microphone or without SPEAK: the mic is optional.
    if (r.body.grants?.canPublishSources?.includes("microphone") ?? true) {
      try {
        await room.localParticipant.setMicrophoneEnabled(true);
      } catch (e) {
        setMic(false);
        toast(`Микрофон недоступен, вы подключены только на прослушивание: ${String(e)}`, "error");
      }
    } else {
      setMic(false);
      toast("В этом канале у вас нет права говорить — вы подключены на прослушивание");
    }
    voiceState();
    refresh();
  });

  onCleanup(() => {
    void room?.disconnect();
    core().send({ case: "voice", value: { guildId: props.guildId === "@me" ? 0n : BigInt(props.guildId), channelId: 0n } });
  });

  const toggle = async (what: "mic" | "cam" | "screen") => {
    if (!room) return;
    const lp = room.localParticipant;
    try {
      if (what === "mic") {
        setMic(!mic());
        await lp.setMicrophoneEnabled(mic());
      } else if (what === "cam") {
        setCam(!cam());
        await lp.setCameraEnabled(cam());
      } else {
        setScreen(!screen());
        await lp.setScreenShareEnabled(screen(), { audio: true });
      }
    } catch (e) {
      toast(`Нет доступа к устройству: ${String(e)}`, "error");
    }
    voiceState();
    refresh();
  };

  return (
    <section class="voice" aria-label={props.guildId === "@me" ? "Звонок" : "Голосовой канал"}>
      <Show when={!error()} fallback={<div class="empty">{error()}<br /><Button variant="secondary" onClick={props.onLeave}>Назад</Button></div>}>
        <div class="tiles">
          <For each={tiles()}>
            {(t) => (
              <div class="tile" classList={{ speaking: t.speaking }}>
                <Show
                  when={t.screen ?? t.video}
                  fallback={
                    <Show when={state.users[t.id]} fallback={<span class="tile-letter">{t.name[0]?.toUpperCase()}</span>}>
                      {(u) => <UserAvatar user={u()} size={88} />}
                    </Show>
                  }
                >
                  {(tr) => <VideoEl track={tr()} />}
                </Show>
                <span class="label">
                  <Show when={t.muted}>
                    <MicOff size={13} aria-label="микрофон выключен" />
                  </Show>
                  {t.name}
                </span>
              </div>
            )}
          </For>
        </div>
        <div class="voice-controls" role="toolbar" aria-label="Управление звонком">
          <Button variant="secondary" aria-pressed={!mic()} onClick={() => toggle("mic")} aria-label={mic() ? "Выключить микрофон" : "Включить микрофон"}>
            {mic() ? <Mic size={18} /> : <MicOff size={18} />}
          </Button>
          <Button variant="secondary" aria-pressed={cam()} onClick={() => toggle("cam")} aria-label={cam() ? "Выключить камеру" : "Включить камеру"}>
            {cam() ? <Video size={18} /> : <VideoOff size={18} />}
          </Button>
          <Button variant="secondary" aria-pressed={screen()} onClick={() => toggle("screen")} aria-label={screen() ? "Остановить демонстрацию" : "Демонстрация экрана"}>
            {screen() ? <MonitorOff size={18} /> : <Monitor size={18} />}
          </Button>
          <Button variant="danger" onClick={props.onLeave} aria-label="Отключиться">
            <PhoneOff size={18} />
          </Button>
        </div>
      </Show>
    </section>
  );
}
