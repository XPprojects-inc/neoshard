// @neoshard/core — typed entry point to the NeoShard client core.
//
// The core runs as WASM in the browser (or natively in Tauri). This wrapper
// only moves bytes: commands are encoded once and passed in; updates are
// drained as one batch per animation frame (ADR-0009). No business logic here.

import { create, fromBinary, toBinary, type MessageInitShape } from "@bufbuild/protobuf";
import {
  CommandSchema,
  UpdateBatchSchema,
  type Update,
} from "./gen/neoshard/v1/core_pb.js";
import { idbDelete, idbGet, idbSet, idbWipe } from "./idb.js";

export { idbWipe as wipeLocalData };

export * from "./gen/neoshard/v1/core_pb.js";
export * from "./gen/neoshard/v1/models_pb.js";
export * as gateway from "./gen/neoshard/v1/gateway_pb.js";
export { create, fromJson } from "@bufbuild/protobuf";
export { Permission, hasPermission } from "./permissions.js";
export { isTauri, tauriBackend } from "./tauri.js";

/** The low-level transport any core implementation exposes (WASM or Tauri native). */
export interface CoreBackend {
  dispatch(command: Uint8Array): void;
  drain(): Uint8Array;
  restore(snapshot: Uint8Array): boolean;
  onPersist(cb: (snapshot: Uint8Array) => void): void;
  onUpdates(cb: () => void): void;
}

/** Command payload, e.g. `{ case: "sendMessage", value: { channelId, content } }`. */
export type CommandKind = MessageInitShape<typeof CommandSchema>["kind"];
export type UpdateHandler = (updates: Update[]) => void;

const SNAPSHOT_KEY = "snapshot";
const E2EE_KEY = "neoshard.e2ee";

/**
 * Encrypted DMs on this device. The MLS build of the core is ~2.9x larger,
 * so it is loaded only where the user turned encryption on (ADR-0019).
 */
export const deviceE2ee = {
  get(): boolean {
    try {
      return localStorage.getItem(E2EE_KEY) === "1";
    } catch {
      return false;
    }
  },
  set(on: boolean): void {
    try {
      if (on) localStorage.setItem(E2EE_KEY, "1");
      else localStorage.removeItem(E2EE_KEY);
    } catch {
      // Private mode: the setting lasts for this tab only.
    }
  },
};

export class NeoShardCore {
  private handlers = new Set<UpdateHandler>();
  private scheduled = false;
  private nextRequest = 1n;
  /** Whether this core build can encrypt (MLS). */
  e2ee = false;

  private constructor(private backend: CoreBackend) {
    backend.onUpdates(() => this.schedule());
    backend.onPersist((bytes) => {
      void idbSet(SNAPSHOT_KEY, bytes.slice());
    });
  }

  /** Load the WASM core (browser). Restores the offline snapshot before returning. */
  static async loadWasm(clientName = "neoshard-web", opts: { e2ee?: boolean } = {}): Promise<NeoShardCore> {
    const e2ee = opts.e2ee ?? deviceE2ee.get();
    const mod = e2ee ? await import("../wasm-e2ee/neoshard_core.js") : await import("../wasm/neoshard_core.js");
    await mod.default();
    const backend = new mod.WasmCore(clientName) as unknown as CoreBackend;
    const core = await NeoShardCore.withBackend(backend);
    core.e2ee = e2ee;
    return core;
  }

  /** Use any backend (Tauri native core, tests). */
  static async withBackend(backend: CoreBackend, restore = true): Promise<NeoShardCore> {
    const core = new NeoShardCore(backend);
    if (restore) {
      try {
        const snap = await idbGet(SNAPSHOT_KEY);
        if (snap) backend.restore(snap);
      } catch {
        // Offline cache is best effort.
      }
    }
    core.schedule();
    return core;
  }

  /** Subscribe to update batches. Returns an unsubscribe function. */
  subscribe(handler: UpdateHandler): () => void {
    this.handlers.add(handler);
    this.schedule();
    return () => this.handlers.delete(handler);
  }

  /** Send a command; returns its request id (echoed in `CommandResult`). */
  send(kind: CommandKind): bigint {
    const requestId = this.nextRequest++;
    const cmd = create(CommandSchema, { requestId, kind });
    this.backend.dispatch(toBinary(CommandSchema, cmd));
    this.schedule();
    return requestId;
  }

  /** Forget the offline cache (logout). */
  async clearCache(): Promise<void> {
    await idbDelete(SNAPSHOT_KEY);
  }

  /** Drain now (tests); the UI normally drains once per animation frame. */
  flush(): void {
    this.scheduled = false;
    const bytes = this.backend.drain();
    if (bytes.length === 0) return;
    const batch = fromBinary(UpdateBatchSchema, bytes);
    for (const h of this.handlers) h(batch.updates);
  }

  private schedule(): void {
    if (this.scheduled) return;
    this.scheduled = true;
    const run = () => {
      if (this.scheduled) this.flush();
    };
    // Once per frame while visible. Hidden tabs get no animation frames at all,
    // so a timer guarantees updates (messages, notifications) are still processed.
    const hidden = typeof document !== "undefined" && document.visibilityState === "hidden";
    if (!hidden && typeof requestAnimationFrame === "function") requestAnimationFrame(run);
    setTimeout(run, hidden ? 50 : 100);
  }
}
