import clsx from "clsx";
import type { LucideIcon } from "lucide-react";
import {
  ArrowLeftRight,
  Check,
  ChevronLeft,
  ChevronRight,
  ChevronUp,
  Columns3,
  Ellipsis,
  LayoutGrid,
  List,
  Monitor,
  Network,
  Search,
  Trash2,
  X,
} from "lucide-react";
import { Fragment, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { ActionId } from "../keymap/actions";
import { actions } from "../keymap/actions";
import { chordForAction, prettyChord } from "../keymap/keymap";
import { parentPath } from "../lib/path";
import { dragRegion } from "../lib/platform";
import { isMask } from "../lib/wildcard";
import { useDialogs } from "../store/dialogs";
import { useKeymap } from "../store/keymapStore";
import type { ViewMode } from "../store/panels";
import { usePanels } from "../store/panels";
import { selectFkeysWithoutFn, useSettings } from "../store/settings";
import { Icon } from "./icons";

const VIEWS: Array<{ mode: ViewMode; icon: LucideIcon }> = [
  { mode: "icon", icon: LayoutGrid },
  { mode: "brief", icon: Columns3 },
  { mode: "full", icon: List },
];

/** Search text → path-bar mask: plain text matches anywhere in the name,
 *  text with * or ? is taken as a mask as-is. */
const toMask = (text: string) => {
  const t = text.trim();
  return !t ? "" : isMask(t) ? t : `*${t}*`;
};

export function Toolbar() {
  const { t } = useTranslation();
  const keymap = useKeymap((s) => s.map);
  const activeSide = usePanels((s) => s.activeSide);
  const viewMode = usePanels((s) => s.panels[s.activeSide].viewMode);
  const tab = usePanels((s) => s.panels[s.activeSide].tabs[s.panels[s.activeSide].active]);
  const setViewMode = usePanels((s) => s.setViewMode);

  const tip = (label: string, id: ActionId) => {
    const chord = chordForAction(keymap, id);
    return chord ? `${label} (${prettyChord(chord)})` : label;
  };

  const canBack = (tab?.back.length ?? 0) > 0;
  const canForward = (tab?.fwd.length ?? 0) > 0;
  const canUp =
    !!tab && (tab.location.kind === "archive" || parentPath(tab.path) !== null);

  return (
    <div className="toolbar" {...dragRegion}>
      <div className="cap">
        <button
          title={tip(t("toolbar.desktop"), "openDesktop")}
          onClick={() => actions.openDesktop.run()}
        >
          <Icon of={Monitor} size={17} />
        </button>
        <button
          title={tip(t("toolbar.trash"), "openTrash")}
          onClick={() => actions.openTrash.run()}
        >
          <Icon of={Trash2} size={17} />
        </button>
      </div>

      <div className="cap">
        <button
          disabled={!canBack}
          title={tip(t("toolbar.back"), "goBack")}
          onClick={() => actions.goBack.run()}
        >
          <Icon of={ChevronLeft} size={18} />
        </button>
        <button
          disabled={!canForward}
          title={tip(t("toolbar.forward"), "goForward")}
          onClick={() => actions.goForward.run()}
        >
          <Icon of={ChevronRight} size={18} />
        </button>
        <button disabled={!canUp} title={tip(t("toolbar.up"), "up")} onClick={() => actions.up.run()}>
          <Icon of={ChevronUp} size={18} />
        </button>
      </div>

      <div className="cap cap-seg" role="radiogroup" aria-label={t("toolbar.viewMode")}>
        {VIEWS.map(({ mode, icon }) => (
          <button
            key={mode}
            role="radio"
            aria-checked={viewMode === mode}
            className={clsx(viewMode === mode && "on")}
            title={t(`toolbar.${mode}`)}
            onClick={() => setViewMode(activeSide, mode)}
          >
            <Icon of={icon} size={17} />
          </button>
        ))}
      </div>

      <div className="cap">
        <button title={tip(t("toolbar.swap"), "swap")} onClick={() => actions.swap.run()}>
          <Icon of={ArrowLeftRight} size={17} />
        </button>
      </div>

      <div className="cap">
        <button
          title={t("toolbar.network")}
          onClick={() => useDialogs.getState().open({ type: "connect", protocol: "smb" })}
        >
          <Icon of={Network} size={17} />
        </button>
        <MoreMenu />
      </div>

      <div className="spacer" {...dragRegion} />

      <SearchField />
    </div>
  );
}

/** An action (run and chord from the keymap) or a plain toggle. */
type MenuItem = {
  label: string;
  id?: ActionId;
  run?: () => void;
  checked?: boolean;
  disabled?: boolean;
  title?: string;
};

function MoreMenu() {
  const { t } = useTranslation();
  const keymap = useKeymap((s) => s.map);
  const fkeysOn = useSettings(selectFkeysWithoutFn);
  const fkeysAvailable = useSettings((s) => s.fkeysAvailable);
  const compact = usePanels((s) => s.compact);
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const away = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    // Capture phase, so Esc closes the menu instead of reaching the keymap.
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        setOpen(false);
      }
    };
    window.addEventListener("mousedown", away);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("mousedown", away);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [open]);

  const sections: MenuItem[][] = [
    [
      { id: "refresh", label: t("toolbar.refresh") },
      { id: "compareFiles", label: t("action.compareFiles") },
      { id: "multiRename", label: t("action.multiRename") },
      { id: "findFiles", label: t("action.findFiles") },
      { id: "syncFolders", label: t("action.syncFolders") },
    ],
    [
      { id: "toggleCompact", label: t("toolbar.compact"), checked: compact },
      {
        label: t("toolbar.fkeys"),
        checked: fkeysOn && fkeysAvailable,
        disabled: !fkeysAvailable,
        title: t(fkeysAvailable ? "toolbar.fnHint" : "toolbar.fnUnavailable"),
        run: () => useSettings.getState().toggleFkeys(),
      },
      { id: "openSettings", label: t("toolbar.settings") },
    ],
  ];

  return (
    <div className="tb-more" ref={ref}>
      <button
        className={clsx(open && "on")}
        title={t("toolbar.more")}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        <Icon of={Ellipsis} size={18} />
      </button>
      {open && (
        <div className="menu tb-menu" role="menu">
          {sections.map((section, i) => (
            <Fragment key={i}>
              {i > 0 && <div className="sep" />}
              {section.map((item) => {
                const chord = item.id ? chordForAction(keymap, item.id) : null;
                return (
                  <button
                    key={item.label}
                    className="item"
                    role={item.checked === undefined ? "menuitem" : "menuitemcheckbox"}
                    aria-checked={item.checked}
                    disabled={item.disabled}
                    title={item.title}
                    onClick={() => {
                      setOpen(false);
                      if (item.run) item.run();
                      else if (item.id) actions[item.id].run();
                    }}
                  >
                    <span className="tb-label">
                      <span className="tb-check">
                        {item.checked && <Icon of={Check} size={13} />}
                      </span>
                      {item.label}
                    </span>
                    {chord && <span className="tb-chord">{prettyChord(chord)}</span>}
                  </button>
                );
              })}
            </Fragment>
          ))}
        </div>
      )}
    </div>
  );
}

function SearchField() {
  const { t } = useTranslation();
  const side = usePanels((s) => s.activeSide);
  const tabId = usePanels((s) => s.panels[s.activeSide].tabs[s.panels[s.activeSide].active]?.id);
  const filter = usePanels(
    (s) => s.panels[s.activeSide].tabs[s.panels[s.activeSide].active]?.filter ?? "",
  );
  const [text, setText] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  // Follow the tab's filter when it changes elsewhere (path bar, navigation,
  // another tab or side), keeping the user's own text while it maps to it.
  useEffect(() => {
    setText((cur) => {
      if (toMask(cur) === filter) return cur;
      const m = /^\*([^*?]+)\*$/.exec(filter);
      return m ? m[1] : filter;
    });
  }, [filter, side, tabId]);

  const apply = (next: string) => {
    setText(next);
    usePanels.getState().setFilter(side, toMask(next));
  };

  return (
    <label className="cap search-cap">
      <Icon of={Search} size={15} />
      <input
        ref={inputRef}
        value={text}
        placeholder={t("toolbar.search")}
        title={t("toolbar.searchHint")}
        spellCheck={false}
        autoCorrect="off"
        autoCapitalize="off"
        onChange={(e) => apply(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            apply("");
            inputRef.current?.blur();
          } else if (e.key === "Enter") {
            e.preventDefault();
            const s = usePanels.getState();
            const tab = s.activeTab(s.activeSide);
            inputRef.current?.blur();
            useDialogs.getState().open({
              type: "find",
              side: s.activeSide,
              location: tab.location,
              dir: tab.path,
              mask: toMask(text) || undefined,
            });
          } else if (e.key === "ArrowDown") {
            // Hand the keyboard back to the list.
            e.preventDefault();
            inputRef.current?.blur();
          }
        }}
      />
      {text && (
        <button
          className="search-clear"
          title={t("toolbar.searchHint")}
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => apply("")}
        >
          <Icon of={X} size={12} />
        </button>
      )}
    </label>
  );
}
