import { getCurrentWindow } from "@tauri-apps/api/window";
import i18n from "../i18n";
import * as ipc from "../ipc/commands";
import { iconGridCols } from "../lib/iconGrid";
import { alreadyIn, isTrashDir, remotePrefix } from "../lib/path";
import { targetEntries } from "../lib/selectors";
import { useClipboard } from "../store/clipboard";
import { useDialogs } from "../store/dialogs";
import { useOps } from "../store/ops";
import { usePanels } from "../store/panels";
import { useSettings } from "../store/settings";

export type ActionId =
  | "switchPanel"
  | "enter"
  | "up"
  | "goBack"
  | "goForward"
  | "focusSearch"
  | "cursorUp"
  | "cursorDown"
  | "cursorLeft"
  | "cursorRight"
  | "cursorHome"
  | "cursorEnd"
  | "markToggle"
  | "markAll"
  | "clearMarks"
  | "quickView"
  | "quickViewHex"
  | "getInfo"
  | "edit"
  | "copy"
  | "move"
  | "clipCopy"
  | "clipCut"
  | "clipPaste"
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
  | "newTerminalHere"
  | "newVscodeHere"
  | "newZedHere"
  | "swap"
  | "toggleHidden"
  | "commandPalette"
  | "openSettings"
  | "openTrash"
  | "openDesktop"
  | "restore"
  | "emptyTrash"
  | "quit";

type Modded = { mark?: boolean };

export interface Action {
  run: (mods?: Modded) => void;
  /** When false, the key is left to the browser (not consumed). */
  enabled?: () => boolean;
}

/** Archive names by suffix. Keep both in sync with the backend's SUFFIXES
 *  table (ops/archive.rs): everything it extracts, and the subset the panel
 *  can step into like a folder (fs/archive_fs.rs). */
export const ARCHIVE_RE = /\.(zip|7z|tar|tgz|gz)$/i;
const BROWSABLE_ARCHIVE_RE = /\.(zip|tar|tgz|tar\.gz)$/i;

const panels = () => usePanels.getState();
const ops = () => useOps.getState();

/** Open a backend-resolved local folder (Trash, Desktop) in the active panel. */
const openSpecialDir = (resolve: () => Promise<string>) =>
  void resolve()
    .then((dir) => panels().navigate(panels().activeSide, dir, { kind: "local" }))
    .catch((e) => ops().toast(String(e), "error"));

const inIconView = () => {
  const s = panels();
  return s.panels[s.activeSide].viewMode === "icon";
};

/** ↑/↓ step: one row, which in the Icon view is a whole line of tiles. */
const rowStep = () => (inIconView() ? iconGridCols[panels().activeSide] : 1);
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

/** "New … Here": launch a terminal/editor in the active panel's folder. */
function openHere(tool: "terminal" | "vscode" | "zed") {
  const s = panels();
  const tab = s.activeTab(s.activeSide);
  if (tab.location.kind !== "local") return ops().toast(i18n.t("panel2.localOnly"));
  void ipc.openIn(tool, tab.path).catch((e) => ops().toast(String(e), "error"));
}

/** ⌘C/⌘X: put the selection on the in-app clipboard for a later paste. */
function clipTake(move: boolean) {
  const { tab, items } = activeTargets();
  if (items.length === 0) return ops().toast(i18n.t("status.nothingSelected"));
  if (move && tab.location.kind === "archive")
    return ops().toast(i18n.t("status.archiveReadOnly"));
  useClipboard.getState().set({ items, location: tab.location, move });
  ops().toast(i18n.t(move ? "status.clipCut" : "status.clipCopied", { count: items.length }));
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
    run: () => {
      const s = panels();
      const target = s.otherSide(s.activeSide);
      // Never focus a pane that is showing the Quick View preview.
      if (s.quickViewSide === target) return;
      s.setActiveSide(target);
    },
  },

  enter: {
    run: () => {
      const s = panels();
      const side = s.activeSide;
      const entry = s.visible(side)[s.activeTab(side).cursor];
      if (!entry) return;
      if (entry.isDir) {
        void s.navigate(side, entry.path);
      } else if (s.activeTab(side).location.kind === "local") {
        // Browsable archives open like folders (screen 2d).
        if (BROWSABLE_ARCHIVE_RE.test(entry.name)) {
          void s.navigate(side, "", { kind: "archive", archive: entry.path }, entry.name);
        } else {
          void ipc.openPath(entry.path).catch((e) => ops().toast(String(e), "error"));
        }
      }
    },
  },

  up: { run: () => panels().up(panels().activeSide) },
  goBack: { run: () => panels().goBack(panels().activeSide) },
  goForward: { run: () => panels().goForward(panels().activeSide) },
  // The toolbar's search field (live filter of the active panel).
  focusSearch: {
    run: () => {
      const el = document.querySelector<HTMLInputElement>(".search-cap input");
      el?.focus();
      el?.select();
    },
  },

  cursorUp: {
    run: (mods) =>
      panels().moveCursor(panels().activeSide, -rowStep(), { mark: mods?.mark }),
  },
  cursorDown: {
    run: (mods) =>
      panels().moveCursor(panels().activeSide, rowStep(), { mark: mods?.mark }),
  },
  // ←/→ only mean something in the Icon view's grid.
  cursorLeft: {
    enabled: inIconView,
    run: (mods) => panels().moveCursor(panels().activeSide, -1, { mark: mods?.mark }),
  },
  cursorRight: {
    enabled: inIconView,
    run: (mods) => panels().moveCursor(panels().activeSide, 1, { mark: mods?.mark }),
  },
  cursorHome: {
    run: (mods) =>
      panels().moveCursor(panels().activeSide, -1, { edge: true, mark: mods?.mark }),
  },
  cursorEnd: {
    run: (mods) =>
      panels().moveCursor(panels().activeSide, 1, { edge: true, mark: mods?.mark }),
  },

  markToggle: {
    run: () => panels().toggleMark(panels().activeSide, undefined, true),
  },
  markAll: { run: () => panels().markAll(panels().activeSide) },
  clearMarks: {
    run: () => {
      // Esc peels one layer at a time: a pending ⌘C/⌘X first, marks next.
      if (useClipboard.getState().clip) {
        useClipboard.getState().clear();
        ops().toast(i18n.t("status.clipCancelled"));
        return;
      }
      panels().clearMarks(panels().activeSide);
    },
  },

  quickView: { run: () => panels().toggleQuickView() },
  quickViewHex: { run: () => panels().toggleQuickViewHex() },

  getInfo: {
    run: () => {
      const { tab, items } = activeTargets();
      if (tab.location.kind !== "local") return ops().toast(i18n.t("panel2.localOnly"));
      if (items.length === 0) return ops().toast(i18n.t("status.nothingSelected"));
      void ipc
        .getInfo(items.map((e) => e.path))
        .catch((e) => ops().toast(String(e), "error"));
    },
  },

  edit: { run: () => ops().toast(i18n.t("status.comingSoon")) },

  copy: { run: () => openCopyMove(false) },
  move: { run: () => openCopyMove(true) },

  clipCopy: { run: () => clipTake(false) },
  clipCut: { run: () => clipTake(true) },

  clipPaste: {
    run: () => {
      const clip = useClipboard.getState().clip;
      if (!clip) return ops().toast(i18n.t("status.clipEmpty"));
      const s = panels();
      const tab = s.activeTab(s.activeSide);
      if (tab.location.kind === "archive")
        return ops().toast(i18n.t("status.archiveReadOnly"));
      // Cut and pasted back where it came from: nothing to move. The cut
      // stays on the clipboard for a paste somewhere else.
      if (
        clip.move &&
        alreadyIn(clip.location, clip.items.map((e) => e.path), tab.location, tab.path)
      )
        return ops().toast(i18n.t("status.moveSameFolder"), "error");
      ops().copy({
        src: clip.location,
        sources: clip.items.map((e) => ({ path: e.path, isDir: e.isDir, isSymlink: e.isSymlink })),
        dest: tab.location,
        destDir: tab.path,
        move: clip.move,
      });
      // A cut pastes once; a copy can paste again elsewhere.
      if (clip.move) useClipboard.getState().clear();
    },
  },

  newFolder: {
    run: () => dialogs().open({ type: "newFolder", side: panels().activeSide }),
  },

  renameOne: {
    run: () => {
      const s = panels();
      const side = s.activeSide;
      const entry = s.visible(side)[s.activeTab(side).cursor];
      if (!entry) return ops().toast(i18n.t("status.nothingSelected"));
      dialogs().open({ type: "rename", side, entry });
    },
  },

  multiRename: {
    run: () => {
      const { side, items } = activeTargets();
      if (items.length === 0) return ops().toast(i18n.t("status.nothingSelected"));
      dialogs().open({ type: "multiRename", side, items });
    },
  },

  compareFiles: {
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
      dialogs().open({
        type: "compare",
        left: { location: l.tab.location, path: le.path, label: remotePrefix(l.tab) + le.path },
        right: { location: r.tab.location, path: re.path, label: remotePrefix(r.tab) + re.path },
      });
    },
  },

  copyPath: {
    run: () => {
      const { items } = activeTargets();
      if (items.length === 0) return ops().toast(i18n.t("status.nothingSelected"));
      copyText(items.map((e) => e.path).join("\n"));
      ops().toast(i18n.t("status.pathCopied", { count: items.length }));
    },
  },

  syncFolders: {
    run: () => {
      const s = panels();
      const left = s.activeTab(0);
      const right = s.activeTab(1);
      dialogs().open({
        type: "sync",
        left: left.location,
        leftDir: left.path,
        leftLabel: remotePrefix(left) + left.path,
        right: right.location,
        rightDir: right.path,
        rightLabel: remotePrefix(right) + right.path,
      });
    },
  },

  findFiles: {
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

  // Trash folder only: put the targets back where they came from.
  restore: {
    run: () => {
      const { tab, items } = activeTargets();
      if (!isTrashDir(tab.location, tab.path, useSettings.getState().trashDir))
        return ops().toast(i18n.t("status.notInTrash"));
      if (items.length === 0) return ops().toast(i18n.t("status.nothingSelected"));
      void ops().run(
        i18n.t("action.restore"),
        { op: "restore", items: items.map((e) => e.path) },
        (done) => ops().toast(i18n.t("status.restored", { count: done.count })),
      );
    },
  },

  emptyTrash: {
    run: () => dialogs().open({ type: "confirmEmptyTrash" }),
  },

  deletePermanent: {
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
    run: () => void panels().refresh(panels().activeSide),
  },

  newTab: { run: () => panels().openTab(panels().activeSide) },
  closeTab: {
    run: () => {
      const s = panels();
      // Browser convention: ⌘W closes the tab, and the window itself once
      // the active side is down to its last tab.
      if (s.panels[s.activeSide].tabs.length <= 1) {
        void getCurrentWindow().close();
        return;
      }
      s.closeTab(s.activeSide, s.panels[s.activeSide].active);
    },
  },

  newWindow: {
    run: () => void ipc.newWindow().catch((e) => ops().toast(String(e), "error")),
  },

  newTerminalHere: { run: () => openHere("terminal") },
  newVscodeHere: { run: () => openHere("vscode") },
  newZedHere: { run: () => openHere("zed") },

  swap: { run: () => panels().swapPanels() },

  toggleHidden: {
    run: () => {
      useSettings.getState().toggleShowHidden();
      void panels().refresh(0, { silent: true });
      void panels().refresh(1, { silent: true });
    },
  },

  commandPalette: {
    run: () => dialogs().open({ type: "palette" }),
  },

  openSettings: {
    run: () => dialogs().open({ type: "settings", section: "general" }),
  },

  // Show the Trash's / Desktop's contents in the active panel.
  openTrash: { run: () => openSpecialDir(ipc.trashDir) },
  openDesktop: { run: () => openSpecialDir(ipc.desktopDir) },

  // Quit closes every window (size/state still saved per window); the red
  // traffic light closes just one.
  quit: { run: () => void ipc.quitApp() },
};
