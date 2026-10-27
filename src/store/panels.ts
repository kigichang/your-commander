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

export interface PanelState {
  tabs: TabState[];
  active: number;
  viewMode: ViewMode;
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

export const usePanels = create<PanelsStore>((set, get) => ({
  panels: [
    { tabs: [], active: 0, viewMode: "full" },
    { tabs: [], active: 0, viewMode: "full" },
  ],
  activeSide: 0,
  quickViewSide: null,

  init: async () => {
    const home = await ipc.homeDir();
    const config = useSettings.getState().config;
    const left = config?.left_path ?? home;
    const right = config?.right_path ?? home;
    set({
      panels: [
        { tabs: [newTab({ kind: "local" }, left)], active: 0, viewMode: "full" },
        { tabs: [newTab({ kind: "local" }, right)], active: 0, viewMode: "full" },
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
      const settings = useSettings.getState();
      if (loc.kind === "local" && settings.config) {
        settings.save(side === 0 ? { left_path: path } : { right_path: path });
      }
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
  },

  setActiveTab: (side, index) => {
    set((s) => {
      const next = [...s.panels] as [PanelState, PanelState];
      next[side] = { ...next[side], active: index };
      return { panels: next };
    });
    void get().refresh(side, { silent: true });
  },

  swapPanels: () => {
    set((s) => ({
      panels: [s.panels[1], s.panels[0]] as [PanelState, PanelState],
    }));
    void get().refresh(0, { silent: true });
    void get().refresh(1, { silent: true });
  },

  setViewMode: (side, mode) => {
    set((s) => {
      const next = [...s.panels] as [PanelState, PanelState];
      next[side] = { ...next[side], viewMode: mode };
      return { panels: next };
    });
  },

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
