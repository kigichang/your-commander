import { useTranslation } from "react-i18next";
import type { ActionId } from "../keymap/actions";
import { actions } from "../keymap/actions";
import { usePanels } from "../store/panels";
import { useRepair } from "../store/repair";

const KEYS: Array<{ chip: string; label: string; action: ActionId }> = [
  { chip: "F3", label: "fkeys.f3", action: "quickView" },
  { chip: "F4", label: "fkeys.f4", action: "edit" },
  { chip: "F5", label: "fkeys.f5", action: "copy" },
  { chip: "F6", label: "fkeys.f6", action: "move" },
  { chip: "F7", label: "fkeys.f7", action: "newFolder" },
  { chip: "F8", label: "fkeys.f8", action: "trash" },
  { chip: "⌘K", label: "fkeys.cmdK", action: "commandPalette" },
  { chip: "⌘Q", label: "fkeys.cmdQ", action: "quit" },
];

export function FKeyBar() {
  const { t } = useTranslation();
  const repairJob = useRepair((s) => s.job);
  // While the Quick View pane is visible its repair button already shows
  // the progress; the bar takes over once the pane is closed or hidden.
  const qvVisible = usePanels((s) => s.quickViewSide !== null && s.quickViewShown);
  return (
    <footer className="fkeybar">
      {KEYS.map((k) => (
        <button key={k.chip} className="fkey" onClick={() => actions[k.action].run()}>
          <kbd>{k.chip}</kbd>
          {t(k.label)}
        </button>
      ))}
      <div className="spacer" data-tauri-drag-region />
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
