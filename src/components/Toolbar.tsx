import { useTranslation } from "react-i18next";
import { actions } from "../keymap/actions";
import { useDialogs } from "../store/dialogs";
import { usePanels } from "../store/panels";
import { useSettings } from "../store/settings";

export function Toolbar() {
  const { t } = useTranslation();
  const activeSide = usePanels((s) => s.activeSide);
  const viewMode = usePanels((s) => s.panels[s.activeSide].viewMode);
  const setViewMode = usePanels((s) => s.setViewMode);
  const fkeysOn = useSettings((s) => s.fkeysWithoutFn);
  const fkeysAvailable = useSettings((s) => s.fkeysAvailable);

  return (
    <div className="toolbar" data-tauri-drag-region>
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
      <button
        className="tbtn"
        title="⌘,"
        onClick={() => actions.openSettings.run()}
      >
        <span className="glyph">⚙</span>
      </button>
      <button
        className={`tbtn${fkeysOn && fkeysAvailable ? " accent" : ""}`}
        disabled={!fkeysAvailable}
        title={t(fkeysAvailable ? "toolbar.fnHint" : "toolbar.fnUnavailable")}
        onClick={() => useSettings.getState().toggleFkeys()}
      >
        Fn
      </button>
      <button className="tbtn" onClick={() => actions.compareFiles.run()}>
        <span className="glyph">≡</span> {t("toolbar.compare")}
      </button>
      <button className="tbtn" title="⌃M" onClick={() => actions.multiRename.run()}>
        <span className="glyph">✎</span> {t("toolbar.rename")}
      </button>
      <button className="tbtn" title="⌘F" onClick={() => actions.findFiles.run()}>
        <span className="glyph">⌕</span> {t("toolbar.find")}
      </button>
      <button className="tbtn" title="⌘=" onClick={() => actions.syncFolders.run()}>
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
