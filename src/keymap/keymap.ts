import type { ActionId } from "./actions";

/** Default chords. Keys: `mods+key`, mods alphabetical (alt/ctrl/meta/shift),
 *  letter keys lower-cased, named keys as KeyboardEvent.key. The future
 *  hotkey-settings screen edits a user overlay merged over this table. */
export const defaultKeymap: Record<string, ActionId> = {
  Tab: "switchPanel",
  Enter: "enter",
  Backspace: "up",
  "meta+ArrowUp": "up",
  ArrowUp: "cursorUp",
  ArrowDown: "cursorDown",
  "shift+ArrowUp": "cursorUp",
  "shift+ArrowDown": "cursorDown",
  Home: "cursorHome",
  End: "cursorEnd",
  "shift+Home": "cursorHome",
  "shift+End": "cursorEnd",
  " ": "markToggle",
  "meta+a": "markAll",
  Escape: "clearMarks",

  F3: "quickView",
  F4: "edit",
  F5: "copy",
  F6: "move",
  F7: "newFolder",
  F8: "trash",
  "meta+Backspace": "trash",
  "meta+shift+Backspace": "deletePermanent",

  "shift+F6": "renameOne",
  "meta+shift+r": "renameOne",
  "meta+shift+c": "copy",
  "meta+shift+m": "move",
  "meta+shift+n": "newFolder",
  "meta+shift+k": "compress",

  F2: "refresh",
  "meta+r": "refresh",
  "meta+t": "newTab",
  "meta+w": "closeTab",
  "meta+u": "swap",
  "meta+shift+.": "toggleHidden",
  "meta+k": "commandPalette",
};

export function chordOf(e: KeyboardEvent): string {
  const mods: string[] = [];
  if (e.altKey) mods.push("alt");
  if (e.ctrlKey) mods.push("ctrl");
  if (e.metaKey) mods.push("meta");
  // Shift participates for arrows/Home/End via explicit entries; for letters
  // it is implied by the key itself staying lower-cased here.
  if (e.shiftKey) mods.push("shift");
  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
  return [...mods, key].join("+");
}
