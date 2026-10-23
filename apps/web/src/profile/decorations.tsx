// Avatar frames, second edition: layered vector art with light, shade and
// gentle motion. Coordinates: 100×100 box; the avatar is the circle r≈37.3
// at the centre (the frame box is 134% of the avatar). All art is NeoShard's own.
// Motion uses CSS classes from profile.css and stops with reduced motion.

import { For, type JSX } from "solid-js";

let seq = 0;
const ids = (n: number) => {
  const base = `dc${++seq}`;
  return Array.from({ length: n }, (_, i) => `${base}-${i}`);
};
const url = (id: string) => `url(#${id})`;
const polar = (r: number, deg: number, cx = 50, cy = 50) => {
  const a = ((deg - 90) * Math.PI) / 180;
  return { x: cx + r * Math.cos(a), y: cy + r * Math.sin(a) };
};
const f = (n: number) => n.toFixed(2);

/** Arc path on a circle from angle a to b (degrees, 0 = top, clockwise). */
function arc(r: number, a: number, b: number) {
  const s = polar(r, a);
  const e = polar(r, b);
  const large = (b - a + 360) % 360 > 180 ? 1 : 0;
  return `M${f(s.x)} ${f(s.y)}A${r} ${r} 0 ${large} 1 ${f(e.x)} ${f(e.y)}`;
}

/** A 4-point sparkle. */
function sparkle(x: number, y: number, s: number) {
  const k = s * 0.22;
  return `M${f(x)} ${f(y - s)}C${f(x + k)} ${f(y - k)} ${f(x + k)} ${f(y - k)} ${f(x + s)} ${f(y)}C${f(x + k)} ${f(y + k)} ${f(x + k)} ${f(y + k)} ${f(x)} ${f(y + s)}C${f(x - k)} ${f(y + k)} ${f(x - k)} ${f(y + k)} ${f(x - s)} ${f(y)}C${f(x - k)} ${f(y - k)} ${f(x - k)} ${f(y - k)} ${f(x)} ${f(y - s)}Z`;
}

function Orbit() {
  const [band, glow, planet, planetHi, ring2] = ids(5);
  return (
    <svg viewBox="0 0 100 100" class="deco">
      <defs>
        <linearGradient id={band} x1="0" y1="0" x2="1" y2="0">
          <stop offset="0" stop-color="#7cf3ff" stop-opacity=".1" />
          <stop offset=".35" stop-color="#7cf3ff" />
          <stop offset=".7" stop-color="#b58cff" />
          <stop offset="1" stop-color="#b58cff" stop-opacity=".15" />
        </linearGradient>
        <linearGradient id={ring2} x1="1" y1="0" x2="0" y2="0">
          <stop offset="0" stop-color="#ffd6f6" stop-opacity="0" />
          <stop offset=".5" stop-color="#ffd6f6" stop-opacity=".9" />
          <stop offset="1" stop-color="#ffd6f6" stop-opacity="0" />
        </linearGradient>
        <filter id={glow} x="-30%" y="-30%" width="160%" height="160%">
          <feGaussianBlur stdDeviation="1.4" />
        </filter>
        <radialGradient id={planet} cx=".35" cy=".3" r=".8">
          <stop offset="0" stop-color="#fff2c7" />
          <stop offset=".45" stop-color="#ff9e6b" />
          <stop offset="1" stop-color="#9b3d7a" />
        </radialGradient>
        <radialGradient id={planetHi} cx=".3" cy=".25" r=".5">
          <stop offset="0" stop-color="#fff" stop-opacity=".9" />
          <stop offset="1" stop-color="#fff" stop-opacity="0" />
        </radialGradient>
      </defs>
      <g transform="rotate(-24 50 50)">
        <ellipse cx="50" cy="50" rx="49" ry="15" fill="none" stroke={url(band)} stroke-width="4" filter={url(glow)} opacity=".8" />
        <ellipse cx="50" cy="50" rx="49" ry="15" fill="none" stroke={url(band)} stroke-width="1.6" />
        <ellipse cx="50" cy="50" rx="44" ry="12.5" fill="none" stroke={url(ring2)} stroke-width=".7" />
      </g>
      <g class="deco-orbit-spin">
        <circle cx="50" cy="3.5" r="1.1" fill="#fff" />
      </g>
      <circle cx="88" cy="25" r="7.2" fill={url(planet)} />
      <ellipse cx="88" cy="25.5" rx="11" ry="2.6" fill="none" stroke="#ffe3b3" stroke-width=".9" transform="rotate(-18 88 25.5)" opacity=".85" />
      <circle cx="86" cy="22.5" r="3.4" fill={url(planetHi)} />
      <circle cx="13" cy="80" r="3.1" fill="#9fe8ff" />
      <circle cx="12.2" cy="79.1" r="1.2" fill="#fff" opacity=".8" />
      <path d={sparkle(20, 14, 3.4)} fill="#fff" class="deco-twinkle" />
      <path d={sparkle(80, 88, 2.6)} fill="#e8d4ff" class="deco-twinkle d2" />
      <path d={sparkle(95, 60, 1.8)} fill="#fff" class="deco-twinkle d3" />
    </svg>
  );
}

function Blossom(props: { x: number; y: number; s: number; rot?: number; light: string; dark: string; id: string }) {
  return (
    <g transform={`translate(${props.x} ${props.y}) rotate(${props.rot ?? 0}) scale(${props.s})`}>
      <For each={[0, 72, 144, 216, 288]}>
        {(a) => <path d="M0 0C-3.2 -2.4 -3.4 -7.4 0 -9.4C3.4 -7.4 3.2 -2.4 0 0Z" fill={url(props.id)} stroke={props.dark} stroke-width=".35" transform={`rotate(${a})`} />}
      </For>
      <circle r="1.7" fill="#ffd36b" />
      <For each={[0, 60, 120, 180, 240, 300]}>{(a) => <circle cx={f(Math.cos((a * Math.PI) / 180) * 2.8)} cy={f(Math.sin((a * Math.PI) / 180) * 2.8)} r=".45" fill="#e88a3c" />}</For>
      <path d="M0 -9.4C1 -7.6 1 -6 0 -4.5" stroke={props.light} stroke-width=".6" fill="none" opacity=".7" />
    </g>
  );
}

function Petals() {
  const [petal, branch] = ids(2);
  return (
    <svg viewBox="0 0 100 100" class="deco">
      <defs>
        <radialGradient id={petal} cx=".5" cy="1" r="1">
          <stop offset="0" stop-color="#ff7fb0" />
          <stop offset=".55" stop-color="#ffc2da" />
          <stop offset="1" stop-color="#fff2f7" />
        </radialGradient>
        <linearGradient id={branch} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stop-color="#6b3b2a" />
          <stop offset="1" stop-color="#3f2219" />
        </linearGradient>
      </defs>
      <path d="M6 66C10 84 30 98 52 98C70 98 84 90 92 78" fill="none" stroke={url(branch)} stroke-width="2.6" stroke-linecap="round" />
      <path d="M24 90C22 84 24 79 28 76M70 95C74 90 80 89 84 88" fill="none" stroke={url(branch)} stroke-width="1.5" stroke-linecap="round" />
      <path d="M40 97C38 101 33 101 31 98C34 98 37 97 40 97Z" fill="#7cbf5a" />
      <path d="M78 87C82 85 86 87 86 90C83 89 80 89 78 87Z" fill="#7cbf5a" />
      <Blossom x={9} y={68} s={1.15} rot={-20} light="#fff" dark="#e0679a" id={petal} />
      <Blossom x={27} y={88} s={0.95} rot={10} light="#fff" dark="#e0679a" id={petal} />
      <Blossom x={58} y={97} s={1.05} rot={30} light="#fff" dark="#e0679a" id={petal} />
      <Blossom x={88} y={80} s={0.85} rot={-10} light="#fff" dark="#e0679a" id={petal} />
      <circle cx="44" cy="99" r="2.4" fill="#ff9cc4" />
      <circle cx="75" cy="92" r="2" fill="#ffb3d1" />
      <g class="deco-fall">
        <path d="M84 20c2-2 5-1 5 1s-3 4-5 3c-1-1-1-3 0-4z" fill="#ffc2da" />
      </g>
      <g class="deco-fall d2">
        <path d="M14 30c2-2 5-1 5 1s-3 4-5 3c-1-1-1-3 0-4z" fill="#ffd8e7" />
      </g>
    </svg>
  );
}

function Snowflake(props: { x: number; y: number; s: number; cls?: string }) {
  return (
    <g transform={`translate(${props.x} ${props.y}) scale(${props.s})`}>
      <g class={props.cls}>
      <g stroke="#f4fbff" stroke-width=".9" stroke-linecap="round" fill="none">
        <For each={[0, 60, 120, 180, 240, 300]}>
          {(a) => (
            <g transform={`rotate(${a})`}>
              <path d="M0 0V-6M0 -2.6L-1.6 -4M0 -2.6L1.6 -4M0 -4.4L-1.1 -5.4M0 -4.4L1.1 -5.4" />
            </g>
          )}
        </For>
      </g>
      <circle r="1" fill="#fff" />
      </g>
    </g>
  );
}

function Frost() {
  const [ice, shardA, shardB, sheen] = ids(4);
  const shards = [
    [-50, 9, 1],
    [-28, 6, 0],
    [205, 8, 1],
    [228, 11, 0],
    [252, 7, 1],
    [140, 6, 0],
    [158, 9, 1],
  ];
  return (
    <svg viewBox="0 0 100 100" class="deco">
      <defs>
        <linearGradient id={ice} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stop-color="#e9f8ff" stop-opacity=".95" />
          <stop offset=".5" stop-color="#8fd3ff" stop-opacity=".75" />
          <stop offset="1" stop-color="#4f8fd6" stop-opacity=".9" />
        </linearGradient>
        <linearGradient id={shardA} x1="0" y1="0" x2="1" y2="0">
          <stop offset="0" stop-color="#ffffff" />
          <stop offset="1" stop-color="#9ad8ff" />
        </linearGradient>
        <linearGradient id={shardB} x1="0" y1="0" x2="1" y2="0">
          <stop offset="0" stop-color="#5fa8e8" />
          <stop offset="1" stop-color="#2e6db8" />
        </linearGradient>
        <linearGradient id={sheen} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stop-color="#fff" stop-opacity=".9" />
          <stop offset="1" stop-color="#fff" stop-opacity="0" />
        </linearGradient>
      </defs>
      <circle cx="50" cy="50" r="40.5" fill="none" stroke={url(ice)} stroke-width="4" />
      <path d={arc(40.5, 290, 30)} fill="none" stroke={url(sheen)} stroke-width="1.4" stroke-linecap="round" />
      <circle cx="50" cy="50" r="42.8" fill="none" stroke="#dff4ff" stroke-width=".5" stroke-dasharray="1 2.5" opacity=".8" />
      <For each={shards}>
        {([deg, len, flip]) => {
          const base = polar(41, deg);
          return (
            <g transform={`translate(${f(base.x)} ${f(base.y)}) rotate(${deg + (flip ? 8 : -8)})`}>
              <path d={`M-2.6 0L0 ${-len}L0 1Z`} fill={url(shardA)} />
              <path d={`M0 ${-len}L2.6 0L0 1Z`} fill={url(shardB)} />
            </g>
          );
        }}
      </For>
      <Snowflake x={86} y={20} s={1.15} cls="deco-spin-flake" />
      <Snowflake x={14} y={78} s={0.8} cls="deco-spin-flake d2" />
      <Snowflake x={68} y={95} s={0.6} />
      <path d={sparkle(30, 7, 2.6)} fill="#fff" class="deco-twinkle" />
      <path d={sparkle(96, 52, 2)} fill="#fff" class="deco-twinkle d2" />
    </svg>
  );
}

/** A ring of flame tongues: irregular, leaning, taller at the bottom. */
function flameRing(base: number, scale: number, seed: number): string {
  let x = seed;
  const rnd = () => {
    x = (x * 16807) % 2147483647;
    return x / 2147483647;
  };
  const n = 17;
  const span = 360 / n;
  const start = 8;
  const height = (deg: number) => {
    const fromBottom = Math.abs(((deg % 360) + 360) % 360 - 180) / 180; // 0 at bottom, 1 at top
    return (3 + 11 * (1 - fromBottom) ** 1.4) * (0.75 + rnd() * 0.5) * scale;
  };
  let d = "";
  for (let k = 0; k < n; k++) {
    const v = start + k * span;
    const v2 = v + span;
    const h = height(v + span / 2);
    const lean = 4 + rnd() * 4;
    const tip = v + span / 2 + lean;
    const p0 = polar(base, v);
    if (k === 0) d += `M${f(p0.x)} ${f(p0.y)}`;
    const c1 = polar(base + h * 0.35, v + span * 0.15);
    const c2 = polar(base + h * 0.85, tip - lean * 0.9);
    const t = polar(base + h, tip);
    const c3 = polar(base + h * 0.55, tip + span * 0.05);
    const c4 = polar(base + h * 0.12, v2 - span * 0.25);
    const e = polar(base, v2);
    d += `C${f(c1.x)} ${f(c1.y)} ${f(c2.x)} ${f(c2.y)} ${f(t.x)} ${f(t.y)}C${f(c3.x)} ${f(c3.y)} ${f(c4.x)} ${f(c4.y)} ${f(e.x)} ${f(e.y)}`;
  }
  // Close along the inside of the ring (counter-clockwise) so the avatar stays clear.
  const inner = base - 2.6;
  const a = polar(inner, start);
  const b = polar(inner, start + 180);
  return `${d}L${f(a.x)} ${f(a.y)}A${inner} ${inner} 0 1 0 ${f(b.x)} ${f(b.y)}A${inner} ${inner} 0 1 0 ${f(a.x)} ${f(a.y)}Z`;
}

function Flame() {
  const [outer, mid, inner, ember, glow] = ids(5);
  return (
    <svg viewBox="0 0 100 100" class="deco">
      <defs>
        <radialGradient id={outer} cx="50" cy="50" r="52" gradientUnits="userSpaceOnUse">
          <stop offset=".74" stop-color="#ff3d14" />
          <stop offset=".88" stop-color="#e3200c" />
          <stop offset="1" stop-color="#8f1408" stop-opacity=".6" />
        </radialGradient>
        <radialGradient id={mid} cx="50" cy="50" r="50" gradientUnits="userSpaceOnUse">
          <stop offset=".76" stop-color="#ffb02e" />
          <stop offset="1" stop-color="#ff6a1a" />
        </radialGradient>
        <radialGradient id={inner} cx="50" cy="50" r="48" gradientUnits="userSpaceOnUse">
          <stop offset=".78" stop-color="#fffbe0" />
          <stop offset="1" stop-color="#ffd84a" />
        </radialGradient>
        <radialGradient id={ember}>
          <stop offset="0" stop-color="#fff2a8" />
          <stop offset="1" stop-color="#ff7a1a" stop-opacity="0" />
        </radialGradient>
        <filter id={glow} x="-20%" y="-20%" width="140%" height="140%">
          <feGaussianBlur stdDeviation="1.8" />
        </filter>
      </defs>
      <path d={flameRing(38.5, 1.1, 7)} fill="#ff4a14" filter={url(glow)} opacity=".55" />
      <g class="deco-burn">
        <path d={flameRing(38.5, 1, 7)} fill={url(outer)} />
      </g>
      <g class="deco-burn b2">
        <path d={flameRing(38.5, 0.68, 31)} fill={url(mid)} />
      </g>
      <g class="deco-burn b3">
        <path d={flameRing(38.5, 0.36, 91)} fill={url(inner)} />
      </g>
      <circle cx="50" cy="50" r="38.2" fill="none" stroke="#ffe7a0" stroke-width=".8" opacity=".8" />
      <circle cx="16" cy="18" r="2.2" fill={url(ember)} class="deco-rise" />
      <circle cx="84" cy="14" r="1.6" fill={url(ember)} class="deco-rise d2" />
      <circle cx="93" cy="40" r="1.3" fill={url(ember)} class="deco-rise d3" />
      <circle cx="8" cy="44" r="1.1" fill={url(ember)} class="deco-rise d2" />
    </svg>
  );
}

function Circuit() {
  const [metal, pulse] = ids(2);
  const traces = [
    { a: 20, path: "l6 -4h7" },
    { a: 70, path: "l7 2v6" },
    { a: 145, path: "l5 5h5" },
    { a: 200, path: "l-6 4h-6" },
    { a: 250, path: "l-7 -1v-6" },
    { a: 320, path: "l-5 -5h-6" },
  ];
  return (
    <svg viewBox="0 0 100 100" class="deco">
      <defs>
        <linearGradient id={metal} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stop-color="#3a4a5c" />
          <stop offset=".5" stop-color="#1b2633" />
          <stop offset="1" stop-color="#46596e" />
        </linearGradient>
        <linearGradient id={pulse} x1="0" y1="0" x2="1" y2="0">
          <stop offset="0" stop-color="#3dffb0" stop-opacity="0" />
          <stop offset="1" stop-color="#3dffb0" />
        </linearGradient>
      </defs>
      <circle cx="50" cy="50" r="40.5" fill="none" stroke={url(metal)} stroke-width="5" />
      <circle cx="50" cy="50" r="42.6" fill="none" stroke="#2bd98e" stroke-width=".7" opacity=".7" />
      <circle cx="50" cy="50" r="38.4" fill="none" stroke="#2bd98e" stroke-width=".5" opacity=".5" />
      <For each={[0, 30, 60, 90, 120, 150, 180, 210, 240, 270, 300, 330]}>
        {(d) => {
          const p = polar(40.5, d);
          return <rect x={f(p.x - 1.1)} y={f(p.y - 0.6)} width="2.2" height="1.2" rx=".3" fill="#7dffd0" transform={`rotate(${d} ${f(p.x)} ${f(p.y)})`} opacity=".85" />;
        }}
      </For>
      <circle cx="50" cy="50" r="40.5" fill="none" stroke="#7dffd0" stroke-width="1.2" stroke-dasharray="10 244" stroke-linecap="round" class="deco-pulse-run" />
      <For each={traces}>
        {(t) => {
          const p = polar(43, t.a);
          return (
            <g stroke="#2bd98e" stroke-width=".9" fill="none" stroke-linecap="round" stroke-linejoin="round">
              <path d={`M${f(p.x)} ${f(p.y)}${t.path}`} />
              <circle cx={f(p.x)} cy={f(p.y)} r=".9" fill="#7dffd0" stroke="none" />
            </g>
          );
        }}
      </For>
      <For each={traces}>
        {(t) => {
          const p = polar(43, t.a);
          const m = /l(-?\d+) (-?\d+)[hv](-?\d+)/.exec(t.path)!;
          const dx = Number(m[1]) + (t.path.includes("h") ? Number(m[3]) : 0);
          const dy = Number(m[2]) + (t.path.includes("v") ? Number(m[3]) : 0);
          return <rect x={f(p.x + dx - 1.6)} y={f(p.y + dy - 1.6)} width="3.2" height="3.2" rx=".6" fill="#0e1a14" stroke="#3dffb0" stroke-width=".7" />;
        }}
      </For>
    </svg>
  );
}

function Stardust() {
  const [galaxy, glow] = ids(2);
  const stars = [
    [12, 3.2],
    [48, 2.2],
    [84, 2.8],
    [130, 1.8],
    [168, 3],
    [210, 2],
    [250, 2.6],
    [300, 1.9],
    [335, 2.4],
  ];
  return (
    <svg viewBox="0 0 100 100" class="deco">
      <defs>
        <linearGradient id={galaxy} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stop-color="#ff9ae6" />
          <stop offset=".45" stop-color="#7b6cff" />
          <stop offset="1" stop-color="#3fd0ff" />
        </linearGradient>
        <filter id={glow} x="-30%" y="-30%" width="160%" height="160%">
          <feGaussianBlur stdDeviation="2" />
        </filter>
      </defs>
      <circle cx="50" cy="50" r="40.8" fill="none" stroke={url(galaxy)} stroke-width="5" filter={url(glow)} opacity=".75" />
      <circle cx="50" cy="50" r="40.8" fill="none" stroke={url(galaxy)} stroke-width="1.6" />
      <circle cx="50" cy="50" r="43.6" fill="none" stroke="#fff" stroke-width=".4" stroke-dasharray=".5 3" opacity=".7" />
      <For each={stars}>
        {([deg, s], i) => {
          const p = polar(42.5 + (i() % 3) * 2.2, deg);
          return <path d={sparkle(p.x, p.y, s)} fill={i() % 2 ? "#fff6c9" : "#ffffff"} class={`deco-twinkle d${(i() % 3) + 1}`} />;
        }}
      </For>
      <g class="deco-comet">
        <path d="M50 4.5C44 5.5 38 7.5 33 10.5" stroke="#fff" stroke-width="1.4" stroke-linecap="round" opacity=".55" fill="none" />
        <circle cx="50" cy="4.5" r="1.8" fill="#fff" />
      </g>
    </svg>
  );
}

function Leaves() {
  const [leaf, leafDark, ribbon] = ids(3);
  const side = (dir: 1 | -1) =>
    [196, 208, 220, 232, 244, 256, 268, 280, 292, 304].map((deg, i) => {
      const d = dir === 1 ? deg : 360 - deg;
      const p = polar(42, d);
      return { ...p, d, s: 1.35 - i * 0.06, i };
    });
  const Leaf = (props: { x: number; y: number; d: number; s: number; out: boolean; dir: number }) => (
    <g transform={`translate(${f(props.x)} ${f(props.y)}) rotate(${props.d + (props.out ? 70 : -70) * props.dir}) scale(${props.s})`}>
      <path d="M0 0C2.5 -3 2.6 -8 0 -11C-2.6 -8 -2.5 -3 0 0Z" fill={url(props.out ? leaf : leafDark)} />
      <path d="M0 -.5V-10" stroke="#e6ffd1" stroke-width=".35" opacity=".6" />
    </g>
  );
  return (
    <svg viewBox="0 0 100 100" class="deco">
      <defs>
        <linearGradient id={leaf} x1="0" y1="1" x2="0" y2="0">
          <stop offset="0" stop-color="#3f8f3a" />
          <stop offset="1" stop-color="#9be36a" />
        </linearGradient>
        <linearGradient id={leafDark} x1="0" y1="1" x2="0" y2="0">
          <stop offset="0" stop-color="#2c6a2c" />
          <stop offset="1" stop-color="#6cc04f" />
        </linearGradient>
        <linearGradient id={ribbon} x1="0" y1="0" x2="1" y2="0">
          <stop offset="0" stop-color="#b8322f" />
          <stop offset=".5" stop-color="#ff5d57" />
          <stop offset="1" stop-color="#b8322f" />
        </linearGradient>
      </defs>
      <path d={arc(42, 192, 312)} fill="none" stroke="#5a3a20" stroke-width="1.2" opacity=".8" />
      <path d={arc(42, 48, 168)} fill="none" stroke="#5a3a20" stroke-width="1.2" opacity=".8" />
      <For each={side(1)}>{(p) => <><Leaf x={p.x} y={p.y} d={p.d} s={p.s} out dir={1} /><Leaf x={p.x} y={p.y} d={p.d} s={p.s * 0.9} out={false} dir={1} /></>}</For>
      <For each={side(-1)}>{(p) => <><Leaf x={p.x} y={p.y} d={p.d} s={p.s} out dir={-1} /><Leaf x={p.x} y={p.y} d={p.d} s={p.s * 0.9} out={false} dir={-1} /></>}</For>
      <circle cx="30" cy="16" r="1.8" fill="#d64545" />
      <circle cx="71" cy="16" r="1.8" fill="#d64545" />
      <circle cx="29.4" cy="15.4" r=".6" fill="#ffb3b3" />
      <path d="M41 92L36 99L42 97L44 101L47 94Z" fill={url(ribbon)} />
      <path d="M59 92L64 99L58 97L56 101L53 94Z" fill={url(ribbon)} />
      <ellipse cx="50" cy="92" rx="7" ry="3.4" fill={url(ribbon)} />
      <ellipse cx="50" cy="91.2" rx="4" ry="1.1" fill="#ffb0a8" opacity=".6" />
    </svg>
  );
}

function Crown() {
  const [gold, goldDark, gem, gemB] = ids(4);
  return (
    <svg viewBox="0 0 100 100" class="deco">
      <defs>
        <linearGradient id={gold} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stop-color="#fff6c4" />
          <stop offset=".35" stop-color="#f5c542" />
          <stop offset=".7" stop-color="#c98a14" />
          <stop offset="1" stop-color="#ffe08a" />
        </linearGradient>
        <linearGradient id={goldDark} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stop-color="#ffe7a3" />
          <stop offset=".5" stop-color="#b8780e" />
          <stop offset="1" stop-color="#f2c14e" />
        </linearGradient>
        <radialGradient id={gem} cx=".35" cy=".3" r=".8">
          <stop offset="0" stop-color="#ffd1d1" />
          <stop offset=".4" stop-color="#ff3d5a" />
          <stop offset="1" stop-color="#8a0f2a" />
        </radialGradient>
        <radialGradient id={gemB} cx=".35" cy=".3" r=".8">
          <stop offset="0" stop-color="#d6f1ff" />
          <stop offset=".45" stop-color="#3d8bff" />
          <stop offset="1" stop-color="#14306e" />
        </radialGradient>
      </defs>
      <circle cx="50" cy="50" r="40.3" fill="none" stroke={url(goldDark)} stroke-width="3.6" />
      <circle cx="50" cy="50" r="40.3" fill="none" stroke="#fff3c4" stroke-width=".6" stroke-dasharray="2 4" opacity=".7" />
      <For each={[60, 120, 240, 300]}>
        {(d) => {
          const p = polar(40.3, d);
          return <circle cx={f(p.x)} cy={f(p.y)} r="2.1" fill={url(d < 180 ? gemB : gem)} stroke="#c98a14" stroke-width=".6" />;
        }}
      </For>
      <g transform="translate(50 9) rotate(-8)">
        <path d="M-17 6L-19 -9L-9 -1L0 -14L9 -1L19 -9L17 6Z" fill={url(gold)} stroke="#9a6408" stroke-width=".8" stroke-linejoin="round" />
        <path d="M-17 6H17V10H-17Z" fill={url(goldDark)} stroke="#9a6408" stroke-width=".8" />
        <path d="M-15 4L-16 -5L-9 1" fill="none" stroke="#fff6c4" stroke-width=".7" opacity=".8" />
        <circle cx="0" cy="-14.5" r="2.2" fill={url(gem)} />
        <circle cx="-19" cy="-9.5" r="1.7" fill={url(gemB)} />
        <circle cx="19" cy="-9.5" r="1.7" fill={url(gemB)} />
        <circle cx="0" cy="3" r="2.6" fill={url(gem)} stroke="#9a6408" stroke-width=".5" />
        <circle cx="-9" cy="3.6" r="1.6" fill={url(gemB)} />
        <circle cx="9" cy="3.6" r="1.6" fill={url(gemB)} />
        <circle cx="-.8" cy="2.2" r=".8" fill="#fff" opacity=".8" />
      </g>
      <path d={sparkle(73, 6, 2.6)} fill="#fff8d6" class="deco-twinkle" />
      <path d={sparkle(26, 3, 1.8)} fill="#fff8d6" class="deco-twinkle d2" />
    </svg>
  );
}

function Neon() {
  const [tube, glow, glowWide] = ids(3);
  return (
    <svg viewBox="0 0 100 100" class="deco deco-neon">
      <defs>
        <linearGradient id={tube} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stop-color="#ff4fd8" />
          <stop offset=".5" stop-color="#b56bff" />
          <stop offset="1" stop-color="#4fe3ff" />
        </linearGradient>
        <filter id={glow} x="-30%" y="-30%" width="160%" height="160%">
          <feGaussianBlur stdDeviation="1.3" />
        </filter>
        <filter id={glowWide} x="-40%" y="-40%" width="180%" height="180%">
          <feGaussianBlur stdDeviation="3.2" />
        </filter>
      </defs>
      <circle cx="50" cy="50" r="41" fill="none" stroke={url(tube)} stroke-width="5" filter={url(glowWide)} opacity=".7" />
      <circle cx="50" cy="50" r="41" fill="none" stroke={url(tube)} stroke-width="2.6" filter={url(glow)} />
      <circle cx="50" cy="50" r="41" fill="none" stroke="#fff" stroke-width=".9" opacity=".9" />
      <path d={arc(46, 300, 340)} fill="none" stroke="#4fe3ff" stroke-width="1.6" stroke-linecap="round" filter={url(glow)} class="deco-buzz" />
      <path d={arc(46, 300, 340)} fill="none" stroke="#e8fbff" stroke-width=".6" stroke-linecap="round" class="deco-buzz" />
      <g transform="translate(84 80)">
        <path d="M0 3.5C-4.5 0 -6 -2.5 -4.2 -4.6C-2.8 -6.2 -.8 -5.4 0 -3.8C.8 -5.4 2.8 -6.2 4.2 -4.6C6 -2.5 4.5 0 0 3.5Z" fill="none" stroke="#ff4fd8" stroke-width="1.6" filter={url(glow)} />
        <path d="M0 3.5C-4.5 0 -6 -2.5 -4.2 -4.6C-2.8 -6.2 -.8 -5.4 0 -3.8C.8 -5.4 2.8 -6.2 4.2 -4.6C6 -2.5 4.5 0 0 3.5Z" fill="none" stroke="#ffe3f7" stroke-width=".6" />
      </g>
    </svg>
  );
}

function Shards() {
  const [hi, mid, lo] = ids(3);
  const gems = [
    { d: -40, s: 1.25 },
    { d: 25, s: 0.9 },
    { d: 100, s: 1.05 },
    { d: 160, s: 0.75 },
    { d: 215, s: 1.15 },
    { d: 275, s: 0.85 },
  ];
  return (
    <svg viewBox="0 0 100 100" class="deco">
      <defs>
        <linearGradient id={hi} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stop-color="#e8fff9" />
          <stop offset="1" stop-color="#7debd2" />
        </linearGradient>
        <linearGradient id={mid} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stop-color="#3fd3b0" />
          <stop offset="1" stop-color="#1f9c84" />
        </linearGradient>
        <linearGradient id={lo} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stop-color="#17705f" />
          <stop offset="1" stop-color="#0b3d35" />
        </linearGradient>
      </defs>
      <circle cx="50" cy="50" r="41" fill="none" stroke="#2ec4a6" stroke-width=".8" stroke-dasharray="3 3" opacity=".6" />
      <g class="deco-spin-slow">
        <For each={gems}>
          {(g, i) => {
            const p = polar(45, g.d);
            const s = 8.6 * g.s;
            return (
              <g transform={`translate(${f(p.x)} ${f(p.y)}) rotate(${g.d + 20})`}>
                <path d={`M0 ${-s}L${s * 0.55} ${-s * 0.15}L0 ${s}Z`} fill={url(mid)} />
                <path d={`M0 ${-s}L${-s * 0.55} ${-s * 0.15}L0 ${s}Z`} fill={url(lo)} />
                <path d={`M0 ${-s}L${s * 0.55} ${-s * 0.15}L${s * 0.1} ${-s * 0.05}Z`} fill={url(hi)} />
                <path d={`M0 ${-s}L${-s * 0.55} ${-s * 0.15}L${-s * 0.1} ${-s * 0.05}Z`} fill={url(hi)} opacity=".55" />
                <path d={`M0 ${-s}L0 ${s}`} stroke="#d9fff5" stroke-width=".3" opacity=".7" />
                <path d={sparkle(s * 0.25, -s * 0.55, 1.4)} fill="#fff" class={`deco-twinkle d${(i() % 3) + 1}`} />
              </g>
            );
          }}
        </For>
      </g>
    </svg>
  );
}

function CatEars() {
  const [fur, inner, shade] = ids(3);
  const ear = (flip: 1 | -1) => (
    <g transform={`translate(${50 + flip * 25} 17) rotate(${flip * 22}) scale(${flip} 1)`}>
      <path d="M-11 8C-12 -3 -8 -12 -1 -17C2 -11 9 -4 11 7C4 3 -4 3 -11 8Z" fill={url(fur)} stroke="#4d4f5c" stroke-width=".9" stroke-linejoin="round" />
      <path d="M-6.5 5.5C-7 -2 -4.5 -8 -1 -11.5C1 -7 5 -2 6.5 4.5C2 2.5 -2.5 2.5 -6.5 5.5Z" fill={url(inner)} />
      <path d="M-8 6C-4 3.6 3 3.6 8.5 5.6" fill="none" stroke={url(shade)} stroke-width="2.4" opacity=".55" />
      <path d="M-3 -9L-4.4 -12M0 -10L-.4 -13.6M2.5 -8L3.6 -11.4" stroke="#fff" stroke-width=".6" stroke-linecap="round" opacity=".9" />
    </g>
  );
  return (
    <svg viewBox="0 0 100 100" class="deco">
      <defs>
        <linearGradient id={fur} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stop-color="#f3f4f8" />
          <stop offset="1" stop-color="#a9adbf" />
        </linearGradient>
        <linearGradient id={inner} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stop-color="#ffd0e2" />
          <stop offset="1" stop-color="#ff8db6" />
        </linearGradient>
        <linearGradient id={shade} x1="0" y1="0" x2="1" y2="0">
          <stop offset="0" stop-color="#4d4f5c" stop-opacity="0" />
          <stop offset=".5" stop-color="#4d4f5c" />
          <stop offset="1" stop-color="#4d4f5c" stop-opacity="0" />
        </linearGradient>
      </defs>
      <g class="deco-ear-l">{ear(-1)}</g>
      <g class="deco-ear-r">{ear(1)}</g>
    </svg>
  );
}

function Headphones() {
  const [band, cup, led] = ids(3);
  const cupAt = (x: number, flip: 1 | -1) => (
    <g transform={`translate(${x} 55) scale(${flip} 1)`}>
      <rect x="-7" y="-13" width="12" height="26" rx="5.5" fill={url(cup)} stroke="#11151c" stroke-width=".8" />
      <rect x="2.4" y="-10" width="4.6" height="20" rx="2.3" fill="#2a3140" />
      <rect x="-5.2" y="-10.5" width="2.2" height="12" rx="1.1" fill="#fff" opacity=".22" />
      <circle cx="-1" cy="8" r="1.3" fill={url(led)} class="deco-buzz" />
    </g>
  );
  return (
    <svg viewBox="0 0 100 100" class="deco">
      <defs>
        <linearGradient id={band} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stop-color="#5a6478" />
          <stop offset=".45" stop-color="#2b3240" />
          <stop offset="1" stop-color="#161b24" />
        </linearGradient>
        <linearGradient id={cup} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stop-color="#4a5368" />
          <stop offset="1" stop-color="#1a1f29" />
        </linearGradient>
        <radialGradient id={led}>
          <stop offset="0" stop-color="#d6fff4" />
          <stop offset="1" stop-color="#2ec4a6" />
        </radialGradient>
      </defs>
      <path d="M11 54C11 26 28 8 50 8S89 26 89 54" fill="none" stroke={url(band)} stroke-width="6.5" stroke-linecap="round" />
      <path d="M16 40C21 22 34 13 50 13" fill="none" stroke="#fff" stroke-width="1" stroke-linecap="round" opacity=".25" />
      <path d="M30 13.5C36 10.5 43 9.4 50 9.4S64 10.5 70 13.5" fill="none" stroke="#2ec4a6" stroke-width="1.1" stroke-linecap="round" opacity=".8" />
      {cupAt(10, 1)}
      {cupAt(90, -1)}
    </svg>
  );
}

function Halo() {
  const [gold, glow] = ids(2);
  return (
    <svg viewBox="0 0 100 100" class="deco">
      <defs>
        <linearGradient id={gold} x1="0" y1="0" x2="1" y2="0">
          <stop offset="0" stop-color="#ffd86b" />
          <stop offset=".5" stop-color="#fff8d6" />
          <stop offset="1" stop-color="#ffc53d" />
        </linearGradient>
        <filter id={glow} x="-40%" y="-80%" width="180%" height="260%">
          <feGaussianBlur stdDeviation="2.4" />
        </filter>
      </defs>
      <g class="deco-hover">
        <ellipse cx="50" cy="5" rx="26" ry="6" fill="none" stroke="#ffe08a" stroke-width="5" filter={url(glow)} opacity=".8" />
        <ellipse cx="50" cy="5" rx="26" ry="6" fill="none" stroke={url(gold)} stroke-width="2.6" />
        <ellipse cx="50" cy="4.2" rx="22" ry="4.2" fill="none" stroke="#fff" stroke-width=".6" opacity=".7" />
      </g>
      <circle cx="50" cy="50" r="41" fill="none" stroke="#fff3c4" stroke-width=".6" opacity=".45" />
      <path d={sparkle(80, 10, 2.4)} fill="#fff8d6" class="deco-twinkle" />
      <path d={sparkle(20, 12, 1.8)} fill="#fff8d6" class="deco-twinkle d2" />
    </svg>
  );
}

function Hearts() {
  const [pink, shine] = ids(2);
  const heart = (x: number, y: number, s: number, rot: number, cls: string) => (
    <g transform={`translate(${x} ${y}) rotate(${rot}) scale(${s})`}>
      <g class={cls}>
        <path d="M0 5C-6 1 -8 -2.5 -6 -5.2C-4.2 -7.4 -1.4 -6.6 0 -4.4C1.4 -6.6 4.2 -7.4 6 -5.2C8 -2.5 6 1 0 5Z" fill={url(pink)} stroke="#c2185b" stroke-width=".5" />
        <path d="M-4.6 -4.4C-3.6 -5.6 -2 -5.4 -1.4 -4.2" stroke={url(shine)} stroke-width="1.2" stroke-linecap="round" fill="none" />
      </g>
    </g>
  );
  return (
    <svg viewBox="0 0 100 100" class="deco">
      <defs>
        <linearGradient id={pink} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stop-color="#ff9ccb" />
          <stop offset="1" stop-color="#e8336f" />
        </linearGradient>
        <linearGradient id={shine} x1="0" y1="0" x2="1" y2="0">
          <stop offset="0" stop-color="#fff" />
          <stop offset="1" stop-color="#fff" stop-opacity=".2" />
        </linearGradient>
      </defs>
      <circle cx="50" cy="50" r="41" fill="none" stroke="#ff7fb0" stroke-width="1.2" stroke-dasharray="1 3" stroke-linecap="round" />
      {heart(84, 18, 1.35, 18, "deco-beat")}
      {heart(14, 76, 1.05, -16, "deco-beat d2")}
      {heart(90, 70, 0.75, 22, "deco-beat d3")}
      {heart(22, 14, 0.65, -10, "deco-beat d2")}
      {heart(58, 96, 0.8, 4, "deco-beat")}
    </svg>
  );
}

const ART: Record<string, () => JSX.Element> = {
  orbit: Orbit,
  petals: Petals,
  frost: Frost,
  flame: Flame,
  circuit: Circuit,
  stardust: Stardust,
  leaves: Leaves,
  crown: Crown,
  neon: Neon,
  shards: Shards,
  cat_ears: CatEars,
  headphones: Headphones,
  halo: Halo,
  hearts: Hearts,
};

export function DecorationArt(props: { id: string }): JSX.Element {
  const Art = ART[props.id];
  return Art ? <Art /> : null;
}
