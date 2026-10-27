import { getCurrentWindow } from "@tauri-apps/api/window";
import i18n from "../i18n";
import * as ipc from "../ipc/commands";
import { targetEntries } from "../lib/selectors";
import { useDialogs } from "../store/dialogs";
import { useOps } from "../store/ops";
import { usePanels } from "../store/panels";
import { useSettings } from "../store/settings";

export type ActionId =
  | "switchPanel"
  | "enter"
  | "up"
  | "cursorUp"
  | "cursorDown"
  | "cursorHome"
  | "cursorEnd"
  | "markToggle"
  | "markAll"
  | "clearMarks"
  | "quickView"
  | "edit"
  | "copy"
  | "move"
  | "newFolder"
  | "renameOne"
  | "multiRename"
  | "syncFolders"
  | "findFiles"
  | "trash"
  | "deletePermanent"
  | "compress"
  | "refresh"
  | "newTab"
  | "closeTab"
  | "swap"
  | "toggleHidden"
  | "commandPalette"
  | "quit";

type Modded = { mark?: boolean };

export interface Action {
  id: ActionId;
  run: (mods?: Modded) => void;
}

const panels = () => usePanels.getState();
const ops = () => useOps.getState();
const dialogs = () => useDialogs.getState();

function activeTargets() {
  const s = panels();
  const side = s.activeSide;
  const tab = s.activeTab(side);
  return { side, tab, items: targetEntries(tab, s.visible(side)) };
}

function openCopyMove(move: boolean) {
  const { side, items } = activeTargets();
  if (items.length === 0) {
    ops().toast(i18n.t("status.nothingSelected"));
    return;
  }
  const other = panels().otherSide(side);
  const destDir = panels().activeTab(other).path;
  dialogs().open({ type: "copyMove", move, side, items, destDir });
}

export const actions: Record<ActionId, Action> = {
  switchPanel: {
    id: "switchPanel",
    run: () => panels().setActiveSide(panels().otherSide(panels().activeSide)),
  },

  enter: {
    id: "enter",
    run: () => {
      const s = panels();
      const side = s.activeSide;
      const entry = s.visible(side)[s.activeTab(side).cursor];
      if (!entry) return;
      if (entry.isDir) {
        void s.navigate(side, entry.path);
      } else if (s.activeTab(side).location.kind === "local") {
        void ipc.openPath(entry.path).catch((e) => ops().toast(String(e), "error"));
      }
    },
  },

  up: { id: "up", run: () => panels().up(panels().activeSide) },

  cursorUp: {
    id: "cursorUp",
    run: (mods) => panels().moveCursor(panels().activeSide, -1, { mark: mods?.mark }),
  },
  cursorDown: {
    id: "cursorDown",
    run: (mods) => panels().moveCursor(panels().activeSide, 1, { mark: mods?.mark }),
  },
  cursorHome: {
    id: "cursorHome",
    run: (mods) =>
      panels().moveCursor(panels().activeSide, -1, { edge: true, mark: mods?.mark }),
  },
  cursorEnd: {
    id: "cursorEnd",
    run: (mods) =>
      panels().moveCursor(panels().activeSide, 1, { edge: true, mark: mods?.mark }),
  },

  markToggle: {
    id: "markToggle",
    run: () => panels().toggleMark(panels().activeSide, undefined, true),
  },
  markAll: { id: "markAll", run: () => panels().markAll(panels().activeSide) },
  clearMarks: { id: "clearMarks", run: () => panels().clearMarks(panels().activeSide) },

  quickView: { id: "quickView", run: () => panels().toggleQuickView() },

  edit: { id: "edit", run: () => ops().toast(i18n.t("status.comingSoon")) },

  copy: { id: "copy", run: () => openCopyMove(false) },
  move: { id: "move", run: () => openCopyMove(true) },

  newFolder: {
    id: "newFolder",
    run: () => dialogs().open({ type: "newFolder", side: panels().activeSide }),
  },

  renameOne: {
    id: "renameOne",
    run: () => {
      const s = panels();
      const side = s.activeSide;
      const entry = s.visible(side)[s.activeTab(side).cursor];
      if (!entry) return ops().toast(i18n.t("status.nothingSelected"));
      dialogs().open({ type: "rename", side, entry });
    },
  },

  multiRename: {
    id: "multiRename",
    run: () => {
      const { side, items } = activeTargets();
      if (items.length === 0) return ops().toast(i18n.t("status.nothingSelected"));
      dialogs().open({ type: "multiRename", side, items });
    },
  },

  syncFolders: {
    id: "syncFolders",
    run: () => {
      const s = panels();
      const left = s.activeTab(0);
      const right = s.activeTab(1);
      const label = (tab: typeof left) =>
        (tab.remoteLabel ? `${tab.remoteLabel}:` : "") + tab.path;
      dialogs().open({
        type: "sync",
        left: left.location,
        leftDir: left.path,
        leftLabel: label(left),
        right: right.location,
        rightDir: right.path,
        rightLabel: label(right),
      });
    },
  },

  findFiles: {
    id: "findFiles",
    run: () => {
      const s = panels();
      const tab = s.activeTab(s.activeSide);
      dialogs().open({
        type: "find",
        side: s.activeSide,
        location: tab.location,
        dir: tab.path,
      });
    },
  },

  trash: {
    id: "trash",
    run: () => {
      const { side, tab, items } = activeTargets();
      if (items.length === 0) return ops().toast(i18n.t("status.nothingSelected"));
      const remote = tab.location.kind === "sftp";
      // Local trash is restorable: do it without a dialog, like Finder.
      if (!remote) {
        void ops().run(i18n.t("fkeys.f8"), {
          op: "trash",
          items: items.map((e) => e.path),
        }, (done) => ops().toast(i18n.t("status.trashed", { count: done.count })));
      } else {
        dialogs().open({ type: "confirmDelete", side, items, remote });
      }
    },
  },

  deletePermanent: {
    id: "deletePermanent",
    run: () => {
      const { side, tab, items } = activeTargets();
      if (items.length === 0) return ops().toast(i18n.t("status.nothingSelected"));
      dialogs().open({
        type: "confirmDelete",
        side,
        items,
        remote: tab.location.kind === "sftp",
      });
    },
  },

  compress: {
    id: "compress",
    run: () => {
      const { side, tab, items } = activeTargets();
      if (tab.location.kind !== "local") return ops().toast(i18n.t("panel2.localOnly"));
      if (items.length === 0) return ops().toast(i18n.t("status.nothingSelected"));
      const other = panels().otherSide(side);
      dialogs().open({
        type: "compress",
        side,
        items,
        destDir: panels().activeTab(other).path,
      });
    },
  },

  refresh: {
    id: "refresh",
    run: () => void panels().refresh(panels().activeSide),
  },

  newTab: { id: "newTab", run: () => panels().openTab(panels().activeSide) },
  closeTab: {
    id: "closeTab",
    run: () => {
      const s = panels();
      s.closeTab(s.activeSide, s.panels[s.activeSide].active);
    },
  },

  swap: { id: "swap", run: () => panels().swapPanels() },

  toggleHidden: {
    id: "toggleHidden",
    run: () => {
      useSettings.getState().toggleShowHidden();
      void panels().refresh(0, { silent: true });
      void panels().refresh(1, { silent: true });
    },
  },

  commandPalette: {
    id: "commandPalette",
    run: () => ops().toast(i18n.t("status.comingSoon")),
  },

  quit: { id: "quit", run: () => void getCurrentWindow().close() },
};
