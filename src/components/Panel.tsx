import clsx from "clsx";
import i18next from "i18next";
import { ChevronDown, Eject, HardDrive, House, Plus, Star, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import * as ipc from "../ipc/commands";
import { useOps } from "../store/ops";
import type { VolumeInfo } from "../ipc/types";
import { humanSize } from "../lib/format";
import { baseName, displayPath, expandHome, parentPath } from "../lib/path";
import { isMask } from "../lib/wildcard";
import { targetEntries, visibleEntries } from "../lib/selectors";
import type { Side, TabState } from "../store/panels";
import { usePanels } from "../store/panels";
import { useSettings } from "../store/settings";
import { FileTable } from "./FileTable";
import { Icon } from "./icons";
import { QuickView } from "./QuickView";

async function ejectVolume(volPath: string, side: Side, currentPath: string) {
  const { toast } = useOps.getState();
  try {
    // Leave the volume first when this panel is inside it.
    if (currentPath.startsWith(volPath)) {
      const home = useSettings.getState().home ?? "/";
      await usePanels.getState().navigate(side, home, { kind: "local" }, null);
    }
    await ipc.smbUnmount(volPath);
    toast(i18next.t("panel2.ejected"));
  } catch (e) {
    toast(String(e), "error");
  }
}

export function Panel({ side }: { side: Side }) {
  const panel = usePanels((s) => s.panels[side]);
  const isActive = usePanels((s) => s.activeSide === side);
  const quickViewSide = usePanels((s) => s.quickViewSide);
  const tab = panel.tabs[panel.active];
  if (!tab) return <section className="panel" />;

  const isQuickView = quickViewSide === side;
  return (
    <section
      className={clsx("panel", isActive && !isQuickView && "active")}
      onMouseDownCapture={() => {
        if (!isQuickView) usePanels.getState().setActiveSide(side);
      }}
    >
      <TabStrip side={side} />
      {isQuickView ? (
        <QuickView />
      ) : (
        <>
          <PanelBar side={side} tab={tab} />
          <FileTable side={side} tab={tab} viewMode={panel.viewMode} />
          <StatusLine tab={tab} />
        </>
      )}
    </section>
  );
}

function TabStrip({ side }: { side: Side }) {
  const { t } = useTranslation();
  const panel = usePanels((s) => s.panels[side]);
  const { setActiveTab, closeTab, openTab } = usePanels.getState();
  return (
    <div className="tabstrip">
      {panel.tabs.map((tab, i) => (
        <button
          key={tab.id}
          className={clsx("tab", i === panel.active && "on")}
          onClick={() => setActiveTab(side, i)}
          onAuxClick={(e) => {
            if (e.button === 1) closeTab(side, i);
          }}
        >
          <span className="label">
            {tab.remoteLabel ? `${tab.remoteLabel}:` : ""}
            {baseName(tab.path)}
          </span>
          {panel.tabs.length > 1 && (
            <span
              className="close"
              title={t("panel.closeTab")}
              onClick={(e) => {
                e.stopPropagation();
                closeTab(side, i);
              }}
            >
              <Icon of={X} size={12} />
            </span>
          )}
        </button>
      ))}
      <button className="tab-new" title={t("panel.newTab")} onClick={() => openTab(side)}>
        <Icon of={Plus} size={14} />
      </button>
    </div>
  );
}

function PanelBar({ side, tab }: { side: Side; tab: TabState }) {
  const { t } = useTranslation();
  const home = useSettings((s) => s.home);
  const config = useSettings((s) => s.config);
  const [menuOpen, setMenuOpen] = useState(false);
  const [vols, setVols] = useState<VolumeInfo[]>([]);
  const menuRef = useRef<HTMLDivElement>(null);
  const navigate = usePanels((s) => s.navigate);

  useEffect(() => {
    if (!menuOpen) return;
    void ipc.volumes().then(setVols);
    const close = (e: MouseEvent) => {
      if (!menuRef.current?.contains(e.target as Node)) setMenuOpen(false);
    };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [menuOpen]);

  const currentVol = vols.find((v) => tab.path.startsWith(v.path));
  const isFav = config?.favorites.some((f) => f.path === tab.path) ?? false;

  const toggleFavorite = () => {
    if (!config) return;
    const favorites = isFav
      ? config.favorites.filter((f) => f.path !== tab.path)
      : [...config.favorites, { name: baseName(tab.path), path: tab.path }];
    useSettings.getState().save({ favorites });
  };

  return (
    <div className="panel-bar">
      <div className="drive" ref={menuRef}>
        <button onClick={() => setMenuOpen((v) => !v)}>
          <span className="name">
            <Icon of={HardDrive} size={14} />{" "}
            {tab.remoteLabel ?? currentVol?.name ?? "Macintosh HD"}{" "}
            <Icon of={ChevronDown} size={12} />
          </span>
          {currentVol && !tab.remoteLabel && currentVol.total > 0 && (
            <span className="free">
              {t("panel.freeOf", {
                free: humanSize(currentVol.free),
                total: humanSize(currentVol.total),
              })}
            </span>
          )}
        </button>
        {menuOpen && (
          <div className="menu">
            {tab.location.kind === "sftp" && (
              <>
                <button
                  className="item"
                  onClick={() => {
                    setMenuOpen(false);
                    if (tab.location.kind === "sftp")
                      usePanels.getState().disconnectSftp(tab.location.connId);
                  }}
                >
                  <span>
                    <Icon of={Eject} size={14} /> {t("panel2.disconnect")}
                  </span>
                  <span className="sub">{tab.remoteLabel}</span>
                </button>
                <div className="sep" />
              </>
            )}
            <button
              className="item"
              onClick={() => {
                setMenuOpen(false);
                if (home) void navigate(side, home, { kind: "local" }, null);
              }}
            >
              <span>
                <Icon of={House} size={14} /> {t("panel.home")}
              </span>
            </button>
            {config && config.favorites.length > 0 && (
              <>
                <div className="sep" />
                <div className="headline">{t("panel.favorites")}</div>
                {config.favorites.map((f) => (
                  <button
                    key={f.path}
                    className="item"
                    onClick={() => {
                      setMenuOpen(false);
                      void navigate(side, f.path, { kind: "local" }, null);
                    }}
                  >
                    <span>
                      <Icon of={Star} size={14} /> {f.name}
                    </span>
                    <span className="sub">{displayPath(f.path, home)}</span>
                  </button>
                ))}
              </>
            )}
            <div className="sep" />
            <div className="headline">{t("panel.drives")}</div>
            {vols.map((v) => (
              <div key={v.path} className="item-row">
                <button
                  className="item"
                  onClick={() => {
                    setMenuOpen(false);
                    void navigate(side, v.path, { kind: "local" }, null);
                  }}
                >
                  <span>
                    <Icon of={HardDrive} size={14} /> {v.name}
                  </span>
                  {v.total > 0 && (
                    <span className="sub">
                      {t("panel.freeOf", {
                        free: humanSize(v.free),
                        total: humanSize(v.total),
                      })}
                    </span>
                  )}
                </button>
                <button
                  className="eject"
                  title={t("panel2.eject")}
                  onClick={(e) => {
                    e.stopPropagation();
                    setMenuOpen(false);
                    void ejectVolume(v.path, side, tab.path);
                  }}
                >
                  <Icon of={Eject} size={14} />
                </button>
              </div>
            ))}
          </div>
        )}
      </div>
      <PathBar side={side} tab={tab} />
      <button
        className={clsx("star", isFav && "on")}
        title={isFav ? t("panel.removeFavorite") : t("panel.addFavorite")}
        onClick={toggleFavorite}
      >
        <Icon of={Star} fill={isFav} />
      </button>
    </div>
  );
}

function PathBar({ side, tab }: { side: Side; tab: TabState }) {
  const home = useSettings((s) => s.home);
  const local = tab.location.kind === "local";
  const shown =
    (tab.remoteLabel ? `${tab.remoteLabel}:` : "") +
    (local ? displayPath(tab.path, home) : tab.path) +
    "/" +
    (tab.filter || "*.*");
  const [value, setValue] = useState(shown);
  const [editing, setEditing] = useState(false);

  useEffect(() => {
    if (!editing) setValue(shown);
  }, [shown, editing]);

  const commit = () => {
    setEditing(false);
    let input = value.trim();
    if (!input) return;
    if (tab.remoteLabel && input.startsWith(`${tab.remoteLabel}:`)) {
      input = input.slice(tab.remoteLabel.length + 1);
    }
    input = expandHome(input, home);
    const last = baseName(input);
    const { navigate, setFilter } = usePanels.getState();
    if (isMask(last)) {
      // A bare mask filters the current folder; a path+mask navigates first.
      const dir = input.includes("/") ? (parentPath(input) ?? tab.path) : tab.path;
      if (dir !== tab.path) {
        void navigate(side, dir).then(() => setFilter(side, last));
      } else {
        setFilter(side, last === "*.*" ? "" : last);
      }
    } else {
      const fallback = tab.location.kind === "archive" ? "" : "/";
      void navigate(side, input.replace(/\/+$/, "") || fallback);
    }
  };

  return (
    <div className="pathbar">
      <input
        value={value}
        spellCheck={false}
        onFocus={(e) => {
          setEditing(true);
          e.currentTarget.select();
        }}
        onChange={(e) => setValue(e.target.value)}
        onBlur={() => {
          setEditing(false);
          setValue(shown);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            commit();
            e.currentTarget.blur();
          } else if (e.key === "Escape") {
            setValue(shown);
            e.currentTarget.blur();
          }
        }}
      />
    </div>
  );
}

function StatusLine({ tab }: { tab: TabState }) {
  const { t } = useTranslation();
  const showHidden = useSettings((s) => s.showHidden);
  const visible = visibleEntries(tab, showHidden);
  const marked = targetEntries(tab, visible);
  if (tab.marked.size === 0) {
    return (
      <div className="statusline">{t("status.items", { count: visible.length })}</div>
    );
  }
  const markedFiles = marked.filter((e) => !e.isDir);
  const size = markedFiles.reduce((a, e) => a + e.size, 0);
  const totalSize = visible.filter((e) => !e.isDir).reduce((a, e) => a + e.size, 0);
  return (
    <div className="statusline">
      {size > 0
        ? t("status.markedSize", {
            marked: tab.marked.size,
            total: visible.length,
            size: humanSize(size),
            totalSize: humanSize(totalSize),
          })
        : t("status.marked", { marked: tab.marked.size, total: visible.length })}
    </div>
  );
}
