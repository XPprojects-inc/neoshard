import solid from "vite-plugin-solid";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [solid({ hot: false })],
  test: {
    environment: "jsdom",
    environmentOptions: { jsdom: { pretendToBeVisual: true } },
    server: { deps: { inline: [/solid-js/, /@solidjs/] } },
  },
  resolve: { conditions: ["development", "browser"] },
  ssr: { resolve: { conditions: ["development", "browser"], externalConditions: ["development", "browser"] } },
});
