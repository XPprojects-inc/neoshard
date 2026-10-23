import { NeoShardCore, AuthPhase, deviceE2ee, isTauri, tauriBackend, wipeLocalData } from "@neoshard/core";
import { applyTheme, builtinThemes, Button, parseTheme, Spinner, Toasts } from "@neoshard/ui";
import { createResource, lazy, Match, onCleanup, onMount, Show, Suspense, Switch } from "solid-js";

import config from "../neoshard.config.json";
import { attachCore, core, state } from "./state";
import { Auth } from "./views/Auth";

// The shell is a separate chunk: the login screen stays tiny.
const Shell = lazy(() => import("./views/Shell"));

function loadTheme() {
  let theme = builtinThemes[config.defaultTheme];
  try {
    const custom = parseTheme(JSON.parse(localStorage.getItem("neoshard.theme") ?? "null"));
    if (custom) theme = custom;
    const named = localStorage.getItem("neoshard.themeName");
    if (named && builtinThemes[named]) theme = builtinThemes[named];
  } catch {
    // keep default
  }
  if (theme) applyTheme(theme);
}

export function App() {
  loadTheme();
  const [ready, { refetch }] = createResource(async () => {
    if (!isTauri() && typeof WebAssembly !== "object") {
      throw new Error("WebAssembly отключён в этом браузере (в Edge его выключает режим «Повышенная безопасность»)");
    }
    // Desktop: native core over Tauri IPC; browser: the same core compiled to WASM.
    const load = isTauri() ? NeoShardCore.withBackend(await tauriBackend(), false) : NeoShardCore.loadWasm(`${config.name.toLowerCase()}-web`);
    // Never spin forever: a stuck load becomes a visible error with a way out.
    const c = await Promise.race([
      load,
      new Promise<never>((_, reject) => setTimeout(() => reject(new Error("ядро не загрузилось за 20 секунд")), 20_000)),
    ]);
    // The desktop core is built with MLS (apps/desktop/src-tauri/Cargo.toml).
    if (isTauri()) c.e2ee = true;
    attachCore(c);
    return true;
  });

  onMount(() => {
    const online = () => ready() && core().send({ case: "network", value: { online: true } });
    const offline = () => ready() && core().send({ case: "network", value: { online: false } });
    window.addEventListener("online", online);
    window.addEventListener("offline", offline);
    onCleanup(() => {
      window.removeEventListener("online", online);
      window.removeEventListener("offline", offline);
    });
  });

  return (
    <>
      <Show
        when={!ready.error && ready()}
        fallback={ready.error ? (
          <div class="splash" role="alert">
            <img src={config.logo} width="64" height="64" alt="" />
            <p style={{ "max-width": "420px", "text-align": "center" }}>
              Не удалось запустить {config.name}: {String(ready.error?.message ?? ready.error)}
            </p>
            <div style={{ display: "flex", gap: "8px" }}>
              <Button onClick={() => refetch()}>Повторить</Button>
              <Button
                variant="secondary"
                onClick={async () => {
                  // Clears this site's offline cache, session and E2EE keys of this device.
                  await wipeLocalData();
                  deviceE2ee.set(false);
                  location.reload();
                }}
              >
                Сбросить локальные данные
              </Button>
            </div>
          </div>
        ) : (
          <div class="splash">
            <img src={config.logo} width="64" height="64" alt="" />
            <Spinner label="Загружаем NeoShard" />
          </div>
        )}
      >
        <Switch>
          <Match when={state.auth.phase === AuthPhase.LOGGED_IN}>
            <Suspense fallback={<div class="splash"><Spinner /></div>}>
              <Shell />
            </Suspense>
          </Match>
          <Match when={true}>
            <Auth />
          </Match>
        </Switch>
      </Show>
      <Toasts />
    </>
  );
}
