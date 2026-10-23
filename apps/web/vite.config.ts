import solid from "vite-plugin-solid";
import { defineConfig, loadEnv } from "vite";

// One version for the web build and the apps: the Tauri config holds it.
import tauriConf from "../desktop/src-tauri/tauri.conf.json";

const appVersion = tauriConf.version;

/** NEOSHARD_WEB_BASE=app → "/app/" (a bare name: shells on Windows rewrite values starting with "/"). */
function webBase(mode: string): string {
  const v = (loadEnv(mode, ".", "NEOSHARD_WEB_").NEOSHARD_WEB_BASE ?? "").replace(/^\/+|\/+$/g, "");
  return v ? `/${v}/` : "/";
}

export default defineConfig(({ mode }) => ({
  // The web client lives under /app/ on a server that also hosts the landing site
  // (NEOSHARD_WEB_BASE=app); the Tauri apps load it from the root.
  base: webBase(mode),
  plugins: [solid()],
  define: { __APP_VERSION__: JSON.stringify(appVersion) },
  build: {
    target: "es2022",
    modulePreload: { polyfill: false },
    rollupOptions: {
      output: {
        // Voice/video (LiveKit) only loads when the user joins a voice channel.
        manualChunks: (id) => (id.includes("livekit-client") ? "voice" : undefined),
      },
    },
  },
  server: {
    host: "127.0.0.1",
    port: 5173,
    proxy: {
      "/api": "http://127.0.0.1:8080",
      "/.well-known": "http://127.0.0.1:8080",
      "/files": "http://127.0.0.1:8080",
      "/gateway": { target: "ws://127.0.0.1:8080", ws: true },
    },
  },
  worker: { format: "es" },
}));
