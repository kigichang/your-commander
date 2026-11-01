import { isMac, MOD } from "../lib/platform";
import type { ActionId } from "./actions";

/** Default chords. Keys: `mods+key`, mods alphabetical (alt/ctrl/meta/shift),
 *  letter keys lower-cased, named keys as KeyboardEvent.key. "mod" stands for
 *  the platform's primary modifier (⌘ / Ctrl) and is expanded by buildKeymap.
 *  The hotkey-settings screen edits a user overlay merged over this table. */
export const defaultKeymap: Record<string, ActionId> = {
  Tab: "switchPanel",
  Enter: "enter",
  Backspace: "up",
  "mod+ArrowUp": "up",
  ArrowUp: "cursorUp",
  ArrowDown: "cursorDown",
  "shift+ArrowUp": "cursorUp",
  "shift+ArrowDown": "cursorDown",
  Home: "cursorHome",
  End: "cursorEnd",
  "shift+Home": "cursorHome",
  "shift+End": "cursorEnd",
  " ": "markToggle",
  "mod+a": "markAll",
  Escape: "clearMarks",

  F3: "quickView",
  F4: "edit",
  F5: "copy",
  F6: "move",
  F7: "newFolder",
  F8: "trash",
  "mod+Backspace": "trash",
  "mod+shift+Backspace": "deletePermanent",

  "shift+F6": "renameOne",
  "ctrl+m": "multiRename",
  "mod+=": "syncFolders",
  "mod+f": "findFiles",
  "mod+shift+r": "renameOne",
  "mod+shift+c": "copy",
  "mod+shift+m": "move",
  "mod+shift+n": "newFolder",
  "mod+shift+k": "compress",

  F2: "refresh",
  "mod+r": "refresh",
  "mod+t": "newTab",
  "mod+w": "closeTab",
  "mod+n": "newWindow",
  "mod+c": "clipCopy",
  "mod+x": "clipCut",
  "mod+v": "clipPaste",
  "mod+p": "clipPaste",
  "mod+i": "getInfo",
  "mod+u": "swap",
  "mod+shift+.": "toggleHidden",
  "mod+k": "commandPalette",
  "mod+,": "openSettings",
};

/// "Finder-like" preset: Space previews, Enter renames, ⌘O / ⌘↓ open.
export const finderDelta: Record<string, ActionId | ""> = {
  " ": "quickView",
  Enter: "renameOne",
  "mod+o": "enter",
  "mod+ArrowDown": "enter",
  F3: "markToggle",
};

/** Bindings only one platform needs. macOS gets ⌘Q from the system menu;
 *  Linux has no menu bar, so the app handles its quit chord itself. */
const platformDelta: Record<string, ActionId | ""> = isMac
  ? {}
  : { "ctrl+q": "quit" };

const MOD_ORDER = ["alt", "ctrl", "meta", "shift"];

/** "mod+shift+c" → "meta+shift+c" / "ctrl+shift+c", mods re-sorted into
 *  the canonical (alphabetical) order chordOf produces. */
export function expandChord(chord: string): string {
  if (!chord.includes("mod+")) return chord;
  const parts = chord.split("+");
  const key = parts.pop() ?? "";
  const mods = parts.map((m) => (m === "mod" ? MOD : m));
  mods.sort((a, b) => MOD_ORDER.indexOf(a) - MOD_ORDER.indexOf(b));
  return [...mods, key].join("+");
}

export type PresetName = "commander" | "finder";

export const PRESETS: Record<PresetName, Record<string, ActionId | "">> = {
  commander: {},
  finder: finderDelta,
};

/** Effective chord→action map: base defaults, preset delta, platform delta,
 *  user overlay. The first two may use the "mod" placeholder; the overlay is
 *  stored with concrete keys and applies after expansion, so existing user
 *  settings keep working unchanged. An empty-string action unbinds. */
export function buildKeymap(
  preset: PresetName,
  overrides: Record<string, string>,
): Record<string, ActionId> {
  const base: Record<string, ActionId | ""> = { ...defaultKeymap, ...PRESETS[preset] };
  const map: Record<string, ActionId | ""> = {};
  for (const [chord, action] of Object.entries(base)) {
    map[expandChord(chord)] = action;
  }
  for (const [chord, action] of Object.entries(platformDelta)) {
    map[chord] = action;
  }
  for (const [chord, action] of Object.entries(overrides)) {
    map[chord] = action as ActionId | "";
  }
  const out: Record<string, ActionId> = {};
  for (const [chord, action] of Object.entries(map)) {
    if (action !== "") out[chord] = action;
  }
  return out;
}

const MAC_GLYPHS: Array<[string, string]> = [
  ["meta+", "⌘"],
  ["alt+", "⌥"],
  ["ctrl+", "⌃"],
  ["shift+", "⇧"],
];

const LINUX_MOD_NAMES: Record<string, string> = {
  meta: "Super",
  ctrl: "Ctrl",
  alt: "Alt",
  shift: "Shift",
};

/** "meta+shift+c" → "⌘⇧C" on macOS; "ctrl+shift+c" → "Ctrl+Shift+C"
 *  elsewhere. Accepts "mod" chords too. */
export function prettyChord(chord: string): string {
  chord = expandChord(chord);
  if (isMac) {
    let out = chord;
    for (const [mod, glyph] of MAC_GLYPHS) out = out.replaceAll(mod, glyph);
    if (out === " " || out.endsWith("+ ")) out = out.slice(0, -1) + "Space";
    return out.length === 1 ? out.toUpperCase() : out;
  }
  const parts = chord === " " ? [" "] : chord.split("+");
  const key = parts.pop() ?? chord;
  const mods = parts.map((m) => LINUX_MOD_NAMES[m] ?? m);
  const keyName = key === " " ? "Space" : key.length === 1 ? key.toUpperCase() : key;
  return [...mods, keyName].join("+");
}

/** First chord bound to `action` in the effective map (shortest wins, so
 *  chips show "F3" rather than a longer alias), pretty-printed; null when
 *  unbound. */
export function chordForAction(
  map: Record<string, ActionId>,
  action: ActionId,
): string | null {
  const chords = Object.keys(map).filter((c) => map[c] === action);
  if (chords.length === 0) return null;
  chords.sort((a, b) => a.length - b.length);
  return chords[0];
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
