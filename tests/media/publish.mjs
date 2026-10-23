// Join a NeoShard voice channel as a bot-like participant and publish a tone
// and a test-pattern camera, to check a client (web/desktop) by hand.
//   NEOSHARD_TOKEN=... node publish.mjs <voice-channel-id> [seconds]
import { AudioFrame, AudioSource, LocalAudioTrack, LocalVideoTrack, Room, TrackPublishOptions, TrackSource, VideoBufferType, VideoFrame, VideoSource, dispose } from "@livekit/rtc-node";

const BASE = (process.env.NEOSHARD_URL ?? "http://127.0.0.1:8080").replace(/\/$/, "");
const [cid, secs = "60"] = process.argv.slice(2);
const r = await fetch(`${BASE}/api/v1/channels/${cid}/voice`, { method: "POST", headers: { authorization: `Bearer ${process.env.NEOSHARD_TOKEN}`, "content-type": "application/json" }, body: "{}" });
if (r.status !== 200) throw new Error(`voice token: ${r.status} ${await r.text()}`);
const v = await r.json();
const room = new Room();
await room.connect(v.url, v.token, { autoSubscribe: true });
const mic = new AudioSource(48000, 1);
const cam = new VideoSource(320, 240);
await room.localParticipant.publishTrack(LocalAudioTrack.createAudioTrack("tone", mic), new TrackPublishOptions({ source: TrackSource.SOURCE_MICROPHONE }));
await room.localParticipant.publishTrack(LocalVideoTrack.createVideoTrack("pattern", cam), new TrackPublishOptions({ source: TrackSource.SOURCE_CAMERA }));
console.log(`publishing in ${v.room} for ${secs}s`);

const until = Date.now() + Number(secs) * 1000;
const px = new Uint8Array(320 * 240 * 4);
let t = 0;
const video = setInterval(() => {
  for (let i = 0; i < 320 * 240; i++) px.set([(i + t) % 256, 120, 200, 255], i * 4);
  t += 8;
  cam.captureFrame(new VideoFrame(px, 320, 240, VideoBufferType.RGBA));
}, 66);
let phase = 0;
while (Date.now() < until) {
  const data = new Int16Array(480);
  for (let i = 0; i < 480; i++) {
    data[i] = Math.round(Math.sin(phase) * 8000);
    phase += (2 * Math.PI * 440) / 48000;
  }
  await mic.captureFrame(new AudioFrame(data, 48000, 1, 480));
}
clearInterval(video);
await room.disconnect();
await dispose();
