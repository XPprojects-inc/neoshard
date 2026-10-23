// Initial JS budget: scripts referenced by index.html (not lazy chunks), gzip < 150 KiB.
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { gzipSync } from "node:zlib";
const dist = process.argv[2] ?? "apps/web/dist";
const html = readFileSync(join(dist, "index.html"), "utf8");
const initial = [...html.matchAll(/src="\/(assets\/[^"]+\.js)"/g)].map((m) => m[1]);
let total = 0;
for (const f of initial) total += gzipSync(readFileSync(join(dist, f)), { level: 9 }).length;
const kib = (total / 1024).toFixed(1);
console.log(`initial JS: ${initial.join(", ")} = ${kib} KiB gzip (budget 150)`);
const wasm = readdirSync(join(dist, "assets")).filter((f) => f.endsWith(".wasm"));
for (const w of wasm) console.log(`wasm ${w}: ${(gzipSync(readFileSync(join(dist, "assets", w))).length / 1024).toFixed(1)} KiB gzip`);
if (total > 150 * 1024) process.exit(1);
