// Per-device UI preferences (localStorage). Nothing here is synced or sent anywhere.

import { createStore } from "solid-js/store";

export interface Prefs {
  micId: string;
  camId: string;
  speakerId: string;
  noiseSuppression: boolean;
  desktopNotifications: boolean;
  notificationSound: boolean;
  membersVisible: boolean;
  /** Collapsed channel categories by id. */
  collapsed: Record<string, boolean>;
}

const KEY = "neoshard.prefs";
const defaults: Prefs = {
  micId: "",
  camId: "",
  speakerId: "",
  noiseSuppression: true,
  desktopNotifications: false,
  notificationSound: true,
  membersVisible: true,
  collapsed: {},
};

function load(): Prefs {
  try {
    return { ...defaults, ...JSON.parse(localStorage.getItem(KEY) ?? "{}") };
  } catch {
    return { ...defaults };
  }
}

export const [prefs, setPrefs] = createStore<Prefs>(load());

export function setPref<K extends keyof Prefs>(key: K, value: Prefs[K]): void {
  setPrefs(key, value as never);
  try {
    localStorage.setItem(KEY, JSON.stringify(prefs));
  } catch {
    // private mode: the preference lasts for this tab only
  }
}

export function toggleCategory(id: string): void {
  setPref("collapsed", { ...prefs.collapsed, [id]: !prefs.collapsed[id] });
}
