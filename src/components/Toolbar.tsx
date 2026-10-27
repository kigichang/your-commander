import { useTranslation } from "react-i18next";
import { actions } from "../keymap/actions";
import { useDialogs } from "../store/dialogs";
import { useOps } from "../store/ops";
import { usePanels } from "../store/panels";

export function Toolbar() {
  const { t } = useTranslation();
  const activeSide = usePanels((s) => s.activeSide);
  const viewMode = usePanels((s) => s.panels[s.activeSide].viewMode);
  const setViewMode = usePanels((s) => s.setViewMode);
  const toast = useOps((s) => s.toast);
  const soon = () => toast(t("status.comingSoon"));

  return (
    <div className="toolbar">
      <button className="tbtn" title="F2" onClick={() => actions.refresh.run()}>
        <span className="glyph">⟳</span>
      </button>
      <div className="seg">
        <button
          className={viewMode === "brief" ? "on" : ""}
          onClick={() => setViewMode(activeSide, "brief")}
        >
          ⊞ {t("toolbar.brief")}
        </button>
        <button
          className={viewMode === "full" ? "on" : ""}
          onClick={() => setViewMode(activeSide, "full")}
        >
          ☰ {t("toolbar.full")}
        </button>
      </div>
      <button className="tbtn" onClick={() => actions.swap.run()}>
        <span className="glyph">⇄</span> {t("toolbar.swap")}
      </button>
      <div className="spacer" data-tauri-drag-region />
      <button className="tbtn" disabled onClick={soon}>
        <span className="glyph">≡</span> {t("toolbar.compare")}
      </button>
      <button className="tbtn" disabled onClick={soon}>
        <span className="glyph">✎</span> {t("toolbar.rename")}
      </button>
      <button className="tbtn" disabled onClick={soon}>
        <span className="glyph">⌕</span> {t("toolbar.find")}
      </button>
      <button className="tbtn" disabled onClick={soon}>
        <span className="glyph">⇅</span> {t("toolbar.sync")}
      </button>
      <button
        className="tbtn accent"
        onClick={() => useDialogs.getState().open({ type: "connect", protocol: "smb" })}
      >
        <span className="glyph">⇄</span> {t("toolbar.connect")}
      </button>
    </div>
  );
}
