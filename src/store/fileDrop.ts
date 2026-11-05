import { getCurrentWebview } from "@tauri-apps/api/webview";
import { create } from "zustand";
import i18n from "../i18n";
import * as ipc from "../ipc/commands";
import { parentPath } from "../lib/path";
import { useOps } from "./ops";
import type { Side } from "./panels";
import { usePanels } from "./panels";

/** Where files dragged in from another app (Finder, the desktop, or this
 *  app's own drag-out) would land: a panel's current folder, or the
 *  folder row under the pointer. */
export interface DropTarget {
  side: Side;
  dir: string;
  /** Set when the pointer is over a folder row (highlights that row). */
  rowPath: string | null;
}

interface FileDropStore {
  target: DropTarget | null;
}

export const useFileDrop = create<FileDropStore>(() => ({ target: null }));

/** Hit-test a webview position (physical px) against the panels. Rows
 *  that are folders carry data-dir-path, panels data-side. */
function hitTest(pos: { x: number; y: number }): DropTarget | null {
  const scale = window.devicePixelRatio || 1;
  const el = document.elementFromPoint(pos.x / scale, pos.y / scale);
  const panelEl = el?.closest<HTMLElement>("[data-side]");
  if (!el || !panelEl) return null;
  const side: Side = panelEl.dataset.side === "1" ? 1 : 0;
  const s = usePanels.getState();
  if (s.quickViewSide === side) return null;
  const tab = s.activeTab(side);
  if (tab.location.kind === "archive") return null;
  const rowPath = el.closest<HTMLElement>("[data-dir-path]")?.dataset.dirPath ?? null;
  return { side, dir: rowPath ?? tab.path, rowPath };
}

function sameTarget(a: DropTarget | null, b: DropTarget | null): boolean {
  return a?.side === b?.side && a?.dir === b?.dir && a?.rowPath === b?.rowPath;
}

function setTarget(next: DropTarget | null) {
  if (!sameTarget(useFileDrop.getState().target, next)) useFileDrop.setState({ target: next });
}

/** Copy dropped files into the target. Always a copy — like the drag-out
 *  side, a drop never moves files away from where they came from. */
async function dropInto(paths: string[], target: DropTarget) {
  const { dir, side } = target;
  // Dropping items onto their own folder (or a folder onto itself) is a
  // no-op, not a duplicate.
  const wanted = paths.filter(
    (p) => parentPath(p) !== dir && dir !== p && !dir.startsWith(p + "/"),
  );
  if (wanted.length === 0) return;
  const ops = useOps.getState();
  try {
    const sources = await ipc.statPaths(wanted);
    if (sources.length === 0) return;
    const panels = usePanels.getState();
    panels.setActiveSide(side);
    ops.run(
      i18n.t("dialog.copy"),
      {
        op: "copy",
        src: { kind: "local" },
        sources,
        dest: panels.activeTab(side).location,
        destDir: dir,
        removeSource: false,
      },
      (done) => ops.toast(i18n.t("status.copied", { count: done.count })),
    );
  } catch (e) {
    ops.toast(String(e), "error");
  }
}

/** Listen for OS file drags over this window. Returns the unlisten fn. */
export async function initFileDrop(): Promise<() => void> {
  return getCurrentWebview().onDragDropEvent(({ payload }) => {
    switch (payload.type) {
      case "enter":
      case "over":
        setTarget(hitTest(payload.position));
        break;
      case "drop": {
        const target = hitTest(payload.position);
        setTarget(null);
        if (target && payload.paths.length > 0) void dropInto(payload.paths, target);
        break;
      }
      case "leave":
        setTarget(null);
        break;
    }
  });
}
