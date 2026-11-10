import { useVirtualizer } from "@tanstack/react-virtual";
import clsx from "clsx";
import { ChevronDown, ChevronUp, CloudDownload, CornerLeftUp } from "lucide-react";
import { memo, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { FsEntry } from "../ipc/types";
import { useClipboard } from "../store/clipboard";
import { useFileDrop } from "../store/fileDrop";
import { useScaled } from "../lib/fontScale";
import { formatDate, humanSize } from "../lib/format";
import { iconGridCols } from "../lib/iconGrid";
import { parentPath, sameLocation } from "../lib/path";
import { isMac, isModKey } from "../lib/platform";
import { targetEntries, visibleEntries } from "../lib/selectors";
import { openPath, startDrag } from "../ipc/commands";
import { actions } from "../keymap/actions";
import type { ColKey, ColWidths, Side, SortKey, TabState, ViewMode } from "../store/panels";
import { usePanels } from "../store/panels";
import { selectShowHidden, useSettings } from "../store/settings";
import { useCtxMenu } from "./ContextMenu";
import { FileIcon, iconKey } from "./FileIcon";
import { Icon } from "./icons";

/** Designed sizes at font scale 1; the font-size setting scales them. */
const ROW_HEIGHT = 26;
/** Icon view tiles: narrowest tile width (the row stretches them to fill)
 *  and height: a fixed part (48 px icon, gap, padding) plus two lines of
 *  name, which grow with the font. */
const TILE_MIN_WIDTH = 96;
const TILE_FIXED_HEIGHT = 62;
const TILE_TEXT_HEIGHT = 30;
const TILE_GAP = 4;

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
  const showHidden = useSettings(selectShowHidden);
  const visible = useMemo(() => visibleEntries(tab, showHidden), [tab, showHidden]);
  const scrollRef = useRef<HTMLDivElement>(null);
  const iconView = viewMode === "icon";

  // Icon view lays tiles out in rows as wide as the panel allows.
  const [width, setWidth] = useState(0);
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setWidth(el.clientWidth));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const rowHeight = useScaled(ROW_HEIGHT);
  const tileHeight = TILE_FIXED_HEIGHT + useScaled(TILE_TEXT_HEIGHT);
  const tileMinWidth = useScaled(TILE_MIN_WIDTH);
  const gridCols = iconView ? Math.max(1, Math.floor(width / tileMinWidth)) : 1;
  iconGridCols[side] = gridCols;

  // In the Icon view each virtual item is a row of tiles.
  const virtualizer = useVirtualizer({
    count: Math.ceil(visible.length / gridCols),
    getScrollElement: () => scrollRef.current,
    estimateSize: () => (iconView ? tileHeight : rowHeight),
    overscan: iconView ? 4 : 12,
  });
  useEffect(() => virtualizer.measure(), [iconView, rowHeight, tileHeight, virtualizer]);

  // Keep the cursor row in view when it moves by keyboard.
  useEffect(() => {
    if (visible.length > 0) virtualizer.scrollToIndex(Math.floor(tab.cursor / gridCols));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab.cursor]);

  const brief = viewMode !== "full";
  // Selection = what an operation would act on (see targetEntries): the
  // marked set, or the cursor entry when nothing is marked.
  const hasMarks = tab.marked.size > 0;
  const colWidths = usePanels((s) => s.panels[side].colWidths);
  const cols = gridColumns(colWidths);

  // Items pending ⌘X show greyed, pending ⌘C show bold, until pasted or Esc.
  const clip = useClipboard((s) => s.clip);
  // Folder row a file drag is hovering (it would drop into that folder).
  const dropRow = useFileDrop((s) => (s.target?.side === side ? s.target.rowPath : null));
  const clipPaths = useMemo(
    () =>
      clip && sameLocation(clip.location, tab.location)
        ? new Set(clip.items.map((i) => i.path))
        : null,
    [clip, tab.location],
  );

  return (
    // Rows read the column template from --grid-cols, so a column drag
    // restyles them through CSS without re-rendering a single memoized row.
    <div
      className={clsx("ftable", hasMarks && "has-marks")}
      style={{ "--grid-cols": cols } as React.CSSProperties}
    >
      {!brief && (
        <div className="thead" style={{ gridTemplateColumns: "var(--grid-cols)" }}>
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
          if ((e.target as HTMLElement).closest(".row, .tile")) return;
          e.preventDefault();
          e.stopPropagation();
          usePanels.getState().setActiveSide(side);
          useCtxMenu.getState().open(e.clientX, e.clientY, side, "empty");
        }}
      >
        {tab.error ? (
          <div className="error">
            <span>{tab.error}</span>
            {/* macOS privacy protection: ~/.Trash needs Full Disk Access. */}
            {isMac &&
              tab.location.kind === "local" &&
              /\/\.Trash\/?$/.test(tab.path) &&
              /not permitted|os error 1\b/i.test(tab.error) && (
                <>
                  <span className="hint">{t("list.trashNeedsAccess")}</span>
                  <button
                    className="btn"
                    onClick={() =>
                      void openPath(
                        "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles",
                      )
                    }
                  >
                    {t("list.openPrivacy")}
                  </button>
                </>
              )}
          </div>
        ) : visible.length === 0 && !tab.loading ? (
          <div className="empty">{t("list.empty")}</div>
        ) : (
          <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
            {iconView &&
              virtualizer.getVirtualItems().map((item) =>
                visible
                  .slice(item.index * gridCols, (item.index + 1) * gridCols)
                  .map((entry, col) => {
                    const index = item.index * gridCols + col;
                    return (
                      <Tile
                        key={entry.path}
                        entry={entry}
                        index={index}
                        side={side}
                        top={item.start}
                        height={item.size}
                        col={col}
                        cols={gridCols}
                        isCursor={index === tab.cursor}
                        isMarked={tab.marked.has(entry.name)}
                        hasMarks={hasMarks}
                        isDropTarget={dropRow === entry.path}
                        clipKind={
                          clipPaths?.has(entry.path) ? (clip!.move ? "cut" : "copy") : null
                        }
                      />
                    );
                  }),
              )}
            {!iconView &&
              virtualizer.getVirtualItems().map((item) => (
              <Row
                key={item.key}
                entry={visible[item.index]}
                index={item.index}
                side={side}
                brief={brief}
                top={item.start}
                height={item.size}
                isCursor={item.index === tab.cursor}
                isMarked={tab.marked.has(visible[item.index].name)}
                hasMarks={hasMarks}
                isDropTarget={dropRow === visible[item.index].path}
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

/** Pointer travel (px) before a press on a row turns into a drag-out. */
const DRAG_THRESHOLD = 4;

/** Finder-style drag-out: once a press on a row moves past the threshold,
 *  hand the files to a native drag session so they can be dropped on other
 *  apps. Dragging a marked row carries the whole mark set; any other row
 *  goes alone. Local files only — SFTP and archive entries have no path
 *  another app could open. */
function armDragOut(e: React.MouseEvent, side: Side, entry: FsEntry) {
  if (usePanels.getState().activeTab(side).location.kind !== "local") return;
  const x0 = e.clientX;
  const y0 = e.clientY;
  const disarm = () => {
    window.removeEventListener("mousemove", move);
    window.removeEventListener("mouseup", disarm);
  };
  const move = (ev: MouseEvent) => {
    if (Math.abs(ev.clientX - x0) < DRAG_THRESHOLD && Math.abs(ev.clientY - y0) < DRAG_THRESHOLD)
      return;
    // The native session owns the pointer from here: no mouseup will
    // reach the webview, so stop listening now.
    disarm();
    const tab = usePanels.getState().activeTab(side);
    const items = tab.marked.has(entry.name)
      ? targetEntries(tab, visibleEntries(tab, selectShowHidden(useSettings.getState())))
      : [entry];
    if (items.length === 0) return;
    startDrag(
      items.map((i) => i.path),
      iconKey(entry.isDir, entry.ext, entry.path),
    ).catch((err) => console.error("drag-out failed:", err));
  };
  window.addEventListener("mousemove", move);
  window.addEventListener("mouseup", disarm);
}

/** Mouse behavior shared by list rows and icon tiles. */
function entryHandlers(side: Side, index: number, entry: FsEntry) {
  const onMouseDown = (e: React.MouseEvent) => {
    if (e.button === 2) return; // handled by onContextMenu
    const s = usePanels.getState();
    if (isModKey(e)) {
      s.toggleMark(side, index, false);
    } else if (e.shiftKey) {
      s.markRange(side, index);
    } else {
      s.setCursor(side, index);
      if (e.button === 0) armDragOut(e, side, entry);
    }
  };

  // Finder convention: right-clicking a marked row acts on the whole mark
  // set; right-clicking anything else retargets to that row alone.
  const onContextMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const s = usePanels.getState();
    s.setActiveSide(side);
    if (!s.activeTab(side).marked.has(entry.name)) s.unmarkAll(side);
    s.setCursor(side, index);
    useCtxMenu.getState().open(e.clientX, e.clientY, side, "row");
  };

  const onDoubleClick = () => {
    usePanels.getState().setActiveSide(side);
    actions.enter.run();
  };
  return { onMouseDown, onContextMenu, onDoubleClick };
}

function stateClasses(
  entry: FsEntry,
  isCursor: boolean,
  isMarked: boolean,
  hasMarks: boolean,
  isDropTarget: boolean,
  clipKind: "cut" | "copy" | null,
) {
  return clsx(
    entry.isDir && "dir",
    entry.isSymlink && "symlink",
    entry.isHidden && "hidden-entry",
    isCursor && "cursor",
    isMarked && "marked",
    (isMarked || (isCursor && !hasMarks)) && "sel",
    clipKind === "cut" && "clip-cut",
    clipKind === "copy" && "clip-copy",
    isDropTarget && "drop-target",
  );
}

/** Finder's "not downloaded" mark after a cloud file's name. */
function CloudBadge() {
  const { t } = useTranslation();
  return (
    <span className="cloud-badge" title={t("panel.inCloud")}>
      <Icon of={CloudDownload} size={14} />
    </span>
  );
}

/** Memoized with scalar props only: scrolling re-renders just the rows
 *  entering the viewport, a cursor move just the two rows that changed. */
const Row = memo(function Row({
  entry,
  index,
  side,
  brief,
  top,
  height,
  isCursor,
  isMarked,
  hasMarks,
  isDropTarget,
  clipKind,
}: {
  entry: FsEntry;
  index: number;
  side: Side;
  brief: boolean;
  top: number;
  height: number;
  isCursor: boolean;
  isMarked: boolean;
  hasMarks: boolean;
  isDropTarget: boolean;
  clipKind: "cut" | "copy" | null;
}) {
  const { onMouseDown, onContextMenu, onDoubleClick } = entryHandlers(side, index, entry);

  return (
    <div
      className={clsx("row", stateClasses(entry, isCursor, isMarked, hasMarks, isDropTarget, clipKind))}
      data-dir-path={entry.isDir ? entry.path : undefined}
      style={{ top, height, gridTemplateColumns: brief ? "1fr" : "var(--grid-cols)" }}
      onMouseDown={onMouseDown}
      onContextMenu={onContextMenu}
      onDoubleClick={onDoubleClick}
    >
      <span className="name">
        <FileIcon isDir={entry.isDir} ext={entry.ext} path={entry.path} />
        <span className="label">{entry.name}</span>
        {entry.isCloud && <CloudBadge />}
      </span>
      {!brief && <span className="ext">{entry.ext}</span>}
      {!brief && (
        <span className="num">{entry.isDir ? "—" : humanSize(entry.size)}</span>
      )}
      {!brief && <span className="num date">{formatDate(entry.modified)}</span>}
    </div>
  );
});

/** Icon view cell: a large icon over a name of up to two lines. */
const Tile = memo(function Tile({
  entry,
  index,
  side,
  top,
  height,
  col,
  cols,
  isCursor,
  isMarked,
  hasMarks,
  isDropTarget,
  clipKind,
}: {
  entry: FsEntry;
  index: number;
  side: Side;
  top: number;
  height: number;
  col: number;
  cols: number;
  isCursor: boolean;
  isMarked: boolean;
  hasMarks: boolean;
  isDropTarget: boolean;
  clipKind: "cut" | "copy" | null;
}) {
  const { onMouseDown, onContextMenu, onDoubleClick } = entryHandlers(side, index, entry);
  return (
    <div
      className={clsx("tile", stateClasses(entry, isCursor, isMarked, hasMarks, isDropTarget, clipKind))}
      data-dir-path={entry.isDir ? entry.path : undefined}
      title={entry.name}
      style={{
        top: top + TILE_GAP / 2,
        left: `calc(${(col * 100) / cols}% + ${TILE_GAP / 2}px)`,
        width: `calc(${100 / cols}% - ${TILE_GAP}px)`,
        height: height - TILE_GAP,
      }}
      onMouseDown={onMouseDown}
      onContextMenu={onContextMenu}
      onDoubleClick={onDoubleClick}
    >
      <FileIcon isDir={entry.isDir} ext={entry.ext} path={entry.path} large />
      <span className="name">
        {entry.isCloud && <CloudBadge />}
        {entry.name}
      </span>
    </div>
  );
});
