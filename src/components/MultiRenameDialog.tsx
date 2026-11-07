import { useVirtualizer } from "@tanstack/react-virtual";
import clsx from "clsx";
import { TriangleAlert } from "lucide-react";
import { useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import * as ipc from "../ipc/commands";
import type { Location } from "../ipc/types";
import { useScaled } from "../lib/fontScale";
import type { CaseMode, RenameRules } from "../lib/renameMask";
import { DEFAULT_RULES, previewRenames } from "../lib/renameMask";
import type { Dialog } from "../store/dialogs";
import { Icon } from "./icons";
import { Overlay } from "./Overlay";
import { useDialogs } from "../store/dialogs";
import { useOps } from "../store/ops";
import { usePanels } from "../store/panels";

/** The last applied batch, kept so "Undo last" can reverse it. */
let lastBatch: {
  location: Location;
  items: Array<{ path: string; newName: string }>;
} | null = null;

const CASES: CaseMode[] = ["unchanged", "lower", "upper"];

export function MultiRenameDialog({ d }: { d: Extract<Dialog, { type: "multiRename" }> }) {
  const { t } = useTranslation();
  const close = useDialogs((s) => s.close);
  const [rules, setRules] = useState<RenameRules>(DEFAULT_RULES);
  const [hasUndo, setHasUndo] = useState(lastBatch !== null);
  const patch = (p: Partial<RenameRules>) => setRules((r) => ({ ...r, ...p }));

  const tab = usePanels((s) => s.activeTab(d.side));
  const existing = useMemo(
    () => new Set(tab.entries.map((e) => e.name)),
    [tab.entries],
  );
  const rows = useMemo(
    () => previewRenames(d.items, rules, existing),
    [d.items, rules, existing],
  );
  const invalid = rows.some((r) => r.problem !== null);
  // Thousands of selected files: only the rows in view are rendered, so
  // each keystroke in the rules re-renders a screenful, not the batch.
  const previewRef = useRef<HTMLDivElement>(null);
  const rowHeight = useScaled(20);
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => previewRef.current,
    estimateSize: () => rowHeight,
    overscan: 10,
  });
  const changed = rows.filter((r) => r.changed && r.problem === null);

  const apply = async () => {
    if (invalid || changed.length === 0) return;
    const items = changed.map((r) => ({ path: r.entry.path, newName: r.newName }));
    try {
      const count = await ipc.batchRename(tab.location, items);
      // Remember the reverse mapping for undo: new path → old name.
      const dir = (p: string) => p.slice(0, p.lastIndexOf("/") + 1);
      lastBatch = {
        location: tab.location,
        items: changed.map((r) => ({
          path: dir(r.entry.path) + r.newName,
          newName: r.entry.name,
        })),
      };
      close();
      useOps.getState().toast(t("rename.done", { count }));
      void usePanels.getState().refresh(d.side, { silent: true });
    } catch (e) {
      useOps.getState().toast(String(e), "error");
    }
  };

  const undo = async () => {
    if (!lastBatch) return;
    try {
      const count = await ipc.batchRename(lastBatch.location, lastBatch.items);
      lastBatch = null;
      setHasUndo(false);
      useOps.getState().toast(t("rename.undone", { count }));
      void usePanels.getState().refresh(0, { silent: true });
      void usePanels.getState().refresh(1, { silent: true });
    } catch (e) {
      useOps.getState().toast(String(e), "error");
    }
  };

  return (
    <Overlay onClose={close} className="wide">
      <h2>{t("rename.title", { count: d.items.length })}</h2>
      <div className="mr-form">
        <div className="field grow">
          <label>{t("rename.nameMask")}</label>
          <input
            value={rules.nameMask}
            spellCheck={false}
            onChange={(e) => patch({ nameMask: e.target.value })}
          />
        </div>
        <div className="field">
          <label>{t("rename.extension")}</label>
          <input
            value={rules.extMask}
            spellCheck={false}
            onChange={(e) => patch({ extMask: e.target.value })}
          />
        </div>
        <div className="field grow">
          <label>{t("rename.find")}</label>
          <input
            value={rules.find}
            spellCheck={false}
            onChange={(e) => patch({ find: e.target.value })}
          />
        </div>
        <div className="field grow">
          <label>{t("rename.replaceWith")}</label>
          <input
            value={rules.replace}
            spellCheck={false}
            onChange={(e) => patch({ replace: e.target.value })}
          />
        </div>
      </div>
      <div className="mr-form">
        <label className="check">
          <input
            type="checkbox"
            checked={rules.regex}
            onChange={(e) => patch({ regex: e.target.checked })}
          />
          {t("rename.regex")}
        </label>
        <div className="field inline">
          <label>{t("rename.caseLabel")}</label>
          <div className="seg">
            {CASES.map((c) => (
              <button
                key={c}
                className={rules.caseMode === c ? "on" : ""}
                onClick={() => patch({ caseMode: c })}
              >
                {t(`rename.case_${c}`)}
              </button>
            ))}
          </div>
        </div>
        <div className="field inline">
          <label>{t("rename.counterStart")}</label>
          <input
            className="num-in"
            value={rules.counterStart}
            onChange={(e) => patch({ counterStart: Number(e.target.value) || 0 })}
          />
        </div>
        <div className="field inline">
          <label>{t("rename.step")}</label>
          <input
            className="num-in"
            value={rules.counterStep}
            onChange={(e) => patch({ counterStep: Number(e.target.value) || 1 })}
          />
        </div>
      </div>
      <div className="mr-hint">[N] [E] [C:3] [YMD] [Y] [M] [D]</div>
      <div className="mr-preview" ref={previewRef}>
        <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
          {virtualizer.getVirtualItems().map((item) => {
            const r = rows[item.index];
            return (
              <div
                key={r.entry.path}
                className={clsx("mr-row", r.problem && "bad")}
                style={{ top: item.start, height: rowHeight }}
              >
                <span className="old">{r.entry.name}</span>
                <span className="arrow">→</span>
                <span className={clsx("new", !r.changed && "same")}>
                  {r.newName}
                  {r.problem && (
                    <>
                      {"  "}
                      <Icon of={TriangleAlert} size={13} /> {t(r.problem)}
                    </>
                  )}
                </span>
              </div>
            );
          })}
        </div>
      </div>
      <div className="buttons">
        {hasUndo && (
          <button className="btn" onClick={() => void undo()}>
            {t("rename.undoLast")}
          </button>
        )}
        <button className="btn" onClick={close}>
          {t("dialog.cancel")}
        </button>
        <button
          className="btn primary"
          disabled={invalid || changed.length === 0}
          onClick={() => void apply()}
        >
          {t("rename.apply", { count: changed.length })}
        </button>
      </div>
    </Overlay>
  );
}
