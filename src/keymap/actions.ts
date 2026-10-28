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
  | "compareFiles"
  | "copyPath"
  | "syncFolders"
  | "findFiles"
  | "trash"
  | "deletePermanent"
  | "compress"
  | "extract"
  | "refresh"
  | "newTab"
  | "closeTab"
  | "newWindow"
  | "swap"
  | "toggleHidden"
  | "commandPalette"
  | "openSettings"
  | "quit";

type Modded = { mark?: boolean };

export interface Action {
  id: ActionId;
  run: (mods?: Modded) => void;
}

/** Keep in sync with the backend's is_archive (ops/archive.rs). */
export const ARCHIVE_RE = /\.(zip|7z|tar|tgz|gz)$/i;

const panels = () => usePanels.getState();
const ops = () => useOps.getState();
const dialogs = () => useDialogs.getState();

function activeTargets() {
  const s = panels();
  const side = s.activeSide;
  const tab = s.activeTab(side);
  return { side, tab, items: targetEntries(tab, s.visible(side)) };
}

/** Clipboard write that works in WKWebView without a permission prompt. */
function copyText(text: string) {
  const ta = document.createElement("textarea");
  ta.value = text;
  ta.style.position = "fixed";
  ta.style.opacity = "0";
  document.body.appendChild(ta);
  ta.select();
  document.execCommand("copy");
  ta.remove();
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
        // Browsable archives open like folders (screen 2d).
        if (/\.(zip|tar|tgz)$/i.test(entry.name) || /\.tar\.gz$/i.test(entry.name)) {
          void s.navigate(side, "", { kind: "archive", archive: entry.path }, entry.name);
        } else {
          void ipc.openPath(entry.path).catch((e) => ops().toast(String(e), "error"));
        }
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

  compareFiles: {
    id: "compareFiles",
    run: () => {
      const s = panels();
      // One file per panel: the single marked entry, or the one under cursor.
      const pick = (side: 0 | 1) => {
        const tab = s.activeTab(side);
        return { tab, entry: targetEntries(tab, s.visible(side))[0] };
      };
      const l = pick(0);
      const r = pick(1);
      const le = l.entry;
      const re = r.entry;
      if (!le || !re || le.isDir || re.isDir) {
        return ops().toast(i18n.t("compare.needTwo"));
      }
      const prefix = (tab: { remoteLabel: string | null }) =>
        tab.remoteLabel ? `${tab.remoteLabel}:` : "";
      dialogs().open({
        type: "compare",
        left: { location: l.tab.location, path: le.path, label: prefix(l.tab) + le.path },
        right: { location: r.tab.location, path: re.path, label: prefix(r.tab) + re.path },
      });
    },
  },

  copyPath: {
    id: "copyPath",
    run: () => {
      const { items } = activeTargets();
      if (items.length === 0) return ops().toast(i18n.t("status.nothingSelected"));
      copyText(items.map((e) => e.path).join("\n"));
      ops().toast(i18n.t("status.pathCopied", { count: items.length }));
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
      if (tab.location.kind === "archive")
        return ops().toast(i18n.t("status.archiveReadOnly"));
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
      if (tab.location.kind === "archive")
        return ops().toast(i18n.t("status.archiveReadOnly"));
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

  extract: {
    id: "extract",
    run: () => {
      const { side, tab, items } = activeTargets();
      if (tab.location.kind !== "local") return ops().toast(i18n.t("panel2.localOnly"));
      const archives = items.filter((e) => !e.isDir && ARCHIVE_RE.test(e.name));
      if (archives.length === 0) return ops().toast(i18n.t("status.nothingSelected"));
      const otherTab = panels().activeTab(panels().otherSide(side));
      if (otherTab.location.kind !== "local") return ops().toast(i18n.t("panel2.localOnly"));
      void ops().run(
        i18n.t("action.extract"),
        { op: "extract", archives: archives.map((e) => e.path), destDir: otherTab.path },
        (done) => ops().toast(i18n.t("status.extracted", { count: done.count })),
      );
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

  newWindow: {
    id: "newWindow",
    run: () => void ipc.newWindow().catch((e) => ops().toast(String(e), "error")),
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
    run: () => dialogs().open({ type: "palette" }),
  },

  openSettings: {
    id: "openSettings",
    run: () => dialogs().open({ type: "settings", section: "hotkeys" }),
  },

  // Quit closes every window (size/state still saved per window); the red
  // traffic light closes just one.
  quit: { id: "quit", run: () => void ipc.quitApp() },
};
