// CoreBackend for the Tauri desktop app: the core runs natively in Rust.
// Commands go through a synchronous Tauri command (keeps order); updates are
// pulled after a `core-updates` event. Concatenated UpdateBatch encodings are
// themselves a valid UpdateBatch, so pending chunks are simply joined.

import type { CoreBackend } from "./index.js";

interface TauriApi {
  invoke<T>(cmd: string, args?: unknown, options?: unknown): Promise<T>;
  listen(event: string, cb: () => void): Promise<() => void>;
}

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export async function tauriBackend(): Promise<CoreBackend> {
  const core = await import("@tauri-apps/api/core");
  const event = await import("@tauri-apps/api/event");
  const api: TauriApi = { invoke: core.invoke, listen: (e, cb) => event.listen(e, cb) };
  let queue: Uint8Array[] = [];
  let notify: () => void = () => {};
  const pull = async () => {
    const bytes = new Uint8Array(await api.invoke<ArrayBuffer>("core_drain"));
    if (bytes.length) {
      queue.push(bytes);
      notify();
    }
  };
  await api.listen("core-updates", () => void pull());
  return {
    dispatch(command) {
      void api.invoke("core_dispatch", command);
    },
    drain() {
      if (queue.length === 0) return new Uint8Array();
      const total = queue.reduce((n, b) => n + b.length, 0);
      const out = new Uint8Array(total);
      let off = 0;
      for (const b of queue) {
        out.set(b, off);
        off += b.length;
      }
      queue = [];
      return out;
    },
    // The native core restores and persists its own SQLite cache.
    restore: () => false,
    onPersist() {},
    onUpdates(cb) {
      notify = cb;
      void pull();
    },
  };
}
