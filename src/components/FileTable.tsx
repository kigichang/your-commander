import { useVirtualizer } from "@tanstack/react-virtual";
import clsx from "clsx";
import { useEffect, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";
import type { FsEntry } from "../ipc/types";
import { formatDate, humanSize } from "../lib/format";
import { parentPath } from "../lib/path";
import { visibleEntries } from "../lib/selectors";
import { actions } from "../keymap/actions";
import type { Side, SortKey, TabState, ViewMode } from "../store/panels";
import { usePanels } from "../store/panels";
import { useSettings } from "../store/settings";
import { FileIcon } from "./FileIcon";

const ROW_HEIGHT = 26;

const COLUMNS: Array<{ key: SortKey; label: string; className?: string }> = [
  { key: "name", label: "list.name" },
  { key: "ext", label: "list.ext" },
  { key: "size", label: "list.size", className: "num" },
  { key: "modified", label: "list.date", className: "num" },
];

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

  return (
    <div className="ftable">
      {!brief && (
        <div className="thead">
          {COLUMNS.map((c) => (
            <button
              key={c.key}
              className={clsx(c.className, tab.sortKey === c.key && "on")}
              onClick={() => usePanels.getState().setSort(side, c.key)}
            >
              {t(c.label)}
              {tab.sortKey === c.key ? (tab.sortDir === 1 ? " ▲" : " ▼") : ""}
            </button>
          ))}
        </div>
      )}
      {parentPath(tab.path) && (
        <div
          className="row parent"
          title=".."
          onDoubleClick={() => {
            usePanels.getState().setActiveSide(side);
            usePanels.getState().up(side);
          }}
        >
          <span className="name">
            <span className="icon">▴</span>..
          </span>
        </div>
      )}
      <div className="tbody" ref={scrollRef}>
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
}: {
  entry: FsEntry;
  index: number;
  side: Side;
  tab: TabState;
  brief: boolean;
  top: number;
}) {
  const isCursor = index === tab.cursor;
  const isMarked = tab.marked.has(entry.name);

  const onMouseDown = (e: React.MouseEvent) => {
    const s = usePanels.getState();
    if (e.metaKey) {
      s.toggleMark(side, index, false);
    } else if (e.shiftKey) {
      s.markRange(side, index);
    } else {
      s.setCursor(side, index);
    }
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
      )}
      style={brief ? { top, gridTemplateColumns: "1fr" } : { top }}
      onMouseDown={onMouseDown}
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
