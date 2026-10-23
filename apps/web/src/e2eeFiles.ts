// Files in end-to-end encrypted conversations. Each file is encrypted on the
// device with its own random AES-256-GCM key; the server stores only
// ciphertext. The keys travel in a small manifest appended to the message
// text, which itself is MLS-encrypted by the core — so only the members of the
// conversation can open the files.

export interface FileKey {
  /** original name, type and size */
  n: string;
  t: string;
  s: number;
  /** base64 raw AES key and IV */
  k: string;
  iv: string;
}

const MARK = "⁣ns-files:";

const b64 = (buf: ArrayBuffer | Uint8Array) => {
  const bytes = buf instanceof Uint8Array ? buf : new Uint8Array(buf);
  let s = "";
  for (const b of bytes) s += String.fromCharCode(b);
  return btoa(s);
};
const unb64 = (s: string) => Uint8Array.from(atob(s), (c) => c.charCodeAt(0));

/** Encrypt a file; returns the ciphertext (to upload) and its key (to put in the message). */
export async function encryptFile(file: File): Promise<{ blob: Blob; key: FileKey }> {
  const key = await crypto.subtle.generateKey({ name: "AES-GCM", length: 256 }, true, ["encrypt"]);
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const ct = await crypto.subtle.encrypt({ name: "AES-GCM", iv }, key, await file.arrayBuffer());
  const raw = await crypto.subtle.exportKey("raw", key);
  return {
    blob: new Blob([ct], { type: "application/octet-stream" }),
    key: { n: file.name.slice(0, 200), t: file.type || "application/octet-stream", s: file.size, k: b64(raw), iv: b64(iv) },
  };
}

/** Message text with the file keys appended (the whole thing gets MLS-encrypted). */
export function withManifest(text: string, keys: FileKey[]): string {
  if (!keys.length) return text;
  return `${text}\n${MARK}${btoa(unescape(encodeURIComponent(JSON.stringify(keys))))}`;
}

/** Split decrypted message text into the visible text and the file keys. */
export function parseManifest(content: string): { text: string; keys: FileKey[] } {
  const i = content.lastIndexOf(MARK);
  if (i < 0) return { text: content, keys: [] };
  try {
    const keys = JSON.parse(decodeURIComponent(escape(atob(content.slice(i + MARK.length).trim())))) as FileKey[];
    if (!Array.isArray(keys)) throw new Error("bad manifest");
    return { text: content.slice(0, i).replace(/\n$/, ""), keys };
  } catch {
    return { text: content, keys: [] };
  }
}

const cache = new Map<string, Promise<string>>();

/** Download, decrypt and return an object URL for the original file (cached per URL). */
export function decryptToUrl(url: string, key: FileKey): Promise<string> {
  let p = cache.get(url);
  if (!p) {
    p = (async () => {
      const res = await fetch(url, { referrerPolicy: "no-referrer" });
      if (!res.ok) throw new Error(`файл недоступен (${res.status})`);
      const k = await crypto.subtle.importKey("raw", unb64(key.k), "AES-GCM", false, ["decrypt"]);
      const plain = await crypto.subtle.decrypt({ name: "AES-GCM", iv: unb64(key.iv) }, k, await res.arrayBuffer());
      return URL.createObjectURL(new Blob([plain], { type: key.t }));
    })();
    p.catch(() => cache.delete(url));
    cache.set(url, p);
  }
  return p;
}
