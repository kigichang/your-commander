import { useVirtualizer } from "@tanstack/react-virtual";
import clsx from "clsx";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import * as ipc from "../ipc/commands";
import { humanSize } from "../lib/format";
import type { Dialog } from "../store/dialogs";
import { useDialogs } from "../store/dialogs";

const CMP_MAX = 4 * 1024 * 1024;
const ROW_HEIGHT = 19;

type Row = {
  kind: "same" | "mod" | "del" | "add";
  ln?: number;
  lt?: string;
  rn?: number;
  rt?: string;
};

function isBinary(buf: ArrayBuffer): boolean {
  return new Uint8Array(buf, 0, Math.min(buf.byteLength, 8192)).includes(0);
}

function bytesEqual(a: ArrayBuffer, b: ArrayBuffer): boolean {
  if (a.byteLength !== b.byteLength) return false;
  const va = new Uint8Array(a);
  const vb = new Uint8Array(b);
  for (let i = 0; i < va.length; i++) if (va[i] !== vb[i]) return false;
  return true;
}

/** Classic LCS backtrack over the changed window. */
function lcsOps(a: string[], b: string[]): Array<{ t: "same" | "del" | "add"; s: string }> {
  const n = a.length;
  const m = b.length;
  const w = m + 1;
  const dp = new Uint32Array((n + 1) * w);
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i * w + j] =
        a[i] === b[j]
          ? dp[(i + 1) * w + j + 1] + 1
          : Math.max(dp[(i + 1) * w + j], dp[i * w + j + 1]);
    }
  }
  const ops: Array<{ t: "same" | "del" | "add"; s: string }> = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      ops.push({ t: "same", s: a[i] });
      i++;
      j++;
    } else if (dp[(i + 1) * w + j] >= dp[i * w + j + 1]) {
      ops.push({ t: "del", s: a[i++] });
    } else {
      ops.push({ t: "add", s: b[j++] });
    }
  }
  while (i < n) ops.push({ t: "del", s: a[i++] });
  while (j < m) ops.push({ t: "add", s: b[j++] });
  return ops;
}

function diffRows(a: string[], b: string[]): Row[] {
  // Trim the common prefix/suffix so the quadratic LCS only sees the window
  // that actually changed.
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start++;
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
    endA--;
    endB--;
  }
  const midA = a.slice(start, endA);
  const midB = b.slice(start, endB);
  const ops =
    midA.length * midB.length <= 4_000_000
      ? lcsOps(midA, midB)
      : [
          // Window too large to align line-by-line: show it as one block.
          ...midA.map((s) => ({ t: "del" as const, s })),
          ...midB.map((s) => ({ t: "add" as const, s })),
        ];

  const rows: Row[] = [];
  let ln = 1;
  let rn = 1;
  const same = (text: string) => rows.push({ kind: "same", ln: ln++, lt: text, rn: rn++, rt: text });
  for (let i = 0; i < start; i++) same(a[i]);
  let k = 0;
  while (k < ops.length) {
    if (ops[k].t === "same") {
      same(ops[k].s);
      k++;
      continue;
    }
    // Pair each run of deletions with the additions that replace it.
    const dels: string[] = [];
    const adds: string[] = [];
    while (k < ops.length && ops[k].t !== "same") {
      (ops[k].t === "del" ? dels : adds).push(ops[k].s);
      k++;
    }
    for (let x = 0; x < Math.max(dels.length, adds.length); x++) {
      const lt = x < dels.length ? dels[x] : undefined;
      const rt = x < adds.length ? adds[x] : undefined;
      rows.push({
        kind: lt !== undefined && rt !== undefined ? "mod" : lt !== undefined ? "del" : "add",
        ln: lt !== undefined ? ln++ : undefined,
        lt,
        rn: rt !== undefined ? rn++ : undefined,
        rt,
      });
    }
  }
  for (let i = endA; i < a.length; i++) same(a[i]);
  return rows;
}

type State =
  | { phase: "loading" }
  | { phase: "error"; msg: string }
  | { phase: "binary"; equal: boolean; lsize: number; rsize: number }
  | { phase: "text"; rows: Row[]; diffs: number };

export function CompareDialog({ d }: { d: Extract<Dialog, { type: "compare" }> }) {
  const { t } = useTranslation();
  const close = useDialogs((s) => s.close);
  const [state, setState] = useState<State>({ phase: "loading" });
  const [diffOnly, setDiffOnly] = useState(false);

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

  useEffect(() => {
    let stale = false;
    void (async () => {
      try {
        const [la, ra] = await Promise.all([
          ipc.readPreview(d.left.location, d.left.path, CMP_MAX),
          ipc.readPreview(d.right.location, d.right.path, CMP_MAX),
        ]);
        if (stale) return;
        if (isBinary(la) || isBinary(ra)) {
          setState({
            phase: "binary",
            equal: bytesEqual(la, ra),
            lsize: la.byteLength,
            rsize: ra.byteLength,
          });
          return;
        }
        const dec = new TextDecoder();
        const rows = diffRows(dec.decode(la).split("\n"), dec.decode(ra).split("\n"));
        setState({
          phase: "text",
          rows,
          diffs: rows.reduce((n, r) => n + (r.kind !== "same" ? 1 : 0), 0),
        });
      } catch (e) {
        if (!stale) setState({ phase: "error", msg: String(e) });
      }
    })();
    return () => {
      stale = true;
    };
  }, [d]);

  const rows = state.phase === "text" ? state.rows : [];
  const visible = useMemo(
    () => (diffOnly ? rows.filter((r) => r.kind !== "same") : rows),
    [rows, diffOnly],
  );

  const scrollRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: visible.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 20,
  });

  // Land on the first difference once the diff is ready.
  const diffs = state.phase === "text" ? state.diffs : 0;
  useEffect(() => {
    if (state.phase !== "text" || diffOnly) return;
    const first = state.rows.findIndex((r) => r.kind !== "same");
    if (first > 0) virtualizer.scrollToIndex(first, { align: "center" });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state.phase]);

  return (
    <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && close()}>
      <div className="dialog wide cmp">
        <div className="cmp-head">
          <span className="cmp-file" title={d.left.label}>
            {d.left.label}
          </span>
          <span className="cmp-file" title={d.right.label}>
            {d.right.label}
          </span>
        </div>
        {state.phase === "loading" && <div className="cmp-note">{t("sync.comparing")}</div>}
        {state.phase === "error" && <div className="cmp-note error">{state.msg}</div>}
        {state.phase === "binary" && (
          <div className="cmp-note">
            {state.equal
              ? t("compare.identical")
              : t("compare.binaryDiffer", {
                  a: humanSize(state.lsize),
                  b: humanSize(state.rsize),
                })}
          </div>
        )}
        {state.phase === "text" &&
          (diffs === 0 ? (
            <div className="cmp-note">{t("compare.identical")}</div>
          ) : (
            <div className="cmp-body" ref={scrollRef}>
              <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
                {virtualizer.getVirtualItems().map((item) => {
                  const r = visible[item.index];
                  return (
                    <div
                      key={item.key}
                      className={clsx("cmp-row", r.kind)}
                      style={{ top: item.start }}
                    >
                      <span className="num">{r.ln ?? ""}</span>
                      <span className={clsx("txt", "l", r.lt === undefined && "void")}>{r.lt}</span>
                      <span className="num">{r.rn ?? ""}</span>
                      <span className={clsx("txt", "r", r.rt === undefined && "void")}>{r.rt}</span>
                    </div>
                  );
                })}
              </div>
            </div>
          ))}
        <div className="buttons cmp-foot">
          {state.phase === "text" && diffs > 0 && (
            <>
              <span className="cmp-count">{t("compare.diffs", { count: diffs })}</span>
              <label className="check">
                <input
                  type="checkbox"
                  checked={diffOnly}
                  onChange={(e) => setDiffOnly(e.target.checked)}
                />
                {t("sync.diffOnly")}
              </label>
            </>
          )}
          <button className="btn primary" onClick={close}>
            {t("settings.done")}
          </button>
        </div>
      </div>
    </div>
  );
}
