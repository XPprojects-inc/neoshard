// Minimal IndexedDB key/value store for the core's offline snapshot.
// Storage only — no business logic.

const DB = "neoshard";
const STORE = "kv";

let conn: Promise<IDBDatabase> | undefined;

/** One shared connection; a blocked or stuck open fails after 3 s instead of hanging the app. */
function open(): Promise<IDBDatabase> {
  conn ??= new Promise<IDBDatabase>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("IndexedDB did not open in 3 s")), 3000);
    const req = indexedDB.open(DB, 1);
    req.onupgradeneeded = () => req.result.createObjectStore(STORE);
    req.onsuccess = () => {
      clearTimeout(timer);
      const db = req.result;
      // Another tab upgrading the schema: let it, reopen next time.
      db.onversionchange = () => {
        db.close();
        conn = undefined;
      };
      resolve(db);
    };
    req.onerror = () => {
      clearTimeout(timer);
      reject(req.error);
    };
    req.onblocked = () => {
      clearTimeout(timer);
      reject(new Error("IndexedDB is blocked by another tab"));
    };
  }).catch((e) => {
    conn = undefined;
    throw e;
  });
  return conn;
}

/** Drop the whole local database (offline cache, session, E2EE keys of this device). */
export async function idbWipe(): Promise<void> {
  if (typeof indexedDB === "undefined") return;
  const db = await conn?.catch(() => undefined);
  db?.close();
  conn = undefined;
  await new Promise<void>((resolve) => {
    const req = indexedDB.deleteDatabase(DB);
    req.onsuccess = req.onerror = req.onblocked = () => resolve();
  });
}

export async function idbGet(key: string): Promise<Uint8Array | undefined> {
  if (typeof indexedDB === "undefined") return undefined;
  const db = await open();
  return new Promise((resolve, reject) => {
    const r = db.transaction(STORE, "readonly").objectStore(STORE).get(key);
    r.onsuccess = () => resolve(r.result instanceof Uint8Array ? r.result : undefined);
    r.onerror = () => reject(r.error);
  });
}

export async function idbSet(key: string, value: Uint8Array): Promise<void> {
  if (typeof indexedDB === "undefined") return;
  const db = await open();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, "readwrite");
    tx.objectStore(STORE).put(value, key);
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
  });
}

export async function idbDelete(key: string): Promise<void> {
  if (typeof indexedDB === "undefined") return;
  const db = await open();
  return new Promise((resolve) => {
    const tx = db.transaction(STORE, "readwrite");
    tx.objectStore(STORE).delete(key);
    tx.oncomplete = () => resolve();
  });
}
