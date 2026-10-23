// Media end-to-end against a real LiveKit server.
//
// NeoShard issues the LiveKit tokens (POST /channels/{id}/voice); this script
// joins the room with two participants through the official LiveKit Node SDK,
// publishes real audio and video, checks that the other side receives them,
// and checks that LiveKit enforces the grants derived from NeoShard permissions.
//
//   livekit-server --dev                        # keys devkey / secret
//   LIVEKIT_URL=ws://127.0.0.1:7880 LIVEKIT_API_KEY=devkey LIVEKIT_API_SECRET=secret scripts/dev-server.sh
//   cd tests/media && npm install && npm test
import { randomBytes } from "node:crypto";
import {
  AudioFrame,
  AudioSource,
  AudioStream,
  LocalAudioTrack,
  LocalVideoTrack,
  Room,
  RoomEvent,
  TrackKind,
  TrackPublishOptions,
  TrackSource,
  VideoBufferType,
  VideoFrame,
  VideoSource,
  VideoStream,
  dispose,
} from "@livekit/rtc-node";

const BASE = (process.env.NEOSHARD_URL ?? "http://127.0.0.1:8080").replace(/\/$/, "");
const SPEAK = 1n << 17n;
const STREAM = 1n << 19n;
const CONNECT = 1n << 16n;

let failures = 0;
const check = (ok, what) => {
  console.log(`${ok ? "ok  " : "FAIL"} ${what}`);
  if (!ok) failures++;
};
const withTimeout = (p, ms, what) =>
  Promise.race([p, new Promise((_, rej) => setTimeout(() => rej(new Error(`timeout: ${what}`)), ms))]);

async function call(method, path, token, body) {
  const r = await fetch(`${BASE}/api/v1${path}`, {
    method,
    headers: { "content-type": "application/json", ...(token ? { authorization: `Bearer ${token}` } : {}) },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await r.text();
  return { status: r.status, body: text ? JSON.parse(text) : null };
}

async function register(name) {
  const username = `${name}_${randomBytes(3).toString("hex")}`;
  const r = await call("POST", "/auth/register", null, { username, password: randomBytes(12).toString("base64url") });
  if (r.status !== 200 && r.status !== 201) throw new Error(`register ${username}: ${r.status} ${JSON.stringify(r.body)}`);
  return { token: r.body.token, id: r.body.user.id, username };
}

/** 48 kHz mono sine, 10 ms frames. */
function sineFrames(source, hz, stopSignal) {
  const rate = 48000;
  const n = rate / 100;
  let phase = 0;
  return (async () => {
    while (!stopSignal.stop) {
      const data = new Int16Array(n);
      for (let i = 0; i < n; i++) {
        data[i] = Math.round(Math.sin(phase) * 12000);
        phase += (2 * Math.PI * hz) / rate;
      }
      await source.captureFrame(new AudioFrame(data, rate, 1, n));
    }
  })();
}

/** Solid-colour RGBA frames at ~15 fps. */
function videoFrames(source, w, h, rgb, stopSignal) {
  const data = new Uint8Array(w * h * 4);
  for (let i = 0; i < w * h; i++) data.set([...rgb, 255], i * 4);
  const timer = setInterval(() => {
    if (stopSignal.stop) return clearInterval(timer);
    source.captureFrame(new VideoFrame(data, w, h, VideoBufferType.RGBA));
  }, 66);
}

function waitTrack(room, source) {
  return new Promise((resolve) => {
    const on = (track, pub) => {
      if (pub.source === source) {
        room.off(RoomEvent.TrackSubscribed, on);
        resolve(track);
      }
    };
    room.on(RoomEvent.TrackSubscribed, on);
    for (const p of room.remoteParticipants.values())
      for (const pub of p.trackPublications.values()) if (pub.source === source && pub.track) resolve(pub.track);
  });
}

async function firstAudibleFrame(track) {
  const stream = new AudioStream(track, 48000, 1);
  const reader = stream.getReader();
  try {
    for (let i = 0; i < 500; i++) {
      const { value, done } = await reader.read();
      if (done) break;
      let sum = 0;
      for (const s of value.data) sum += s * s;
      const rms = Math.sqrt(sum / value.data.length);
      if (rms > 1000) return rms;
    }
    return 0;
  } finally {
    reader.releaseLock();
  }
}

async function firstVideoFrame(track) {
  const reader = new VideoStream(track).getReader();
  try {
    const { value } = await reader.read();
    return value.frame;
  } finally {
    reader.releaseLock();
  }
}

async function tryPublish(room, track, source) {
  try {
    await withTimeout(room.localParticipant.publishTrack(track, new TrackPublishOptions({ source })), 8000, "publish");
    return true;
  } catch {
    return false;
  }
}

async function main() {
  const alice = await register("lk_alice");
  const bob = await register("lk_bob");
  const g = await call("POST", "/guilds", alice.token, { name: "media e2e" });
  const voice = g.body.channels.find((c) => c.type === "CHANNEL_TYPE_VOICE").id;
  const text = g.body.channels.find((c) => c.type === "CHANNEL_TYPE_TEXT").id;
  const inv = await call("POST", `/channels/${text}/invites`, alice.token, {});
  await call("POST", `/invites/${inv.body.code}`, bob.token, {});

  // 1. Tokens from NeoShard.
  const va = await call("POST", `/channels/${voice}/voice`, alice.token, {});
  const vb = await call("POST", `/channels/${voice}/voice`, bob.token, {});
  check(va.status === 200 && vb.status === 200, `voice tokens issued (${va.status}, ${vb.status})`);
  check(va.body.room === `voice:${voice}`, `room name ${va.body.room}`);
  check(va.body.turn?.username && va.body.turn?.credential, "TURN credentials attached");

  // 2. Alice publishes microphone, camera and screen share; Bob receives all three.
  const ra = new Room();
  const rb = new Room();
  await ra.connect(va.body.url, va.body.token, { autoSubscribe: true });
  await rb.connect(vb.body.url, vb.body.token, { autoSubscribe: true });
  check(ra.isConnected && rb.isConnected, "both participants connected to LiveKit");
  const seen = [...rb.remoteParticipants.values()].map((p) => p.identity);
  check(seen.includes(String(alice.id)), `participant identity is the NeoShard user id (${seen.join(",")})`);

  const stop = { stop: false };
  const mic = new AudioSource(48000, 1);
  const cam = new VideoSource(320, 240);
  const scr = new VideoSource(640, 360);
  const micTrack = LocalAudioTrack.createAudioTrack("mic", mic);
  const camTrack = LocalVideoTrack.createVideoTrack("cam", cam);
  const scrTrack = LocalVideoTrack.createVideoTrack("screen", scr);
  const gotMic = waitTrack(rb, TrackSource.SOURCE_MICROPHONE);
  const gotCam = waitTrack(rb, TrackSource.SOURCE_CAMERA);
  const gotScr = waitTrack(rb, TrackSource.SOURCE_SCREENSHARE);
  check(await tryPublish(ra, micTrack, TrackSource.SOURCE_MICROPHONE), "alice publishes microphone");
  check(await tryPublish(ra, camTrack, TrackSource.SOURCE_CAMERA), "alice publishes camera");
  check(await tryPublish(ra, scrTrack, TrackSource.SOURCE_SCREENSHARE), "alice publishes screen share");
  const audio = sineFrames(mic, 440, stop);
  videoFrames(cam, 320, 240, [200, 30, 30], stop);
  videoFrames(scr, 640, 360, [30, 30, 200], stop);

  const tMic = await withTimeout(gotMic, 15000, "bob subscribes to mic");
  check(tMic.kind === TrackKind.KIND_AUDIO, "bob subscribed to alice's microphone");
  const rms = await withTimeout(firstAudibleFrame(tMic), 15000, "audio frames");
  check(rms > 1000, `bob hears audio (RMS ${rms.toFixed(0)})`);
  const tCam = await withTimeout(gotCam, 15000, "bob subscribes to camera");
  const fCam = await withTimeout(firstVideoFrame(tCam), 15000, "camera frame");
  check(fCam.width > 0 && fCam.height > 0, `bob receives camera frames (${fCam.width}x${fCam.height})`);
  const tScr = await withTimeout(gotScr, 15000, "bob subscribes to screen");
  const fScr = await withTimeout(firstVideoFrame(tScr), 15000, "screen frame");
  check(fScr.width > 0 && fScr.height > 0, `bob receives screen share frames (${fScr.width}x${fScr.height})`);
  stop.stop = true;
  await audio.catch(() => {});
  await rb.disconnect();

  // 3. Permissions become grants, and LiveKit enforces them: deny SPEAK + STREAM to @everyone.
  const ow = await call("PUT", `/channels/${voice}/overwrites/${g.body.id}`, alice.token, {
    type: 0,
    allow: "0",
    deny: String(SPEAK | STREAM),
  });
  check(ow.status === 204, `overwrite set (${ow.status})`);
  const vb2 = await call("POST", `/channels/${voice}/voice`, bob.token, {});
  check(
    JSON.stringify(vb2.body.grants.canPublishSources ?? vb2.body.grants.can_publish_sources) === JSON.stringify(["camera"]),
    `bob's grant now allows only the camera (${JSON.stringify(vb2.body.grants)})`,
  );
  const rb2 = new Room();
  await rb2.connect(vb2.body.url, vb2.body.token, { autoSubscribe: false });
  const bobMic = LocalAudioTrack.createAudioTrack("mic", new AudioSource(48000, 1));
  const bobScr = LocalVideoTrack.createVideoTrack("screen", new VideoSource(64, 64));
  const bobCam = LocalVideoTrack.createVideoTrack("cam", new VideoSource(64, 64));
  check(!(await tryPublish(rb2, bobMic, TrackSource.SOURCE_MICROPHONE)), "LiveKit rejects bob's microphone without SPEAK");
  check(!(await tryPublish(rb2, bobScr, TrackSource.SOURCE_SCREENSHARE)), "LiveKit rejects bob's screen share without STREAM");
  check(await tryPublish(rb2, bobCam, TrackSource.SOURCE_CAMERA), "bob may still publish the camera (VIDEO)");
  await rb2.disconnect();

  // 4. Without CONNECT there is no token at all.
  await call("PUT", `/channels/${voice}/overwrites/${g.body.id}`, alice.token, { type: 0, allow: "0", deny: String(CONNECT) });
  const vb3 = await call("POST", `/channels/${voice}/voice`, bob.token, {});
  check(vb3.status === 403, `no token without CONNECT (${vb3.status})`);

  await ra.disconnect();
}

try {
  await main();
} catch (e) {
  console.error(e);
  failures++;
} finally {
  await dispose();
}
console.log(failures ? `\n${failures} check(s) failed` : "\nall media checks passed");
process.exit(failures ? 1 : 0);
