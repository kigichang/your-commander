import { useVirtualizer } from "@tanstack/react-virtual";
import clsx from "clsx";
import { ChevronDown, ChevronUp, CornerLeftUp } from "lucide-react";
import { useEffect, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";
import type { FsEntry, Location } from "../ipc/types";
import { useClipboard } from "../store/clipboard";
import { formatDate, humanSize } from "../lib/format";
import { parentPath } from "../lib/path";
import { visibleEntries } from "../lib/selectors";
import { actions } from "../keymap/actions";
import type { ColKey, ColWidths, Side, SortKey, TabState, ViewMode } from "../store/panels";
import { usePanels } from "../store/panels";
import { useSettings } from "../store/settings";
import { useCtxMenu } from "./ContextMenu";
import { FileIcon } from "./FileIcon";
import { Icon } from "./icons";

const ROW_HEIGHT = 26;

const COLUMNS: Array<{
  key: SortKey;
  label: string;
  className?: string;
  /** Which width this column's LEFT divider adjusts (undefined = none). */
  col?: ColKey;
}> = [
  { key: "name", label: "list.name" },
  { key: "ext", label: "list.ext", col: "ext" },
  { key: "size", label: "list.size", className: "num", col: "size" },
  { key: "modified", label: "list.date", className: "num", col: "date" },
];

function gridColumns(w: ColWidths): string {
  return `minmax(0, 1fr) ${w.ext}px ${w.size}px ${w.date}px`;
}

function sameLocation(a: Location, b: Location): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === "sftp" && b.kind === "sftp") return a.connId === b.connId;
  if (a.kind === "archive" && b.kind === "archive") return a.archive === b.archive;
  return true;
}

/** Header divider: dragging moves the boundary with the mouse by resizing
 *  the column to its right (Name soaks up the slack). Double-click resets. */
function ColResize({ side, col }: { side: Side; col: ColKey }) {
  const onMouseDown = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const startX = e.clientX;
    const startW = usePanels.getState().panels[side].colWidths[col];
    const move = (ev: MouseEvent) =>
      usePanels.getState().setColWidth(side, col, startW - (ev.clientX - startX));
    const up = () => {
      window.removeEventListener("mousemove", move);
      document.body.style.cursor = "";
      usePanels.getState().persist();
    };
    document.body.style.cursor = "col-resize";
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up, { once: true });
  };
  return (
    <div
      className="col-resize"
      onMouseDown={onMouseDown}
      onDoubleClick={(e) => {
        e.stopPropagation();
        usePanels.getState().resetColWidth(side, col);
      }}
    />
  );
}

export function FileTable({
  side,
  tab,
  viewMode,
}: {
  side: Side;
  tab: TabState;
  viewMode: ViewMode;
}) {
  const { t } = useTranslation();
  const showHidden = useSettings((s) => s.showHidden);
  const visible = useMemo(() => visibleEntries(tab, showHidden), [tab, showHidden]);
  const scrollRef = useRef<HTMLDivElement>(null);

  const virtualizer = useVirtualizer({
    count: visible.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 12,
  });

  // Keep the cursor row in view when it moves by keyboard.
  useEffect(() => {
    if (visible.length > 0) virtualizer.scrollToIndex(tab.cursor);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab.cursor]);

  const brief = viewMode === "brief";
  const colWidths = usePanels((s) => s.panels[side].colWidths);
  const cols = gridColumns(colWidths);

  // Items pending ⌘X show greyed, pending ⌘C show bold, until pasted or Esc.
  const clip = useClipboard((s) => s.clip);
  const clipPaths = useMemo(
    () =>
      clip && sameLocation(clip.location, tab.location)
        ? new Set(clip.items.map((i) => i.path))
        : null,
    [clip, tab.location],
  );

  return (
    <div className="ftable">
      {!brief && (
        <div className="thead" style={{ gridTemplateColumns: cols }}>
          {COLUMNS.map((c) => (
            <div key={c.key} className="th">
              {c.col && <ColResize side={side} col={c.col} />}
              <button
                className={clsx(c.className, tab.sortKey === c.key && "on")}
                onClick={() => usePanels.getState().setSort(side, c.key)}
              >
                {t(c.label)}
                {tab.sortKey === c.key && (
                  <Icon of={tab.sortDir === 1 ? ChevronUp : ChevronDown} size={12} />
                )}
              </button>
            </div>
          ))}
        </div>
      )}
      {(parentPath(tab.path) || tab.location.kind === "archive") && (
        <div
          className="row parent"
          title=".."
          onDoubleClick={() => {
            usePanels.getState().setActiveSide(side);
            usePanels.getState().up(side);
          }}
        >
          <span className="name">
            <span className="icon">
              <Icon of={CornerLeftUp} size={14} />
            </span>
            ..
          </span>
        </div>
      )}
      <div
        className="tbody"
        ref={scrollRef}
        onContextMenu={(e) => {
          // Rows handle their own menu; this is the blank space below them.
          if ((e.target as HTMLElement).closest(".row")) return;
          e.preventDefault();
          e.stopPropagation();
          usePanels.getState().setActiveSide(side);
          useCtxMenu.getState().open(e.clientX, e.clientY, side, "empty");
        }}
      >
        {tab.error ? (
          <div className="error">{tab.error}</div>
        ) : visible.length === 0 && !tab.loading ? (
          <div className="empty">{t("list.empty")}</div>
        ) : (
          <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
            {virtualizer.getVirtualItems().map((item) => (
              <Row
                key={item.key}
                entry={visible[item.index]}
                index={item.index}
                side={side}
                tab={tab}
                brief={brief}
                top={item.start}
                cols={cols}
                clipKind={
                  clipPaths?.has(visible[item.index].path)
                    ? clip!.move
                      ? "cut"
                      : "copy"
                    : null
                }
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function Row({
  entry,
  index,
  side,
  tab,
  brief,
  top,
  cols,
  clipKind,
}: {
  entry: FsEntry;
  index: number;
  side: Side;
  tab: TabState;
  brief: boolean;
  top: number;
  cols: string;
  clipKind: "cut" | "copy" | null;
}) {
  const isCursor = index === tab.cursor;
  const isMarked = tab.marked.has(entry.name);

  const onMouseDown = (e: React.MouseEvent) => {
    if (e.button === 2) return; // handled by onContextMenu
    const s = usePanels.getState();
    if (e.metaKey) {
      s.toggleMark(side, index, false);
    } else if (e.shiftKey) {
      s.markRange(side, index);
    } else {
      s.setCursor(side, index);
    }
  };

  // Finder convention: right-clicking a marked row acts on the whole mark
  // set; right-clicking anything else retargets to that row alone.
  const onContextMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const s = usePanels.getState();
    s.setActiveSide(side);
    if (!tab.marked.has(entry.name)) s.unmarkAll(side);
    s.setCursor(side, index);
    useCtxMenu.getState().open(e.clientX, e.clientY, side, "row");
  };

  return (
    <div
      className={clsx(
        "row",
        entry.isDir && "dir",
        entry.isSymlink && "symlink",
        entry.isHidden && "hidden-entry",
        isCursor && "cursor",
        isMarked && "marked",
        clipKind === "cut" && "clip-cut",
        clipKind === "copy" && "clip-copy",
      )}
      style={{ top, gridTemplateColumns: brief ? "1fr" : cols }}
      onMouseDown={onMouseDown}
      onContextMenu={onContextMenu}
      onDoubleClick={() => {
        usePanels.getState().setActiveSide(side);
        actions.enter.run();
      }}
    >
      <span className="name">
        <FileIcon isDir={entry.isDir} ext={entry.ext} />
        {entry.name}
      </span>
      {!brief && <span className="ext">{entry.ext}</span>}
      {!brief && (
        <span className="num">{entry.isDir ? "—" : humanSize(entry.size)}</span>
      )}
      {!brief && <span className="num date">{formatDate(entry.modified)}</span>}
    </div>
  );
}
