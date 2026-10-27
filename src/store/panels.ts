import { create } from "zustand";
import * as ipc from "../ipc/commands";
import type { FsEntry, Location } from "../ipc/types";
import { parentPath } from "../lib/path";
import { visibleEntries } from "../lib/selectors";
import { useSettings } from "./settings";

export type Side = 0 | 1;
export type SortKey = "name" | "ext" | "size" | "modified";
export type ViewMode = "full" | "brief";

export interface TabState {
  id: number;
  location: Location;
  /** Display label for an SFTP location ("user@host"), null for local. */
  remoteLabel: string | null;
  path: string;
  entries: FsEntry[];
  loading: boolean;
  error: string | null;
  /** Cursor index into the VISIBLE (filtered + sorted) list. */
  cursor: number;
  /** Range-selection anchor for Shift-clicks. */
  anchor: number;
  /** Marked entry names (operations act on these, else the cursor item). */
  marked: Set<string>;
  sortKey: SortKey;
  sortDir: 1 | -1;
  filter: string;
}

export type ColKey = "ext" | "size" | "date";
export type ColWidths = Record<ColKey, number>;

export const DEFAULT_COLS: ColWidths = { ext: 64, size: 84, date: 118 };
export const COL_MIN = 36;
export const COL_MAX = 280;

export interface PanelState {
  tabs: TabState[];
  active: number;
  viewMode: ViewMode;
  colWidths: ColWidths;
}

let nextTabId = 1;

function newTab(location: Location, path: string, remoteLabel: string | null = null): TabState {
  return {
    id: nextTabId++,
    location,
    remoteLabel,
    path,
    entries: [],
    loading: true,
    error: null,
    cursor: 0,
    anchor: 0,
    marked: new Set(),
    sortKey: "name",
    sortDir: 1,
    filter: "",
  };
}

interface PanelsStore {
  panels: [PanelState, PanelState];
  activeSide: Side;
  /** Side whose pane is replaced by Quick View (follows the other cursor). */
  quickViewSide: Side | null;

  init: () => Promise<void>;
  activeTab: (side: Side) => TabState;
  visible: (side: Side) => FsEntry[];
  otherSide: (side?: Side) => Side;

  setActiveSide: (side: Side) => void;
  navigate: (side: Side, path: string, location?: Location, remoteLabel?: string | null) => Promise<void>;
  refresh: (side: Side, opts?: { silent?: boolean }) => Promise<void>;
  up: (side: Side) => void;
  setCursor: (side: Side, index: number, opts?: { anchor?: boolean }) => void;
  moveCursor: (side: Side, delta: number, opts?: { mark?: boolean; edge?: boolean }) => void;
  toggleMark: (side: Side, index?: number, advance?: boolean) => void;
  markRange: (side: Side, index: number) => void;
  markAll: (side: Side) => void;
  clearMarks: (side: Side) => void;
  setSort: (side: Side, key: SortKey) => void;
  setFilter: (side: Side, filter: string) => void;
  openTab: (side: Side, path?: string, location?: Location, remoteLabel?: string | null) => void;
  closeTab: (side: Side, index: number) => void;
  setActiveTab: (side: Side, index: number) => void;
  swapPanels: () => void;
  setViewMode: (side: Side, mode: ViewMode) => void;
  /** Live while dragging a header divider; call persist() on release. */
  setColWidth: (side: Side, col: ColKey, width: number) => void;
  resetColWidth: (side: Side, col: ColKey) => void;
  persist: () => void;
  toggleQuickView: () => void;
  /** Close an SFTP connection and send every tab that used it back home. */
  disconnectSftp: (connId: number) => void;
}

function updateTab(
  panels: [PanelState, PanelState],
  side: Side,
  tabId: number,
  patch: Partial<TabState> | ((tab: TabState) => Partial<TabState>),
): [PanelState, PanelState] {
  const next = [...panels] as [PanelState, PanelState];
  const panel = next[side];
  next[side] = {
    ...panel,
    tabs: panel.tabs.map((t) =>
      t.id === tabId ? { ...t, ...(typeof patch === "function" ? patch(t) : patch) } : t,
    ),
  };
  return next;
}

async function loadEntries(location: Location, path: string): Promise<FsEntry[]> {
  return ipc.listDir(location, path);
}

/** Write both sides' local tabs, active tab, view mode, and active sort into
 *  the config. Called after any change worth restoring next launch. */
function persistPanels() {
  const s = usePanels.getState();
  const settings = useSettings.getState();
  if (!settings.config) return;
  const sideOf = (side: Side) => {
    const panel = s.panels[side];
    const localPaths = panel.tabs
      .filter((t) => t.location.kind === "local")
      .map((t) => t.path);
    const activeTab = panel.tabs[panel.active];
    const activeLocal =
      activeTab.location.kind === "local"
        ? Math.max(0, localPaths.indexOf(activeTab.path))
        : 0;
    return {
      tabs: localPaths,
      active: activeLocal,
      view: panel.viewMode,
      sort: `${activeTab.sortKey}:${activeTab.sortDir === 1 ? "asc" : "desc"}`,
      cols: [panel.colWidths.ext, panel.colWidths.size, panel.colWidths.date],
      path: activeTab.location.kind === "local" ? activeTab.path : null,
    };
  };
  const left = sideOf(0);
  const right = sideOf(1);
  settings.save({
    left_tabs: left.tabs,
    right_tabs: right.tabs,
    left_active: left.active,
    right_active: right.active,
    left_view: left.view,
    right_view: right.view,
    left_sort: left.sort,
    right_sort: right.sort,
    left_cols: left.cols,
    right_cols: right.cols,
    ...(left.path ? { left_path: left.path } : {}),
    ...(right.path ? { right_path: right.path } : {}),
  });
}

export const usePanels = create<PanelsStore>((set, get) => ({
  panels: [
    { tabs: [], active: 0, viewMode: "full", colWidths: { ...DEFAULT_COLS } },
    { tabs: [], active: 0, viewMode: "full", colWidths: { ...DEFAULT_COLS } },
  ],
  activeSide: 0,
  quickViewSide: null,

  init: async () => {
    const home = await ipc.homeDir();
    const config = useSettings.getState().config;
    const restore = (
      paths: string[] | undefined,
      fallback: string | null | undefined,
      active: number | undefined,
      view: string | null | undefined,
      sort: string | null | undefined,
      cols: number[] | undefined,
    ): PanelState => {
      const tabPaths = paths?.length ? paths : [fallback ?? home];
      const [sortKey, dir] = (sort ?? "name:asc").split(":");
      const tabs = tabPaths.map((p) => ({
        ...newTab({ kind: "local" }, p),
        sortKey: (["name", "ext", "size", "modified"] as SortKey[]).includes(
          sortKey as SortKey,
        )
          ? (sortKey as SortKey)
          : "name",
        sortDir: (dir === "desc" ? -1 : 1) as 1 | -1,
      }));
      const colWidths = { ...DEFAULT_COLS };
      if (cols?.length === 3) {
        [colWidths.ext, colWidths.size, colWidths.date] = cols.map((w) =>
          Math.min(Math.max(w, COL_MIN), COL_MAX),
        ) as [number, number, number];
      }
      return {
        tabs,
        active: Math.min(active ?? 0, tabs.length - 1),
        viewMode: view === "brief" ? "brief" : "full",
        colWidths,
      };
    };
    set({
      panels: [
        restore(
          config?.left_tabs,
          config?.left_path,
          config?.left_active,
          config?.left_view,
          config?.left_sort,
          config?.left_cols,
        ),
        restore(
          config?.right_tabs,
          config?.right_path,
          config?.right_active,
          config?.right_view,
          config?.right_sort,
          config?.right_cols,
        ),
      ],
    });
    await Promise.all([get().refresh(0), get().refresh(1)]);
  },

  activeTab: (side) => {
    const panel = get().panels[side];
    return panel.tabs[panel.active];
  },

  visible: (side) => visibleEntries(get().activeTab(side), useSettings.getState().showHidden),

  otherSide: (side) => (((side ?? get().activeSide) === 0 ? 1 : 0) as Side),

  setActiveSide: (side) => set({ activeSide: side }),

  navigate: async (side, path, location, remoteLabel) => {
    const tab = get().activeTab(side);
    if (!tab) return;
    const loc = location ?? tab.location;
    set((s) => ({
      panels: updateTab(s.panels, side, tab.id, {
        location: loc,
        remoteLabel: remoteLabel !== undefined ? remoteLabel : location ? null : tab.remoteLabel,
        path,
        loading: true,
        error: null,
        filter: "",
      }),
    }));
    try {
      const entries = await loadEntries(loc, path);
      set((s) => ({
        panels: updateTab(s.panels, side, tab.id, {
          entries,
          loading: false,
          cursor: 0,
          anchor: 0,
          marked: new Set<string>(),
        }),
      }));
      if (loc.kind === "local") void ipc.watchDir(side, path);
      else void ipc.unwatch(side);
      persistPanels();
    } catch (e) {
      set((s) => ({
        panels: updateTab(s.panels, side, tab.id, {
          loading: false,
          error: String(e),
        }),
      }));
    }
  },

  refresh: async (side, opts) => {
    const tab = get().activeTab(side);
    if (!tab) return;
    if (!opts?.silent) {
      set((s) => ({ panels: updateTab(s.panels, side, tab.id, { loading: true }) }));
    }
    try {
      const entries = await loadEntries(tab.location, tab.path);
      set((s) => ({
        panels: updateTab(s.panels, side, tab.id, (t) => {
          // Keep the cursor on the same entry when it survived the refresh.
          const prevVisible = visibleEntries(t, useSettings.getState().showHidden);
          const cursorName = prevVisible[t.cursor]?.name;
          const nextTabState = { ...t, entries };
          const nextVisible = visibleEntries(nextTabState, useSettings.getState().showHidden);
          const idx = cursorName ? nextVisible.findIndex((e) => e.name === cursorName) : -1;
          const names = new Set(entries.map((e) => e.name));
          return {
            entries,
            loading: false,
            error: null,
            cursor: idx >= 0 ? idx : Math.min(t.cursor, Math.max(0, nextVisible.length - 1)),
            marked: new Set([...t.marked].filter((n) => names.has(n))),
          };
        }),
      }));
      if (tab.location.kind === "local") void ipc.watchDir(side, tab.path);
    } catch (e) {
      set((s) => ({
        panels: updateTab(s.panels, side, tab.id, { loading: false, error: String(e) }),
      }));
    }
  },

  up: (side) => {
    const tab = get().activeTab(side);
    const parent = parentPath(tab.path);
    if (parent) void get().navigate(side, parent);
  },

  setCursor: (side, index, opts) => {
    const tab = get().activeTab(side);
    set((s) => ({
      panels: updateTab(s.panels, side, tab.id, {
        cursor: index,
        ...(opts?.anchor === false ? {} : { anchor: index }),
      }),
    }));
  },

  moveCursor: (side, delta, opts) => {
    const tab = get().activeTab(side);
    const visible = get().visible(side);
    if (visible.length === 0) return;
    const target = opts?.edge
      ? delta < 0
        ? 0
        : visible.length - 1
      : Math.min(Math.max(tab.cursor + delta, 0), visible.length - 1);
    set((s) => ({
      panels: updateTab(s.panels, side, tab.id, (t) => {
        const patch: Partial<TabState> = { cursor: target, anchor: target };
        if (opts?.mark) {
          const marked = new Set(t.marked);
          if (opts.edge) {
            // Shift+Home/End: mark everything from the cursor to the edge.
            const [from, to] = t.cursor <= target ? [t.cursor, target] : [target, t.cursor];
            for (let i = from; i <= to; i++) {
              const name = visible[i]?.name;
              if (name) marked.add(name);
            }
          } else {
            // Shift+arrow, TC style: toggle the row we leave, then move.
            const name = visible[t.cursor]?.name;
            if (name) {
              if (marked.has(name)) marked.delete(name);
              else marked.add(name);
            }
          }
          patch.marked = marked;
        }
        return patch;
      }),
    }));
  },

  toggleMark: (side, index, advance) => {
    const tab = get().activeTab(side);
    const visible = get().visible(side);
    const i = index ?? tab.cursor;
    const name = visible[i]?.name;
    if (!name) return;
    set((s) => ({
      panels: updateTab(s.panels, side, tab.id, (t) => {
        const marked = new Set(t.marked);
        if (marked.has(name)) marked.delete(name);
        else marked.add(name);
        return {
          marked,
          cursor: advance ? Math.min(i + 1, visible.length - 1) : t.cursor,
          anchor: i,
        };
      }),
    }));
  },

  markRange: (side, index) => {
    const tab = get().activeTab(side);
    const visible = get().visible(side);
    const [from, to] = tab.anchor <= index ? [tab.anchor, index] : [index, tab.anchor];
    set((s) => ({
      panels: updateTab(s.panels, side, tab.id, (t) => {
        const marked = new Set(t.marked);
        for (let i = from; i <= to; i++) {
          const name = visible[i]?.name;
          if (name) marked.add(name);
        }
        return { marked, cursor: index };
      }),
    }));
  },

  markAll: (side) => {
    const tab = get().activeTab(side);
    const visible = get().visible(side);
    set((s) => ({
      panels: updateTab(s.panels, side, tab.id, {
        marked: new Set(visible.map((e) => e.name)),
      }),
    }));
  },

  clearMarks: (side) => {
    const tab = get().activeTab(side);
    set((s) => ({
      panels: updateTab(s.panels, side, tab.id, { marked: new Set<string>(), filter: "" }),
    }));
  },

  setSort: (side, key) => {
    const tab = get().activeTab(side);
    set((s) => ({
      panels: updateTab(s.panels, side, tab.id, (t) => ({
        sortKey: key,
        sortDir: t.sortKey === key ? ((t.sortDir * -1) as 1 | -1) : 1,
      })),
    }));
    persistPanels();
  },

  setFilter: (side, filter) => {
    const tab = get().activeTab(side);
    set((s) => ({
      panels: updateTab(s.panels, side, tab.id, { filter, cursor: 0, anchor: 0 }),
    }));
  },

  openTab: (side, path, location, remoteLabel) => {
    const current = get().activeTab(side);
    const tab = newTab(
      location ?? current.location,
      path ?? current.path,
      remoteLabel !== undefined ? remoteLabel : current.remoteLabel,
    );
    set((s) => {
      const next = [...s.panels] as [PanelState, PanelState];
      next[side] = {
        ...next[side],
        tabs: [...next[side].tabs, tab],
        active: next[side].tabs.length,
      };
      return { panels: next };
    });
    void get().refresh(side);
    persistPanels();
  },

  closeTab: (side, index) => {
    set((s) => {
      const panel = s.panels[side];
      if (panel.tabs.length <= 1) return s;
      const tabs = panel.tabs.filter((_, i) => i !== index);
      const active = Math.min(
        index < panel.active ? panel.active - 1 : panel.active,
        tabs.length - 1,
      );
      const next = [...s.panels] as [PanelState, PanelState];
      next[side] = { ...panel, tabs, active };
      return { panels: next };
    });
    void get().refresh(side, { silent: true });
    persistPanels();
  },

  setActiveTab: (side, index) => {
    set((s) => {
      const next = [...s.panels] as [PanelState, PanelState];
      next[side] = { ...next[side], active: index };
      return { panels: next };
    });
    void get().refresh(side, { silent: true });
    persistPanels();
  },

  swapPanels: () => {
    set((s) => ({
      panels: [s.panels[1], s.panels[0]] as [PanelState, PanelState],
    }));
    void get().refresh(0, { silent: true });
    void get().refresh(1, { silent: true });
    persistPanels();
  },

  setViewMode: (side, mode) => {
    set((s) => {
      const next = [...s.panels] as [PanelState, PanelState];
      next[side] = { ...next[side], viewMode: mode };
      return { panels: next };
    });
    persistPanels();
  },

  setColWidth: (side, col, width) => {
    const clamped = Math.round(Math.min(Math.max(width, COL_MIN), COL_MAX));
    set((s) => {
      const next = [...s.panels] as [PanelState, PanelState];
      next[side] = {
        ...next[side],
        colWidths: { ...next[side].colWidths, [col]: clamped },
      };
      return { panels: next };
    });
  },

  resetColWidth: (side, col) => {
    get().setColWidth(side, col, DEFAULT_COLS[col]);
    persistPanels();
  },

  persist: () => persistPanels(),

  toggleQuickView: () => {
    set((s) => ({
      quickViewSide: s.quickViewSide === null ? s.otherSide(s.activeSide) : null,
    }));
  },

  disconnectSftp: (connId) => {
    void ipc.sftpDisconnect(connId);
    const home = useSettings.getState().home ?? "/";
    set((s) => ({
      panels: s.panels.map((panel) => ({
        ...panel,
        tabs: panel.tabs.map((tab) =>
          tab.location.kind === "sftp" && tab.location.connId === connId
            ? {
                ...tab,
                location: { kind: "local" as const },
                remoteLabel: null,
                path: home,
                entries: [],
                marked: new Set<string>(),
                cursor: 0,
                anchor: 0,
                filter: "",
              }
            : tab,
        ),
      })) as [PanelState, PanelState],
    }));
    void get().refresh(0, { silent: true });
    void get().refresh(1, { silent: true });
  },
}));
