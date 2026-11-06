import {
  ArrowLeftRight,
  FolderSync,
  GitCompare,
  LayoutGrid,
  List,
  Monitor,
  Network,
  PencilLine,
  RefreshCw,
  Rows3,
  Search,
  Settings,
  Trash2,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { actions } from "../keymap/actions";
import { prettyChord } from "../keymap/keymap";
import { dragRegion } from "../lib/platform";
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
    <div className="toolbar" {...dragRegion}>
      <button className="tbtn" title="F2" onClick={() => actions.refresh.run()}>
        <Icon of={RefreshCw} />
      </button>
      <div className="seg">
        <button
          className={viewMode === "icon" ? "on" : ""}
          onClick={() => setViewMode(activeSide, "icon")}
        >
          <Icon of={LayoutGrid} size={14} /> {t("toolbar.icon")}
        </button>
        <button
          className={viewMode === "brief" ? "on" : ""}
          onClick={() => setViewMode(activeSide, "brief")}
        >
          <Icon of={Rows3} size={14} /> {t("toolbar.brief")}
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
      <div className="spacer" {...dragRegion} />
      <button
        className="tbtn"
        title={t("toolbar.desktop")}
        onClick={() => actions.openDesktop.run()}
      >
        <Icon of={Monitor} />
      </button>
      <button className="tbtn" title={t("toolbar.trash")} onClick={() => actions.openTrash.run()}>
        <Icon of={Trash2} />
      </button>
      <button
        className="tbtn"
        title={prettyChord("mod+,")}
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
      <button
        className="tbtn"
        title={prettyChord("ctrl+m")}
        onClick={() => actions.multiRename.run()}
      >
        <Icon of={PencilLine} /> {t("toolbar.rename")}
      </button>
      <button className="tbtn" title={prettyChord("mod+f")} onClick={() => actions.findFiles.run()}>
        <Icon of={Search} /> {t("toolbar.find")}
      </button>
      <button className="tbtn" title={prettyChord("mod+=")} onClick={() => actions.syncFolders.run()}>
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
