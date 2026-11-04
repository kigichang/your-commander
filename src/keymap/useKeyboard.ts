import { useEffect } from "react";
import { useDialogs } from "../store/dialogs";
import { useKeymap } from "../store/keymapStore";
import { actions } from "./actions";
import { chordOf, expandChord } from "./keymap";

const COPY_CHORD = expandChord("mod+c");

/** True when text is selected inside the Quick View body. */
function hasQuickViewSelection(): boolean {
  const sel = window.getSelection();
  if (!sel || sel.isCollapsed) return false;
  const node = sel.anchorNode;
  const el = node instanceof Element ? node : node?.parentElement;
  return !!el?.closest(".qv-body");
}

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
      // Selected Quick View text (markdown, plain text) takes the system
      // copy instead of the file clipboard.
      if (chord === COPY_CHORD && hasQuickViewSelection()) return;
      const actionId = useKeymap.getState().map[chord];
      // Shift+<chord> falls back to the bare chord with mark semantics
      // only where the keymap registered it explicitly, so no extra logic.
      if (!actionId) return;
      const action = actions[actionId];
      if (action.enabled && !action.enabled()) return;
      e.preventDefault();
      action.run({ mark: e.shiftKey });
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
}
