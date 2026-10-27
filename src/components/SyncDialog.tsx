import { useVirtualizer } from "@tanstack/react-virtual";
import clsx from "clsx";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import * as ipc from "../ipc/commands";
import type { SyncDirection, SyncRow } from "../ipc/types";
import { formatDate } from "../lib/format";
import type { Dialog } from "../store/dialogs";
import { useDialogs } from "../store/dialogs";
import { useOps } from "../store/ops";

type Action = SyncDirection | "skip";

const CYCLE: Record<Action, Action> = { ltr: "rtl", rtl: "skip", skip: "ltr" };
const GLYPH: Record<Action, string> = { ltr: "→", rtl: "←", skip: "·" };

function defaultAction(row: SyncRow): Action {
  if (row.left && !row.right) return "ltr";
  if (!row.left && row.right) return "rtl";
  if (row.equal) return "skip";
  const l = row.left?.modified ?? 0;
  const r = row.right?.modified ?? 0;
  return l >= r ? "ltr" : "rtl";
}

export function SyncDialog({ d }: { d: Extract<Dialog, { type: "sync" }> }) {
  const { t } = useTranslation();
  const close = useDialogs((s) => s.close);
  const bothLocal = d.left.kind === "local" && d.right.kind === "local";
  const [byContent, setByContent] = useState(false);
  const [diffOnly, setDiffOnly] = useState(true);
  const [comparing, setComparing] = useState(false);
  const [rows, setRows] = useState<SyncRow[] | null>(null);
  const [actions, setActions] = useState<Map<string, Action>>(new Map());
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        close();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [close]);

  const compare = async () => {
    setComparing(true);
    try {
      const result = await ipc.syncCompare(
        d.left,
        d.leftDir,
        d.right,
        d.rightDir,
        byContent && bothLocal,
      );
      setRows(result);
      setActions(new Map(result.map((r) => [r.rel, defaultAction(r)])));
    } catch (e) {
      useOps.getState().toast(String(e), "error");
    } finally {
      setComparing(false);
    }
  };

  const shown = useMemo(
    () => (rows ?? []).filter((r) => !diffOnly || !r.equal),
    [rows, diffOnly],
  );

  const virtualizer = useVirtualizer({
    count: shown.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 24,
    overscan: 10,
  });

  const stats = useMemo(() => {
    let ltr = 0;
    let rtl = 0;
    let eq = 0;
    let ne = 0;
    for (const row of rows ?? []) {
      if (row.equal) eq++;
      else if (row.left && row.right) ne++;
      const a = actions.get(row.rel);
      if (a === "ltr") ltr++;
      else if (a === "rtl") rtl++;
    }
    return { ltr, rtl, eq, ne, total: ltr + rtl };
  }, [rows, actions]);

  const cycle = (rel: string) =>
    setActions((m) => {
      const next = new Map(m);
      next.set(rel, CYCLE[m.get(rel) ?? "skip"]);
      return next;
    });

  const synchronize = () => {
    if (!rows || stats.total === 0) return;
    const items = rows
      .map((r) => ({ rel: r.rel, direction: actions.get(r.rel) }))
      .filter((i): i is { rel: string; direction: SyncDirection } => i.direction !== "skip" && i.direction !== undefined);
    close();
    useOps.getState().run(
      t("sync.label"),
      {
        op: "sync",
        left: d.left,
        leftDir: d.leftDir,
        right: d.right,
        rightDir: d.rightDir,
        items,
      },
      (done) => useOps.getState().toast(t("sync.done", { count: done.count })),
    );
  };

  return (
    <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && close()}>
      <div className="dialog wide sync">
        <h2>{t("sync.title")}</h2>
        <div className="sync-paths">
          <span>{d.leftLabel}</span>
          <span className="arrow">⇄</span>
          <span>{d.rightLabel}</span>
        </div>
        <div className="sync-bar">
          <button className="btn primary" disabled={comparing} onClick={() => void compare()}>
            {comparing ? t("sync.comparing") : t("sync.compare")}
          </button>
          <label className="check" title={bothLocal ? "" : t("sync.contentLocalOnly")}>
            <input
              type="checkbox"
              disabled={!bothLocal}
              checked={byContent && bothLocal}
              onChange={(e) => setByContent(e.target.checked)}
            />
            {t("sync.byContent")}
          </label>
          <label className="check">
            <input
              type="checkbox"
              checked={diffOnly}
              onChange={(e) => setDiffOnly(e.target.checked)}
            />
            {t("sync.diffOnly")}
          </label>
          {rows && (
            <span className="sync-stats">
              → {stats.ltr} · ＝ {stats.eq} · ≠ {stats.ne} · ← {stats.rtl}
            </span>
          )}
        </div>
        <div className="sync-rows" ref={scrollRef}>
          {rows === null ? (
            <div className="none">{t("sync.hint")}</div>
          ) : shown.length === 0 ? (
            <div className="none">{t("sync.allEqual")}</div>
          ) : (
            <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
              {virtualizer.getVirtualItems().map((item) => {
                const row = shown[item.index];
                const action = actions.get(row.rel) ?? "skip";
                return (
                  <div
                    key={row.rel}
                    className={clsx("sync-row", !row.left || !row.right ? "single" : "")}
                    style={{ top: item.start }}
                  >
                    <span className={clsx("rel", !row.left && "missing")}>
                      {row.left ? row.rel : "—"}
                    </span>
                    <span className="date">
                      {row.left ? formatDate(row.left.modified) : ""}
                    </span>
                    <button
                      className={clsx("op", action)}
                      title="→ / ← / ·"
                      onClick={() => cycle(row.rel)}
                    >
                      {GLYPH[action]}
                    </button>
                    <span className="date">
                      {row.right ? formatDate(row.right.modified) : ""}
                    </span>
                    <span className={clsx("rel right", !row.right && "missing")}>
                      {row.right ? row.rel : "—"}
                    </span>
                  </div>
                );
              })}
            </div>
          )}
        </div>
        <div className="buttons">
          <span className="sync-actions">
            {rows ? t("sync.actions", { count: stats.total }) : ""}
          </span>
          <button className="btn" onClick={close}>
            {t("dialog.cancel")}
          </button>
          <button className="btn primary" disabled={!rows || stats.total === 0} onClick={synchronize}>
            {t("sync.synchronize")}
          </button>
        </div>
      </div>
    </div>
  );
}
