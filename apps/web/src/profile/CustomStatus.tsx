// Custom status: emoji + text + "clear after".

import { Button, Modal } from "@neoshard/ui";
import { createSignal, For, Show } from "solid-js";

import { setMyPresence } from "../actions";
import { state } from "../state";

export const QUICK_EMOJI = [
  "😀", "😊", "😎", "🤔", "😴", "🤒", "🥳", "😤", "🙃", "🫡",
  "🎮", "🎧", "🎬", "📚", "💻", "🛠", "🍕", "☕", "🍵", "🏃",
  "🏖", "✈️", "🚗", "🏠", "🌙", "☀️", "🌧", "❄️", "🔥", "✨",
  "❤️", "💙", "💚", "⭐", "🚀", "🎉", "📌", "🔕", "🐱", "🐶",
];

const CLEAR_AFTER: Array<[string, () => number]> = [
  ["Не сбрасывать", () => 0],
  ["Через 30 минут", () => Date.now() + 30 * 60_000],
  ["Через час", () => Date.now() + 60 * 60_000],
  ["Через 4 часа", () => Date.now() + 4 * 60 * 60_000],
  ["Сегодня", () => new Date(new Date().setHours(23, 59, 59, 999)).getTime()],
];

export function CustomStatusModal(props: { open: boolean; onClose: () => void }) {
  const [text, setText] = createSignal(state.myPresence.custom);
  const [emoji, setEmoji] = createSignal(state.myPresence.emoji);
  const [after, setAfter] = createSignal(0);
  const [picker, setPicker] = createSignal(false);
  const save = () => {
    setMyPresence({ custom: text().trim().slice(0, 128), emoji: emoji(), expires: CLEAR_AFTER[after()][1]() });
    props.onClose();
  };
  return (
    <Modal open={props.open} title="Свой статус" onClose={props.onClose}>
      <div class="ns-field">
        <label class="ns-label" for="cs-text">Что у вас нового?</label>
        <div class="status-input">
          <button class="emoji-btn" aria-label={emoji() ? `Эмодзи ${emoji()}, сменить` : "Выбрать эмодзи"} aria-expanded={picker()} onClick={() => setPicker(!picker())}>
            {emoji() || "🙂"}
          </button>
          <input id="cs-text" maxLength={128} value={text()} placeholder="Например: в отпуске до понедельника" onInput={(e) => setText(e.currentTarget.value)} onKeyDown={(e) => e.key === "Enter" && save()} />
          <Show when={text() || emoji()}>
            <button class="ns-icon-btn" aria-label="Очистить" onClick={() => (setText(""), setEmoji(""))}>
              ✕
            </button>
          </Show>
        </div>
      </div>
      <Show when={picker()}>
        <div class="emoji-grid" role="group" aria-label="Эмодзи">
          <For each={QUICK_EMOJI}>
            {(e) => (
              <button aria-pressed={emoji() === e} aria-label={e} onClick={() => (setEmoji(e), setPicker(false))}>
                {e}
              </button>
            )}
          </For>
        </div>
      </Show>
      <div class="ns-field">
        <label class="ns-label" for="cs-after">Сбросить</label>
        <select id="cs-after" class="ns-select" value={after()} onChange={(e) => setAfter(Number(e.currentTarget.value))}>
          <For each={CLEAR_AFTER}>{([label], i) => <option value={i()}>{label}</option>}</For>
        </select>
      </div>
      <div style={{ display: "flex", gap: "8px", "justify-content": "flex-end" }}>
        <Button variant="ghost" onClick={props.onClose}>Отмена</Button>
        <Button onClick={save}>Сохранить</Button>
      </div>
    </Modal>
  );
}
