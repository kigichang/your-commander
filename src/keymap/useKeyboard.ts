import { useEffect } from "react";
import { useDialogs } from "../store/dialogs";
import { actions } from "./actions";
import { chordOf, defaultKeymap } from "./keymap";

/** One global keydown listener dispatching through the keymap. Dialogs and
 *  text inputs take precedence over panel shortcuts. */
export function useKeyboard() {
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      // A dialog owns the keyboard while open (its own handlers apply).
      if (useDialogs.getState().dialog) return;

      const target = e.target as HTMLElement | null;
      if (target?.closest("input, textarea, [contenteditable]")) return;

      const chord = chordOf(e);
      let actionId = defaultKeymap[chord];
      // Shift+<chord> falls back to the bare chord with mark semantics
      // only where the keymap registered it explicitly, so no extra logic.
      if (!actionId) return;
      e.preventDefault();
      actions[actionId].run({ mark: e.shiftKey });
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
}
