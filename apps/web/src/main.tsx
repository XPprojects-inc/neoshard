import "@neoshard/ui/src/theme/tokens.css";
import "@neoshard/ui/src/theme/base.css";
import "./app.css";
import "./profile/profile.css";
import "./settings.css";
import "./polish.css";
import "./m3.css";

import { render } from "solid-js/web";

import { App } from "./App";

render(() => <App />, document.getElementById("app")!);

