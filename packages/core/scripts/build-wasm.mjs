// Build the WASM core: cargo (release-wasm: opt-level z, fat LTO) → wasm-bindgen → wasm-opt -Oz.
// Two variants (ADR-0019):
//   wasm/       base core, loaded on every start — budget 400 KiB gzip;
//   wasm-e2ee/  core + MLS, loaded instead only on devices with encrypted DMs turned on.
// NEOSHARD_WASM_VARIANTS=base limits the build to one variant.
import { execFileSync } from "node:child_process";
import { existsSync, statSync, readFileSync } from "node:fs";
import { gzipSync } from "node:zlib";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "../../..");
const run = (cmd, args, opts = {}) => {
  console.log(`$ ${cmd} ${args.join(" ")}`);
  execFileSync(cmd, args, { stdio: "inherit", cwd: root, ...opts });
};

const variants = [
  { name: "base", features: "wasm", out: "wasm", budgetKiB: 400 },
  { name: "e2ee", features: "wasm,e2ee", out: "wasm-e2ee", budgetKiB: 480 },
].filter((v) => (process.env.NEOSHARD_WASM_VARIANTS ?? "base,e2ee").split(",").includes(v.name));

const wasmOpt = [join(root, "node_modules/binaryen/bin/wasm-opt"), "wasm-opt"].find((p) => p === "wasm-opt" || existsSync(p));
let failed = false;
for (const v of variants) {
  const out = resolve(here, "..", v.out);
  run("cargo", [
    "rustc", "-p", "neoshard-core", "--lib", "--crate-type", "cdylib",
    "--target", "wasm32-unknown-unknown",
    "--no-default-features", "--features", v.features,
    "--profile", "release-wasm",
  ]);
  const raw = join(root, "target/wasm32-unknown-unknown/release-wasm/neoshard_core.wasm");
  run("wasm-bindgen", ["--target", "web", "--out-dir", out, "--out-name", "neoshard_core", raw]);
  const bg = join(out, "neoshard_core_bg.wasm");
  try {
    run(process.execPath, [wasmOpt, "-Oz", "--enable-bulk-memory", "--enable-nontrapping-float-to-int", "--enable-sign-ext", "--enable-mutable-globals", "--enable-reference-types", "-o", bg, bg]);
  } catch {
    console.warn("wasm-opt failed or not found; shipping unoptimized wasm");
  }
  const size = statSync(bg).size;
  const gz = gzipSync(readFileSync(bg), { level: 9 }).length;
  console.log(`[${v.name}] neoshard_core_bg.wasm: ${(size / 1024).toFixed(0)} KiB, gzip ${(gz / 1024).toFixed(0)} KiB (budget ${v.budgetKiB} KiB)`);
  if (gz > v.budgetKiB * 1024) {
    console.error(`[${v.name}] WASM size budget exceeded`);
    failed = true;
  }
}
if (failed) process.exit(1);
