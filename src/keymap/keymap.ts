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
  "ctrl+m": "multiRename",
  "meta+=": "syncFolders",
  "meta+f": "findFiles",
  "meta+shift+r": "renameOne",
  "meta+shift+c": "copy",
  "meta+shift+m": "move",
  "meta+shift+n": "newFolder",
  "meta+shift+k": "compress",

  F2: "refresh",
  "meta+r": "refresh",
  "meta+t": "newTab",
  "meta+w": "closeTab",
  "meta+n": "newWindow",
  "meta+c": "clipCopy",
  "meta+x": "clipCut",
  "meta+v": "clipPaste",
  "meta+p": "clipPaste",
  "meta+i": "getInfo",
  "meta+u": "swap",
  "meta+shift+.": "toggleHidden",
  "meta+k": "commandPalette",
  "meta+,": "openSettings",
};

/// "Finder-like" preset: Space previews, Enter renames, ⌘O / ⌘↓ open.
export const finderDelta: Record<string, ActionId | ""> = {
  " ": "quickView",
  Enter: "renameOne",
  "meta+o": "enter",
  "meta+ArrowDown": "enter",
  F3: "markToggle",
};

export type PresetName = "commander" | "finder";

export const PRESETS: Record<PresetName, Record<string, ActionId | "">> = {
  commander: {},
  finder: finderDelta,
};

/** Effective chord→action map: base defaults, preset delta, user overlay.
 *  An empty-string action unbinds the chord. */
export function buildKeymap(
  preset: PresetName,
  overrides: Record<string, string>,
): Record<string, ActionId> {
  const map: Record<string, ActionId | ""> = { ...defaultKeymap, ...PRESETS[preset] };
  for (const [chord, action] of Object.entries(overrides)) {
    map[chord] = action as ActionId | "";
  }
  const out: Record<string, ActionId> = {};
  for (const [chord, action] of Object.entries(map)) {
    if (action !== "") out[chord] = action;
  }
  return out;
}

const MOD_GLYPHS: Array<[string, string]> = [
  ["meta+", "⌘"],
  ["alt+", "⌥"],
  ["ctrl+", "⌃"],
  ["shift+", "⇧"],
];

/** "meta+shift+c" → "⌘⇧C" for display. */
export function prettyChord(chord: string): string {
  let out = chord;
  for (const [mod, glyph] of MOD_GLYPHS) out = out.replaceAll(mod, glyph);
  if (out === " " || out.endsWith("+ ")) out = out.slice(0, -1) + "Space";
  return out.length === 1 ? out.toUpperCase() : out;
}

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
