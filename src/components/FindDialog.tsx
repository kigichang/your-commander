import { useVirtualizer } from "@tanstack/react-virtual";
import clsx from "clsx";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import * as ipc from "../ipc/commands";
import type { FsEntry } from "../ipc/types";
import { useScaled } from "../lib/fontScale";
import { humanSize } from "../lib/format";
import { parentPath } from "../lib/path";
import type { Dialog } from "../store/dialogs";
import { useDialogs } from "../store/dialogs";
import { useOps } from "../store/ops";
import { usePanels } from "../store/panels";
import { useSettings } from "../store/settings";
import { Overlay } from "./Overlay";
import { displayPath, expandHome } from "../lib/path";

const DAY = 24 * 60 * 60 * 1000;
const MODIFIED = [
  { key: "any", ms: null },
  { key: "1d", ms: DAY },
  { key: "7d", ms: 7 * DAY },
  { key: "30d", ms: 30 * DAY },
  { key: "1y", ms: 365 * DAY },
] as const;
const MB = 1024 * 1024;
const SIZES = [
  { key: "any", min: null, max: null },
  { key: "lt1m", min: null, max: MB },
  { key: "gt1m", min: MB, max: null },
  { key: "gt10m", min: 10 * MB, max: null },
  { key: "gt100m", min: 100 * MB, max: null },
] as const;

export function FindDialog({ d }: { d: Extract<Dialog, { type: "find" }> }) {
  const { t } = useTranslation();
  const close = useDialogs((s) => s.close);
  const home = useSettings((s) => s.home);
  const [mask, setMask] = useState("*.*");
  const [inDir, setInDir] = useState(
    d.location.kind === "local" ? displayPath(d.dir, home) : d.dir,
  );
  const [content, setContent] = useState("");
  const [modified, setModified] = useState<(typeof MODIFIED)[number]["key"]>("any");
  const [size, setSize] = useState<(typeof SIZES)[number]["key"]>("any");
  const [results, setResults] = useState<FsEntry[]>([]);
  const [scanning, setScanning] = useState("");
  const [running, setRunning] = useState(false);
  const [selected, setSelected] = useState(-1);
  const buffer = useRef<FsEntry[]>([]);
  const findIdRef = useRef<number | null>(null);
  /** Bumped by every start, stop, and unmount. A search started under an
   *  older number is cancelled as soon as its id arrives (the dialog closed
   *  or Stop was hit while start_find was in flight), and its events are
   *  dropped so two searches never mix results. */
  const runRef = useRef(0);
  /** A search is starting or running (a ref, so a quick double press sees
   *  it before the re-render). */
  const busyRef = useRef(false);
  const scrollRef = useRef<HTMLDivElement>(null);

  // Stream batching: found entries pile into a ref and flush on a timer.
  useEffect(() => {
    const timer = setInterval(() => {
      if (buffer.current.length > 0) {
        const batch = buffer.current.splice(0);
        setResults((r) => [...r, ...batch]);
      }
    }, 120);
    return () => clearInterval(timer);
  }, []);

  // Cancel a still-running search when the dialog unmounts.
  useEffect(
    () => () => {
      runRef.current++;
      if (findIdRef.current !== null) void ipc.cancelFind(findIdRef.current);
    },
    [],
  );

  const stop = () => {
    runRef.current++;
    busyRef.current = false;
    if (findIdRef.current !== null) void ipc.cancelFind(findIdRef.current);
    findIdRef.current = null;
    setRunning(false);
    setScanning("");
  };

  const search = async () => {
    if (busyRef.current) return stop();
    const run = ++runRef.current;
    const live = () => runRef.current === run;
    busyRef.current = true;
    setRunning(true);
    // Done can arrive before start_find's reply (a tiny folder): then the
    // id must not be recorded as a running search.
    let finished = false;
    setResults([]);
    setSelected(-1);
    buffer.current = [];
    const mod = MODIFIED.find((m) => m.key === modified)!;
    const sz = SIZES.find((s) => s.key === size)!;
    try {
      const id = await ipc.startFind(
        d.location,
        expandHome(inDir.trim(), home) || d.dir,
        {
          mask: mask.trim() || "*.*",
          content: content.trim() || null,
          modifiedAfter: mod.ms === null ? null : Date.now() - mod.ms,
          minSize: sz.min,
          maxSize: sz.max,
        },
        (event) => {
          if (!live()) return;
          if (event.type === "found") {
            buffer.current.push(event.entry);
          } else if (event.type === "scanning") {
            setScanning(event.dir);
          } else {
            finished = true;
            findIdRef.current = null;
            busyRef.current = false;
            setRunning(false);
            setScanning("");
            if (event.error) useOps.getState().toast(event.error, "error");
          }
        },
      );
      if (!live()) {
        void ipc.cancelFind(id);
        return;
      }
      if (!finished) findIdRef.current = id;
    } catch (e) {
      if (!live()) return;
      busyRef.current = false;
      setRunning(false);
      useOps.getState().toast(String(e), "error");
    }
  };

  const goTo = (entry: FsEntry | undefined) => {
    if (!entry) return;
    const dir = parentPath(entry.path);
    if (!dir) return;
    stop();
    close();
    void usePanels
      .getState()
      .revealEntry(usePanels.getState().activeSide, dir, entry.name, d.location);
  };

  const rowHeight = useScaled(24);
  const virtualizer = useVirtualizer({
    count: results.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => rowHeight,
    overscan: 10,
  });

  return (
    <Overlay
      onClose={() => {
        stop();
        close();
      }}
      className="wide sync"
    >
      <h2>{t("find.title")}</h2>
      <div className="mr-form">
        <div className="field grow">
          <label>{t("find.name")}</label>
          <input
            value={mask}
            spellCheck={false}
            onChange={(e) => setMask(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && void search()}
          />
        </div>
        <div className="field grow">
          <label>{t("find.in")}</label>
          <input
            value={inDir}
            spellCheck={false}
            onChange={(e) => setInDir(e.target.value)}
          />
        </div>
      </div>
      <div className="mr-form">
        <div className="field grow">
          <label>{t("find.containing")}</label>
          <input
            value={content}
            spellCheck={false}
            onChange={(e) => setContent(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && void search()}
          />
        </div>
        <div className="field">
          <label>{t("find.modified")}</label>
          <select value={modified} onChange={(e) => setModified(e.target.value as never)}>
            {MODIFIED.map((m) => (
              <option key={m.key} value={m.key}>
                {t(`find.mod_${m.key}`)}
              </option>
            ))}
          </select>
        </div>
        <div className="field">
          <label>{t("find.size")}</label>
          <select value={size} onChange={(e) => setSize(e.target.value as never)}>
            {SIZES.map((s) => (
              <option key={s.key} value={s.key}>
                {t(`find.size_${s.key}`)}
              </option>
            ))}
          </select>
        </div>
        <button className="btn primary find-go" onClick={() => void search()}>
          {running ? t("find.stop") : `${t("find.search")} ↵`}
        </button>
      </div>
      <div className="find-status">
        {t("find.results", { count: results.length })}
        {running && scanning && (
          <span className="scanning"> · {scanning}</span>
        )}
      </div>
      <div className="sync-rows" ref={scrollRef}>
        {results.length === 0 ? (
          <div className="none">
            {running ? t("find.searching") : t("find.hint")}
          </div>
        ) : (
          <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
            {virtualizer.getVirtualItems().map((item) => {
              const entry = results[item.index];
              return (
                <div
                  key={entry.path}
                  className={clsx("find-row", item.index === selected && "on")}
                  style={{ top: item.start, height: rowHeight }}
                  onMouseDown={() => setSelected(item.index)}
                  onDoubleClick={() => goTo(entry)}
                >
                  <span className="fname">{entry.name}</span>
                  <span className="fdir">
                    {displayPath(parentPath(entry.path) ?? "", home)}
                  </span>
                  <span className="fsize">{humanSize(entry.size)}</span>
                </div>
              );
            })}
          </div>
        )}
      </div>
      <div className="buttons">
        <button className="btn" onClick={() => (stop(), close())}>
          {t("dialog.cancel")}
        </button>
        <button
          className="btn primary"
          disabled={selected < 0}
          onClick={() => goTo(results[selected])}
        >
          {t("find.goTo")}
        </button>
      </div>
    </Overlay>
  );
}
