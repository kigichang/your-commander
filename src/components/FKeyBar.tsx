import { useTranslation } from "react-i18next";
import type { ActionId } from "../keymap/actions";
import { actions } from "../keymap/actions";
import { chordForAction, prettyChord } from "../keymap/keymap";
import { dragRegion } from "../lib/platform";
import { usePanels } from "../store/panels";
import { useKeymap } from "../store/keymapStore";
import { useRepair } from "../store/repair";

/** `chip` is the fallback when the action is unbound in the current keymap. */
const KEYS: Array<{ chip: string; label: string; action: ActionId }> = [
  { chip: "F3", label: "fkeys.f3", action: "quickView" },
  { chip: "F4", label: "fkeys.f4", action: "edit" },
  { chip: "F5", label: "fkeys.f5", action: "copy" },
  { chip: "F6", label: "fkeys.f6", action: "move" },
  { chip: "F7", label: "fkeys.f7", action: "newFolder" },
  { chip: "F8", label: "fkeys.f8", action: "trash" },
  { chip: "mod+k", label: "fkeys.cmdK", action: "commandPalette" },
  { chip: "mod+q", label: "fkeys.cmdQ", action: "quit" },
];

export function FKeyBar() {
  const { t } = useTranslation();
  const keymap = useKeymap((s) => s.map);
  const repairJob = useRepair((s) => s.job);
  // While the Quick View pane is visible its repair button already shows
  // the progress; the bar takes over once the pane is closed or hidden.
  const qvVisible = usePanels((s) => s.quickViewSide !== null && s.quickViewShown);
  return (
    <footer className="fkeybar">
      {KEYS.map((k) => (
        <button key={k.action} className="fkey" onClick={() => actions[k.action].run()}>
          <kbd>{prettyChord(chordForAction(keymap, k.action) ?? k.chip)}</kbd>
          {t(k.label)}
        </button>
      ))}
      <div className="spacer" {...dragRegion} />
      {repairJob && !qvVisible && (
        <span className="fkey-repair" title={repairJob.name}>
          <span className="name">
            {t("quickview.repairing")} {repairJob.name}
          </span>
          {repairJob.fraction !== null && (
            <>
              <span className="mini-bar">
                <i style={{ width: `${Math.round(repairJob.fraction * 100)}%` }} />
              </span>
              <span className="pct">{Math.round(repairJob.fraction * 100)}%</span>
            </>
          )}
        </span>
      )}
    </footer>
  );
}
