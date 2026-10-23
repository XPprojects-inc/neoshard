// Login / registration / 2FA. The core performs the requests and keeps the session.

import { AuthPhase } from "@neoshard/core";
import { Button, TextField } from "@neoshard/ui";
import { createSignal, Match, Show, Switch } from "solid-js";

import config from "../../neoshard.config.json";
import { FIXED_INSTANCE, INSTANCE_KEY, instanceUrl } from "../instance";
import { core, state } from "../state";

const customInstance = !FIXED_INSTANCE && config.allowCustomInstance;

export function Auth() {
  const [mode, setMode] = createSignal<"login" | "register">("login");
  const [instance, setInstance] = createSignal(instanceUrl());
  const [login, setLogin] = createSignal("");
  const [email, setEmail] = createSignal("");
  const [password, setPassword] = createSignal("");
  const [code, setCode] = createSignal("");
  const busy = () => state.auth.phase === AuthPhase.PENDING;
  // Collapsed when a server is already known; the field opens on «Изменить» or when empty.
  const [editServer, setEditServer] = createSignal(!instance().trim());
  const host = () => base().replace(/^https?:\/\//, "");

  const base = () => {
    const v = instance().trim().replace(/\/+$/, "");
    return /^https?:\/\//.test(v) ? v : `https://${v}`;
  };

  const submit = (e: Event) => {
    e.preventDefault();
    try {
      localStorage.setItem(INSTANCE_KEY, base());
    } catch {
      // ignore
    }
    if (state.auth.phase === AuthPhase.MFA_REQUIRED) {
      core().send({ case: "mfa", value: { code: code().trim() } });
    } else if (mode() === "login") {
      core().send({ case: "login", value: { apiBase: base(), login: login().trim(), password: password() } });
    } else {
      core().send({ case: "register", value: { apiBase: base(), username: login().trim().toLowerCase(), password: password(), email: email().trim() } });
    }
  };

  return (
    <main class="auth">
      <form class="auth-card" onSubmit={submit} aria-busy={busy()}>
        <header>
          <img src={config.logo} width="56" height="56" alt="" />
          <h1>
            <Switch>
              <Match when={state.auth.phase === AuthPhase.MFA_REQUIRED}>Подтверждение входа</Match>
              <Match when={mode() === "login"}>С возвращением</Match>
              <Match when={true}>Создать аккаунт</Match>
            </Switch>
          </h1>
          <p>
            <Show when={state.auth.phase === AuthPhase.MFA_REQUIRED} fallback={`${config.name} — открытый мессенджер для сообществ`}>
              Введите код из приложения-аутентификатора или резервный код
            </Show>
          </p>
        </header>

        <Show
          when={state.auth.phase !== AuthPhase.MFA_REQUIRED}
          fallback={<TextField label="Код" value={code()} onInput={setCode} inputmode="numeric" autocomplete="one-time-code" autofocus required />}
        >
          <Show when={customInstance}>
            {/* There is no central server: a small «server» line, expanded only when needed. */}
            <Show
              when={editServer()}
              fallback={
                <div class="auth-server">
                  <span>Сервер</span>
                  <b title={base()}>{host()}</b>
                  <button type="button" onClick={() => setEditServer(true)}>Изменить</button>
                </div>
              }
            >
              <TextField label="Адрес сервера NeoShard" value={instance()} onInput={setInstance} autocomplete="url" placeholder="chat.example.ru" required autofocus />
              <p class="auth-hint">Центрального сервера нет: укажите свой или сервер друзей. Поднять свой можно одной командой — см. README проекта.</p>
            </Show>
          </Show>
          <TextField label={mode() === "login" ? "Имя пользователя или email" : "Имя пользователя"} value={login()} onInput={setLogin} autocomplete="username" required autofocus />
          <Show when={mode() === "register"}>
            <TextField label="Email (для восстановления доступа)" type="email" value={email()} onInput={setEmail} autocomplete="email" />
          </Show>
          <TextField label="Пароль" type="password" value={password()} onInput={setPassword} autocomplete={mode() === "login" ? "current-password" : "new-password"} required />
        </Show>

        <Show when={state.auth.error}>
          <div class="ns-error" role="alert">
            {state.auth.error}
          </div>
        </Show>

        <Button type="submit" loading={busy()}>
          {state.auth.phase === AuthPhase.MFA_REQUIRED ? "Подтвердить" : mode() === "login" ? "Войти" : "Зарегистрироваться"}
        </Button>

        <Show when={state.auth.phase !== AuthPhase.MFA_REQUIRED}>
          <div class="auth-switch">
            <Show when={mode() === "login"} fallback={<>Уже есть аккаунт? <button type="button" onClick={() => setMode("login")}>Войти</button></>}>
              Нет аккаунта? <button type="button" onClick={() => setMode("register")}>Зарегистрироваться</button>
            </Show>
          </div>
        </Show>
        <p class="auth-version">
          {config.name} {__APP_VERSION__}
          <Show when={FIXED_INSTANCE}> · {FIXED_INSTANCE.replace(/^https?:\/\//, "")}</Show>
        </p>
      </form>
    </main>
  );
}
