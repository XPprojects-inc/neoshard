// Attachments: uploading from the composer and showing them in messages.
// Bytes go straight to storage through a signed URL; the API only registers them.

import type { Attachment } from "@neoshard/core";
import { IconButton } from "@neoshard/ui";
import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { Portal } from "solid-js/web";

import { ExternalLink, FileIcon, X } from "../icons";
import { decryptToUrl, type FileKey } from "../e2eeFiles";
import { api } from "../state";

export const MAX_FILES = 10;
export const MAX_BYTES = 25 * 1024 * 1024;

export interface UploadItem {
  key: number;
  file: File;
  preview?: string;
  progress: number;
  id?: string;
  error?: string;
  /** E2EE conversations: the file's key (sent inside the encrypted message). */
  fileKey?: import("../e2eeFiles").FileKey;
}

export function formatSize(n: number): string {
  if (n < 1024) return `${n} Б`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(n < 10 * 1024 ? 1 : 0)} КБ`;
  return `${(n / 1024 / 1024).toFixed(1)} МБ`;
}

/** Register the file and PUT its bytes; reports progress 0..1. Returns the attachment id. */
export async function uploadAttachment(channelId: string, file: File, onProgress: (p: number) => void): Promise<string> {
  if (file.size > MAX_BYTES) throw new Error(`Файл больше ${formatSize(MAX_BYTES)}`);
  const type = file.type || "application/octet-stream";
  const r = await api<{ id: string; upload_url: string; message?: string }>("POST", `/channels/${channelId}/attachments`, { filename: file.name, content_type: type, size: file.size });
  if (r.status !== 201) throw new Error(r.status === 403 ? "Нет права прикреплять файлы" : r.status === 503 ? "На сервере не настроено хранилище файлов" : (r.body?.message ?? "Не удалось начать загрузку"));
  await new Promise<void>((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open("PUT", r.body.upload_url);
    xhr.setRequestHeader("content-type", type);
    xhr.upload.onprogress = (e) => e.lengthComputable && onProgress(e.loaded / e.total);
    xhr.onload = () => (xhr.status >= 200 && xhr.status < 300 ? resolve() : reject(new Error(`Загрузка не удалась (${xhr.status})`)));
    xhr.onerror = () => reject(new Error("Сеть недоступна"));
    xhr.send(file);
  });
  onProgress(1);
  return r.body.id;
}

/** Files waiting in the composer. */
export function UploadTray(props: { items: UploadItem[]; onRemove: (key: number) => void }) {
  return (
    <Show when={props.items.length}>
      <div class="upload-tray" role="list" aria-label="Вложения">
        <For each={props.items}>
          {(it) => (
            <div class="upload-card" role="listitem" classList={{ failed: !!it.error }}>
              <Show when={it.preview} fallback={<span class="upload-icon"><FileIcon size={32} /></span>}>
                <img src={it.preview} alt="" />
              </Show>
              <span class="upload-name" title={it.file.name}>{it.file.name}</span>
              <small>{it.error ?? (it.id ? formatSize(it.file.size) : `${Math.round(it.progress * 100)}%`)}</small>
              <Show when={!it.id && !it.error}>
                <span class="upload-bar" style={{ "--p": it.progress }} aria-hidden="true" />
              </Show>
              <IconButton label={`Убрать ${it.file.name}`} class="upload-remove" onClick={() => props.onRemove(it.key)}>
                <X size={14} />
              </IconButton>
            </div>
          )}
        </For>
      </div>
    </Show>
  );
}

const [viewing, setViewing] = createSignal<{ url: string; name: string }>();

/** Attachments of a message. */
export function AttachmentList(props: { items: Attachment[]; safeUrl: (u: string) => string | undefined; onLoad?: () => void }) {
  const images = () => props.items.filter((a) => a.contentType.startsWith("image/"));
  const files = () => props.items.filter((a) => !a.contentType.startsWith("image/"));
  return (
    <>
      <Show when={images().length}>
        <div class="att-images" classList={{ grid: images().length > 1 }}>
          <For each={images()}>
            {(a) => (
              <Show when={props.safeUrl(a.url)}>
                {(url) => (
                  <button class="att-image" aria-label={`Открыть изображение ${a.filename}`} onClick={() => setViewing({ url: url(), name: a.filename })}>
                    <img src={url()} alt={a.filename} loading="lazy" referrerpolicy="no-referrer" width={a.width || undefined} height={a.height || undefined} onLoad={() => props.onLoad?.()} />
                  </button>
                )}
              </Show>
            )}
          </For>
        </div>
      </Show>
      <For each={files()}>
        {(a) => (
          <Show when={props.safeUrl(a.url)}>
            {(url) => (
              <a class="att-file" href={url()} target="_blank" rel="noopener noreferrer" download={a.filename}>
                <FileIcon size={30} aria-hidden="true" />
                <span>
                  <b>{a.filename}</b>
                  <small>{formatSize(Number(a.size))}</small>
                </span>
              </a>
            )}
          </Show>
        )}
      </For>
    </>
  );
}

/** Attachments of an E2EE message: decrypted on this device. */
export function EncryptedAttachments(props: { items: Attachment[]; keys: FileKey[]; safeUrl: (u: string) => string | undefined; onLoad?: () => void }) {
  return (
    <div class="att-images">
      <For each={props.items}>
        {(a, i) => {
          const k = () => props.keys[i()];
          const [url, setUrl] = createSignal<string>();
          const [err, setErr] = createSignal("");
          const open = () => {
            const src = props.safeUrl(a.url);
            if (!src || !k()) return Promise.reject(new Error("нет ключа"));
            return decryptToUrl(src, k()!).then((u) => (setUrl(u), u)).catch((e: Error) => (setErr(e.message), Promise.reject(e)));
          };
          // Images are decrypted right away; other files on demand.
          if (k()?.t.startsWith("image/")) void open().catch(() => {});
          return (
            <Show
              when={k()?.t.startsWith("image/")}
              fallback={
                <button
                  class="att-file"
                  onClick={async () => {
                    const u = url() ?? (await open().catch(() => undefined));
                    if (!u) return;
                    const link = document.createElement("a");
                    link.href = u;
                    link.download = k()?.n ?? "file";
                    link.click();
                  }}
                >
                  <FileIcon size={30} aria-hidden="true" />
                  <span>
                    <b>{k()?.n ?? "файл"}</b>
                    <small>{err() || `${formatSize(k()?.s ?? 0)} · зашифрован`}</small>
                  </span>
                </button>
              }
            >
              <Show when={url()} fallback={<span class="att-locked">{err() ? `Не удалось расшифровать: ${err()}` : "Расшифровываем…"}</span>}>
                <button class="att-image" aria-label={`Открыть изображение ${k()!.n}`} onClick={() => setViewing({ url: url()!, name: k()!.n })}>
                  <img src={url()} alt={k()!.n} onLoad={() => props.onLoad?.()} />
                </button>
              </Show>
            </Show>
          );
        }}
      </For>
    </div>
  );
}

/** Full-screen image viewer (one per app). */
export function Lightbox() {
  return (
    <Show when={viewing()}>
      <LightboxBody url={viewing()!.url} name={viewing()!.name} onClose={() => setViewing(undefined)} />
    </Show>
  );
}

function LightboxBody(props: { url: string; name: string; onClose: () => void }) {
  let box!: HTMLDivElement;
  const prev = document.activeElement as HTMLElement | null;
  const onKey = (e: KeyboardEvent) => e.key === "Escape" && props.onClose();
  onMount(() => {
    document.addEventListener("keydown", onKey);
    box.focus();
  });
  onCleanup(() => {
    document.removeEventListener("keydown", onKey);
    prev?.focus?.();
  });
  return (
    <Portal>
      <div ref={box} class="lightbox" role="dialog" aria-modal="true" aria-label={props.name} tabindex="-1" onClick={(e) => e.target === e.currentTarget && props.onClose()}>
        <img src={props.url} alt={props.name} />
        <div class="lightbox-bar">
          <span>{props.name}</span>
          <a href={props.url} target="_blank" rel="noopener noreferrer" class="ns-icon-btn" aria-label="Открыть оригинал">
            <ExternalLink size={18} />
          </a>
          <IconButton label="Закрыть" onClick={props.onClose}>
            <X size={18} />
          </IconButton>
        </div>
      </div>
    </Portal>
  );
}
