// Full-screen user settings (sections on the left, content on the right).

import { deviceE2ee, isTauri } from "@neoshard/core";
import { applyTheme, Avatar, builtinThemes, Button, Modal, parseTheme, Switch, TextField, toast } from "@neoshard/ui";
import { createResource, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { Portal } from "solid-js/web";

import { copy, statusName } from "../actions";
import { Bell, Headphones, Keyboard, Lock, LogOut, MonitorSmartphone, Palette, Shield, Sparkles, UserIcon, X } from "../icons";
import { installPlugin, listPlugins, removePlugin, stopAllPlugins } from "../plugins/host";
import { prefs, setPref } from "../prefs";
import { cssUrl, UserAvatar } from "../profile/components";
import { hex } from "../profile/cosmetics";
import { fetchProfile } from "../profile/data";
import { ProfileEditor } from "../profile/ProfileEditor";
import { api, asset, core, sid, state } from "../state";

export type SettingsTab = "account" | "profiles" | "security" | "privacy" | "devices" | "appearance" | "voice" | "notifications" | "keybinds" | "plugins";
type Tab = SettingsTab;

interface SessionRow {
  id: string;
  device: string;
  ip: string;
  last_seen_at: number;
  current: boolean;
}

const NAV: Array<{ group: string; items: Array<[Tab, string, () => JSX.Element]> }> = [
  {
    group: "Настройки пользователя",
    items: [
      ["account", "Моя учётная запись", () => <UserIcon size={16} />],
      ["profiles", "Профили", () => <Sparkles size={16} />],
      ["security", "Безопасность", () => <Shield size={16} />],
      ["privacy", "Шифрование", () => <Lock size={16} />],
      ["devices", "Устройства", () => <MonitorSmartphone size={16} />],
    ],
  },
  {
    group: "Настройки приложения",
    items: [
      ["appearance", "Внешний вид", () => <Palette size={16} />],
      ["voice", "Голос и видео", () => <Headphones size={16} />],
      ["notifications", "Уведомления", () => <Bell size={16} />],
      ["keybinds", "Сочетания клавиш", () => <Keyboard size={16} />],
      ["plugins", "Плагины", () => <Sparkles size={16} />],
    ],
  },
];

export function SettingsModal(props: { open: boolean; onClose: () => void; initial?: Tab }) {
  return (
    <Show when={props.open}>
      <SettingsLayer onClose={props.onClose} initial={props.initial} />
    </Show>
  );
}

function SettingsLayer(props: { onClose: () => void; initial?: Tab }) {
  const [tab, setTab] = createSignal<Tab>(props.initial ?? "account");
  let root!: HTMLDivElement;
  const prev = document.activeElement as HTMLElement | null;
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape" && !document.querySelector(".ns-backdrop, .ns-popover")) props.onClose();
  };
  onMount(() => {
    document.addEventListener("keydown", onKey);
    requestAnimationFrame(() => root.querySelector<HTMLElement>('[aria-current="page"]')?.focus());
  });
  onCleanup(() => {
    document.removeEventListener("keydown", onKey);
    prev?.focus?.();
  });
  const logout = () => {
    props.onClose();
    stopAllPlugins();
    core().send({ case: "logout", value: {} });
  };
  const page = (id: Tab, title: string, body: () => JSX.Element) => (
    <Show when={tab() === id}>
      <Page title={title}>{body()}</Page>
    </Show>
  );
  return (
    <Portal>
      <div ref={root} class="settings-layer" role="dialog" aria-modal="true" aria-label="Настройки">
        <nav class="settings-nav" aria-label="Разделы настроек">
          <div class="settings-nav-inner">
            <For each={NAV}>
              {(g) => (
                <>
                  <h2>{g.group}</h2>
                  <For each={g.items}>
                    {([id, label, icon]) => (
                      <button class="settings-nav-item" aria-current={tab() === id ? "page" : undefined} onClick={() => setTab(id)}>
                        {icon()}
                        {label}
                      </button>
                    )}
                  </For>
                  <div class="settings-sep" />
                </>
              )}
            </For>
            <button class="settings-nav-item danger" onClick={logout}>
              <LogOut size={16} />
              Выйти
            </button>
            <p class="settings-version">NeoShard · веб-клиент</p>
          </div>
        </nav>
        <main class="settings-content">
          <div class="settings-content-inner">
            <Show when={tab() === "account"}>
              <Account onTab={setTab} />
            </Show>
            <Show when={tab() === "profiles"}>
              <ProfileEditor />
            </Show>
            {page("security", "Безопасность", () => <Security />)}
            {page("privacy", "Шифрование", () => <Privacy />)}
            {page("devices", "Устройства", () => <Devices />)}
            {page("appearance", "Внешний вид", () => <Appearance />)}
            {page("voice", "Голос и видео", () => <VoiceSettings />)}
            {page("notifications", "Уведомления", () => <Notifications />)}
            {page("keybinds", "Сочетания клавиш", () => <Keybinds />)}
            {page("plugins", "Плагины", () => <Plugins />)}
          </div>
          <div class="settings-close">
            <button class="settings-close-btn" aria-label="Закрыть настройки (Esc)" onClick={props.onClose}>
              <X size={18} />
            </button>
            <small aria-hidden="true">ESC</small>
          </div>
        </main>
      </div>
    </Portal>
  );
}

function Page(props: { title: string; children: JSX.Element }) {
  return (
    <div class="settings-page">
      <h1>{props.title}</h1>
      <div class="settings-stack">{props.children}</div>
    </div>
  );
}

function Account(props: { onTab: (t: Tab) => void }) {
  const me = () => state.me;
  // The same banner as the profile card: image, colour, theme, or the avatar tint.
  const [profile] = createResource(() => sid(state.me?.id ?? 0n), (id) => fetchProfile(id, undefined, true));
  const banner = () => {
    const p = profile();
    const key = p?.banner;
    if (key && asset(key)) return `center / cover no-repeat ${cssUrl(asset(key)!)}`;
    if (p?.banner_color !== undefined && p?.banner_color !== null) return hex(p.banner_color);
    if (p?.theme_colors?.length === 2) return `linear-gradient(120deg, ${hex(p.theme_colors[0])}, ${hex(p.theme_colors[1])})`;
    return undefined;
  };
  const [pw, setPw] = createSignal(false);
  const [del, setDel] = createSignal(false);
  return (
    <div class="settings-page">
      <h1>Моя учётная запись</h1>
      <div class="acct-card">
        <div class="acct-banner" style={banner() ? { background: banner() } : undefined} />
        <div class="acct-head">
          <UserAvatar user={me()} size={80} status={statusName(state.myPresence.status)} />
          <div style={{ flex: 1, "min-width": 0 }}>
            <h2>{me()?.displayName || me()?.username}</h2>
            <span class="edited">@{me()?.username}</span>
          </div>
          <Button onClick={() => props.onTab("profiles")}>Редактировать профиль</Button>
        </div>
        <div class="acct-fields">
          <div class="acct-field">
            <div>
              <h3>Отображаемое имя</h3>
              <span>{me()?.displayName || "не задано"}</span>
            </div>
            <Button size="sm" variant="secondary" onClick={() => props.onTab("profiles")}>Изменить</Button>
          </div>
          <div class="acct-field">
            <div>
              <h3>Имя пользователя</h3>
              <span>{me()?.username}</span>
            </div>
            <Button size="sm" variant="secondary" onClick={() => void copy(me()?.username ?? "", "Имя скопировано")}>Копировать</Button>
          </div>
          <div class="acct-field">
            <div>
              <h3>ID пользователя</h3>
              <span class="mono">{sid(me()?.id ?? 0n)}</span>
            </div>
            <Button size="sm" variant="secondary" onClick={() => void copy(sid(me()?.id ?? 0n), "ID скопирован")}>Копировать</Button>
          </div>
        </div>
      </div>
      <h2 class="settings-h2">Пароль и вход</h2>
      <div>
        <Button variant="secondary" onClick={() => setPw(true)}>Сменить пароль</Button>
      </div>
      <h2 class="settings-h2">Удаление аккаунта</h2>
      <p class="pe-hint">Профиль, почта, ключи и сеансы удаляются. Ваши сообщения остаются в беседах, но привязываются к анонимному аккаунту.</p>
      <div>
        <Button variant="danger" onClick={() => setDel(true)}>Удалить аккаунт</Button>
      </div>
      <PasswordModal open={pw()} onClose={() => setPw(false)} />
      <DeleteModal open={del()} onClose={() => setDel(false)} />
    </div>
  );
}

function PasswordModal(props: { open: boolean; onClose: () => void }) {
  const [old, setOld] = createSignal("");
  const [next, setNext] = createSignal("");
  const [code, setCode] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const submit = async () => {
    setBusy(true);
    const r = await api<{ message?: string }>("POST", "/auth/password", { password: old(), new_password: next(), code: code() || undefined });
    setBusy(false);
    if (r.status === 200 || r.status === 204) {
      toast("Пароль изменён, остальные сеансы завершены");
      props.onClose();
    } else toast(r.body?.message ?? "Неверный пароль или код", "error");
  };
  return (
    <Modal open={props.open} title="Сменить пароль" onClose={props.onClose}>
      <TextField label="Текущий пароль" type="password" value={old()} onInput={setOld} autocomplete="current-password" />
      <TextField label="Новый пароль (от 8 символов)" type="password" value={next()} onInput={setNext} autocomplete="new-password" />
      <Show when={state.me?.mfaEnabled}>
        <TextField label="Код 2FA" value={code()} onInput={setCode} inputmode="numeric" autocomplete="one-time-code" />
      </Show>
      <Button loading={busy()} disabled={next().length < 8} onClick={submit}>Сохранить</Button>
    </Modal>
  );
}

function DeleteModal(props: { open: boolean; onClose: () => void }) {
  const [password, setPassword] = createSignal("");
  const [code, setCode] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const submit = async () => {
    setBusy(true);
    const r = await api<{ message?: string }>("POST", "/users/@me/delete", { password: password(), code: code() || undefined });
    setBusy(false);
    if (r.status === 204) {
      stopAllPlugins();
      core().send({ case: "logout", value: {} });
    } else toast(r.status === 409 ? "Сначала передайте или удалите свои серверы" : (r.body?.message ?? "Неверный пароль или код"), "error");
  };
  return (
    <Modal open={props.open} title="Удалить аккаунт?" onClose={props.onClose}>
      <p style={{ margin: 0 }}>Это нельзя отменить.</p>
      <TextField label="Пароль" type="password" value={password()} onInput={setPassword} autocomplete="current-password" />
      <Show when={state.me?.mfaEnabled}>
        <TextField label="Код 2FA" value={code()} onInput={setCode} inputmode="numeric" />
      </Show>
      <Button variant="danger" loading={busy()} disabled={!password()} onClick={submit}>Удалить навсегда</Button>
    </Modal>
  );
}

function Security() {
  const [setup, setSetup] = createSignal<{ secret: string; otpauth_url: string }>();
  const [code, setCode] = createSignal("");
  const [password, setPassword] = createSignal("");
  const [backup, setBackup] = createSignal<string[]>();
  const start = async () => {
    const r = await api<{ secret: string; otpauth_url: string }>("POST", "/auth/mfa/totp/setup", {});
    if (r.status === 200) setSetup(r.body);
    else toast("2FA уже включена", "error");
  };
  const enable = async () => {
    const r = await api<{ backup_codes: string[] }>("POST", "/auth/mfa/totp/enable", { code: code(), password: password() });
    if (r.status === 200) {
      setBackup(r.body.backup_codes);
      setSetup(undefined);
      toast("Двухфакторная аутентификация включена");
    } else toast("Неверный код или пароль", "error");
  };
  return (
    <>
      <h2 class="settings-h2">Двухфакторная аутентификация</h2>
      <Show when={state.me?.mfaEnabled && !backup()}>
        <p class="pe-hint">Включена. Для входа нужен код из приложения.</p>
      </Show>
      <Show when={!state.me?.mfaEnabled && !setup() && !backup()}>
        <div>
          <Button variant="secondary" onClick={start}>Настроить приложение-аутентификатор</Button>
        </div>
      </Show>
      <Show when={setup()}>
        <p style={{ margin: 0 }}>Добавьте ключ в приложение (Яндекс Ключ, Aegis, FreeOTP и т.п.):</p>
        <code class="md-code" style={{ "word-break": "break-all" }}>{setup()!.secret}</code>
        <TextField label="Код из приложения" value={code()} onInput={setCode} inputmode="numeric" autocomplete="one-time-code" />
        <TextField label="Пароль" type="password" value={password()} onInput={setPassword} autocomplete="current-password" />
        <div>
          <Button onClick={enable}>Включить</Button>
        </div>
      </Show>
      <Show when={backup()}>
        <p style={{ margin: 0 }}>Резервные коды — сохраните их, каждый работает один раз:</p>
        <pre class="md-pre">{backup()!.join("\n")}</pre>
      </Show>
    </>
  );
}

function Privacy() {
  return (
    <>
      <p class="pe-hint">
        Зашифрованные ЛС (MLS): ключи создаются и хранятся только на устройстве, сервер видит лишь шифротекст. В браузере это загружает ядро побольше (≈440 КБ вместо ≈150 КБ).
      </p>
      <Show when={!isTauri()} fallback={<p>В настольном приложении шифрование включено всегда.</p>}>
        <Switch
          label="Зашифрованные ЛС на этом устройстве"
          description="После переключения страница перезагрузится."
          checked={deviceE2ee.get()}
          onChange={(on) => {
            deviceE2ee.set(on);
            location.reload();
          }}
        />
      </Show>
    </>
  );
}

function Devices() {
  const [sessions, { refetch }] = createResource(async () => (await api<SessionRow[]>("GET", "/sessions")).body ?? []);
  return (
    <>
      <p class="pe-hint">Устройства, где выполнен вход. Завершите сеанс, если не узнаёте устройство.</p>
      <For each={sessions() ?? []}>
        {(s) => (
          <div class="device-row">
            <MonitorSmartphone size={28} aria-hidden="true" />
            <div style={{ flex: 1 }}>
              <b>{s.device || "Неизвестное устройство"}</b>
              <Show when={s.current}>
                <span class="tag-chip">это устройство</span>
              </Show>
              <br />
              <small class="edited">
                {s.ip} · {new Date(s.last_seen_at).toLocaleString("ru-RU")}
              </small>
            </div>
            <Show when={!s.current}>
              <Button size="sm" variant="ghost" onClick={async () => (await api("DELETE", `/sessions/${s.id}`), refetch())}>Завершить</Button>
            </Show>
          </div>
        )}
      </For>
    </>
  );
}

function VoiceSettings() {
  const [devices] = createResource(async () => {
    try {
      return await navigator.mediaDevices.enumerateDevices();
    } catch {
      return [];
    }
  });
  const list = (kind: MediaDeviceKind) => (devices() ?? []).filter((d) => d.kind === kind);
  const pick = (label: string, kind: MediaDeviceKind, key: "micId" | "camId" | "speakerId") => (
    <label class="ns-field">
      <span class="ns-label">{label}</span>
      <select class="ns-select" value={prefs[key]} onChange={(e) => setPref(key, e.currentTarget.value)}>
        <option value="">По умолчанию</option>
        <For each={list(kind)}>{(d, i) => <option value={d.deviceId}>{d.label || `Устройство ${i() + 1}`}</option>}</For>
      </select>
    </label>
  );
  return (
    <>
      {pick("Микрофон", "audioinput", "micId")}
      {pick("Динамики", "audiooutput", "speakerId")}
      {pick("Камера", "videoinput", "camId")}
      <p class="pe-hint">Названия устройств браузер показывает после первого разрешения доступа к микрофону.</p>
      <Switch label="Шумоподавление" description="Подавление фонового шума и эха при захвате микрофона." checked={prefs.noiseSuppression} onChange={(v) => setPref("noiseSuppression", v)} />
    </>
  );
}

function Notifications() {
  const [perm, setPerm] = createSignal<NotificationPermission>(typeof Notification === "undefined" ? "denied" : Notification.permission);
  return (
    <>
      <Switch
        label="Уведомления на рабочем столе"
        description={perm() === "denied" ? "Браузер запретил уведомления для этого сайта — разрешите их в настройках браузера." : "Об упоминаниях и личных сообщениях, когда вкладка неактивна."}
        checked={prefs.desktopNotifications && perm() === "granted"}
        disabled={perm() === "denied"}
        onChange={async (on) => {
          if (on && perm() !== "granted") setPerm(await Notification.requestPermission());
          setPref("desktopNotifications", on);
        }}
      />
      <Switch label="Звук уведомлений" checked={prefs.notificationSound} onChange={(v) => setPref("notificationSound", v)} />
    </>
  );
}

const KEYS: Array<[string, string]> = [
  ["Enter", "Отправить сообщение"],
  ["Shift + Enter", "Новая строка"],
  ["↑ в пустом поле", "Изменить последнее сообщение"],
  ["Esc", "Отменить ответ или правку, закрыть окно"],
  ["Alt + ↑ / ↓", "Переставить сервер в списке (фокус на сервере)"],
  ["Shift + F10 / ПКМ", "Меню действий с сообщением или пользователем"],
  ["Ctrl + K", "Быстрый переход по каналам"],
];

function Keybinds() {
  return (
    <div class="keys">
      <For each={KEYS}>
        {([k, what]) => (
          <div class="keys-row">
            <span>{what}</span>
            <kbd>{k}</kbd>
          </div>
        )}
      </For>
    </div>
  );
}

function Appearance() {
  const [json, setJson] = createSignal("");
  const pick = (name: string) => {
    applyTheme(builtinThemes[name]);
    try {
      localStorage.setItem("neoshard.themeName", name);
      localStorage.removeItem("neoshard.theme");
    } catch {
      // ignore
    }
  };
  const importTheme = () => {
    let raw: unknown;
    try {
      raw = JSON.parse(json());
    } catch {
      toast("Это не JSON", "error");
      return;
    }
    const t = parseTheme(raw);
    if (!t) return toast("Тема не прошла проверку", "error");
    applyTheme(t);
    try {
      localStorage.setItem("neoshard.theme", JSON.stringify(t));
      localStorage.removeItem("neoshard.themeName");
    } catch {
      // ignore
    }
    toast(`Тема «${t.name}» применена`);
  };
  return (
    <>
      <div style={{ display: "flex", gap: "8px" }}>
        <Button variant="secondary" onClick={() => pick("neoshard-dark")}>Тёмная</Button>
        <Button variant="secondary" onClick={() => pick("neoshard-light")}>Светлая</Button>
        <Button variant="secondary" onClick={() => pick("neoshard-midnight")}>Полночь</Button>
      </div>
      <div class="ns-field">
        <label class="ns-label" for="theme-json">Своя тема (JSON)</label>
        <textarea id="theme-json" class="ns-input" style={{ height: "120px", padding: "8px" }} value={json()} onInput={(e) => setJson(e.currentTarget.value)} placeholder='{"name":"моя","scheme":"dark","colors":{"accent":"#ff8800"}}' />
      </div>
      <Button onClick={importTheme}>Применить</Button>
    </>
  );
}

function Plugins() {
  const [url, setUrl] = createSignal("");
  const [list, setList] = createSignal(listPlugins());
  return (
    <>
      <p style={{ margin: 0 }} class="edited">Плагины работают в изолированной песочнице и видят только то, что вы разрешите.</p>
      <TextField label="Адрес плагина (https://…/plugin.js)" value={url()} onInput={setUrl} />
      <Button
        onClick={async () => {
          try {
            await installPlugin(url().trim());
            setList(listPlugins());
            setUrl("");
          } catch (e) {
            toast(String(e), "error");
          }
        }}
      >
        Установить
      </Button>
      <For each={list()} fallback={<span class="edited">Нет установленных плагинов</span>}>
        {(p) => (
          <div style={{ display: "flex", "justify-content": "space-between", "align-items": "center" }}>
            <span>
              <b>{p.manifest.name}</b> — {p.manifest.permissions.join(", ") || "без разрешений"}
            </span>
            <Button size="sm" variant="ghost" onClick={() => (removePlugin(p.url), setList(listPlugins()))}>Удалить</Button>
          </div>
        )}
      </For>
    </>
  );
}
