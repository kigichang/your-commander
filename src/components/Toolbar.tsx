import {
  ArrowLeftRight,
  FolderSync,
  GitCompare,
  LayoutGrid,
  List,
  Network,
  PencilLine,
  RefreshCw,
  Search,
  Settings,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { actions } from "../keymap/actions";
import { useDialogs } from "../store/dialogs";
import { usePanels } from "../store/panels";
import { useSettings } from "../store/settings";
import { Icon } from "./icons";

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
        <Icon of={RefreshCw} />
      </button>
      <div className="seg">
        <button
          className={viewMode === "brief" ? "on" : ""}
          onClick={() => setViewMode(activeSide, "brief")}
        >
          <Icon of={LayoutGrid} size={14} /> {t("toolbar.brief")}
        </button>
        <button
          className={viewMode === "full" ? "on" : ""}
          onClick={() => setViewMode(activeSide, "full")}
        >
          <Icon of={List} size={14} /> {t("toolbar.full")}
        </button>
      </div>
      <button className="tbtn" onClick={() => actions.swap.run()}>
        <Icon of={ArrowLeftRight} /> {t("toolbar.swap")}
      </button>
      <div className="spacer" data-tauri-drag-region />
      <button
        className="tbtn"
        title="⌘,"
        onClick={() => actions.openSettings.run()}
      >
        <Icon of={Settings} />
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
        <Icon of={GitCompare} /> {t("toolbar.compare")}
      </button>
      <button className="tbtn" title="⌃M" onClick={() => actions.multiRename.run()}>
        <Icon of={PencilLine} /> {t("toolbar.rename")}
      </button>
      <button className="tbtn" title="⌘F" onClick={() => actions.findFiles.run()}>
        <Icon of={Search} /> {t("toolbar.find")}
      </button>
      <button className="tbtn" title="⌘=" onClick={() => actions.syncFolders.run()}>
        <Icon of={FolderSync} /> {t("toolbar.sync")}
      </button>
      <button
        className="tbtn accent"
        onClick={() => useDialogs.getState().open({ type: "connect", protocol: "smb" })}
      >
        <Icon of={Network} /> {t("toolbar.connect")}
      </button>
    </div>
  );
}
