// Dev-only preview of avatar frames (not part of the build).
import "@neoshard/ui/src/theme/tokens.css";
import "@neoshard/ui/src/theme/base.css";
import "../src/profile/profile.css";

import { Avatar } from "@neoshard/ui";
import { For, render } from "solid-js/web";

import { Decoration, DECORATIONS } from "../src/profile/cosmetics";

render(
  () => (
    <div style={{ display: "grid", "grid-template-columns": "repeat(7, 190px)", gap: "40px", padding: "50px", color: "#fff", font: "14px system-ui" }}>
      <For each={DECORATIONS}>
        {(d) => (
          <div style={{ display: "grid", "justify-items": "center", gap: "28px" }}>
            <Avatar name="Б" size={150} decoration={<Decoration id={d.id} />} seed="7" />
            <span>{d.label}</span>
          </div>
        )}
      </For>
    </div>
  ),
  document.getElementById("app")!,
);
