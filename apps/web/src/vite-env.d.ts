/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** Bake a server address into the build (desktop/Android apps, own deployments). */
  readonly VITE_NEOSHARD_INSTANCE?: string;
}

/** App version (from apps/desktop/src-tauri/tauri.conf.json). */
declare const __APP_VERSION__: string;
