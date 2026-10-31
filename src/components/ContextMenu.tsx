import { Fragment, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { create } from "zustand";
import * as ipc from "../ipc/commands";
import type { ActionId } from "../keymap/actions";
import { actions, ARCHIVE_RE } from "../keymap/actions";
import { prettyChord } from "../keymap/keymap";
import { targetEntries, visibleEntries } from "../lib/selectors";
import type { ExternalTool, OpenApp } from "../ipc/types";
import { useClipboard } from "../store/clipboard";
import { useOps } from "../store/ops";
import type { Side } from "../store/panels";
import { usePanels } from "../store/panels";
import { useKeymap } from "../store/keymapStore";
import { useSettings } from "../store/settings";

interface CtxState {
  menu: { x: number; y: number; side: Side; kind: "row" | "empty" } | null;
  open: (x: number, y: number, side: Side, kind: "row" | "empty") => void;
  close: () => void;
}

export const useCtxMenu = create<CtxState>((set) => ({
  menu: null,
  open: (x, y, side, kind) => set({ menu: { x, y, side, kind } }),
  close: () => set({ menu: null }),
}));

type Item = {
  label: string;
  chord?: string;
  run?: () => void;
  /** Flyout submenu; `run` is ignored when present. */
  sub?: Array<{ label: string; run: () => void }>;
};
type Section = Item[];

/** Every selected entry must match the tool's filter, so the tool receives
 *  only files its label promises to handle. */
function toolMatches(tool: ExternalTool, targets: { name: string; isDir: boolean }[]) {
  const exts = tool.extensions
    .split(",")
    .map((s) => s.trim().toLowerCase().replace(/^\./, ""))
    .filter(Boolean);
  return (
    targets.length > 0 &&
    targets.every((e) =>
      e.isDir
        ? tool.dirs
        : exts.length === 0 || exts.some((x) => e.name.toLowerCase().endsWith("." + x)),
    )
  );
}

export function ContextMenu() {
  const menu = useCtxMenu((s) => s.menu);
  if (!menu) return null;
  return <Menu key={`${menu.x},${menu.y}`} {...menu} />;
}

function Menu({ x, y, side, kind }: NonNullable<CtxState["menu"]>) {
  const { t } = useTranslation();
  const close = useCtxMenu((s) => s.close);
  const keymap = useKeymap((s) => s.map);
  const showHidden = useSettings((s) => s.showHidden);
  const config = useSettings((s) => s.config);
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x, y });

  // The cursor/marks were already placed by the right-click handler.
  const tab = usePanels((s) => s.panels[side].tabs[s.panels[side].active]);

  // "Open With" candidates for the file under the cursor (local files only).
  const [apps, setApps] = useState<OpenApp[]>([]);
  useEffect(() => {
    if (kind !== "row" || tab.location.kind !== "local") return;
    const entry = visibleEntries(tab, showHidden)[tab.cursor];
    if (!entry || entry.isDir) return;
    let stale = false;
    void ipc
      .appsForPath(entry.path)
      .then((v) => {
        if (!stale) setApps(v);
      })
      .catch(() => {});
    return () => {
      stale = true;
    };
    // The menu remounts per open (keyed on x,y), so once is enough.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const sections = useMemo<Section[]>(() => {
    const chordOf = (id: ActionId) => {
      const chord = Object.entries(keymap).find(([, a]) => a === id)?.[0];
      return chord ? prettyChord(chord) : undefined;
    };
    const act = (id: ActionId, label?: string): Item => ({
      label: label ?? t(`action.${id}`),
      chord: chordOf(id),
      run: () => actions[id].run(),
    });

    const loc = tab.location.kind;
    if (kind === "empty") {
      const hasClip = useClipboard.getState().clip !== null;
      return [
        [
          ...(hasClip && loc !== "archive" ? [act("clipPaste")] : []),
          ...(loc !== "archive" ? [act("newFolder")] : []),
          ...(loc === "local"
            ? [
                {
                  label: t("action.getInfo"),
                  run: () => void ipc.getInfo([tab.path]),
                },
              ]
            : []),
          act("refresh"),
          act("markAll"),
          act("toggleHidden"),
        ],
        ...(loc === "local"
          ? [
              [
                act("newTerminalHere"),
                ...(config?.menu_vscode ? [act("newVscodeHere")] : []),
                ...(config?.menu_zed ? [act("newZedHere")] : []),
              ],
            ]
          : []),
      ];
    }

    const visible = visibleEntries(tab, showHidden);
    const entry = visible[tab.cursor];
    if (!entry) return [];
    const targets = targetEntries(tab, visible);
    const n = targets.length;
    const archives = targets.filter((e) => !e.isDir && ARCHIVE_RE.test(e.name)).length;
    const many = (label: string) => (n > 1 ? `${label} (${n})` : label);
    const s = usePanels.getState();

    const open: Section = [
      act("enter", t("ctx.open")),
      ...(loc === "local" && !entry.isDir && apps.length > 0
        ? [
            {
              label: t("ctx.openWith"),
              sub: apps.map((a) => ({
                label: a.default ? t("ctx.defaultApp", { name: a.name }) : a.name,
                run: () =>
                  void ipc
                    .openWith(a.path, targets.filter((e) => !e.isDir).map((e) => e.path))
                    .catch((err) => useOps.getState().toast(String(err), "error")),
              })),
            },
          ]
        : []),
      ...(entry.isDir
        ? [
            {
              label: t("ctx.openInNewTab"),
              run: () => s.openTab(side, entry.path, tab.location, tab.remoteLabel),
            },
            {
              label: t("ctx.openOtherPane"),
              run: () =>
                void s.navigate(s.otherSide(side), entry.path, tab.location, tab.remoteLabel),
            },
          ]
        : []),
      act("quickView"),
      ...(loc === "local"
        ? [
            {
              label: t("quickview.reveal"),
              run: () => void ipc.revealInFinder(entry.path),
            },
            { ...act("getInfo"), label: many(t("action.getInfo")) },
          ]
        : []),
    ];

    const work: Section = [
      { ...act("copy"), label: many(t("action.copy")) },
      ...(loc !== "archive" ? [{ ...act("move"), label: many(t("action.move")) }] : []),
      ...(loc !== "archive" ? [act("renameOne")] : []),
      ...(loc !== "archive" && n > 1 ? [act("multiRename")] : []),
      ...(loc === "local" ? [{ ...act("compress"), label: many(t("action.compress")) }] : []),
      ...(loc === "local" && archives > 0
        ? [
            {
              ...act("extract"),
              label:
                archives > 1
                  ? `${t("action.extract")} (${archives})`
                  : t("action.extract"),
            },
          ]
        : []),
      ...(!entry.isDir ? [act("compareFiles")] : []),
    ];

    const hasClip = useClipboard.getState().clip !== null;
    const clipboard: Section = [
      { ...act("clipCopy"), label: many(t("action.clipCopy")) },
      ...(loc !== "archive" ? [{ ...act("clipCut"), label: many(t("action.clipCut")) }] : []),
      ...(hasClip && loc !== "archive" ? [act("clipPaste")] : []),
      { ...act("copyPath"), label: many(t("action.copyPath")) },
    ];

    const tools: Section =
      loc === "local"
        ? (config?.external_tools ?? [])
            .filter((tl) => tl.name && tl.app && toolMatches(tl, targets))
            .map((tl) => ({
              label: many(tl.name),
              run: () =>
                void ipc
                  .openWith(tl.app, targets.map((e) => e.path))
                  .catch((e) => useOps.getState().toast(String(e), "error")),
            }))
        : [];

    const danger: Section = [
      ...(loc === "local" ? [{ ...act("trash"), label: many(t("action.trash")) }] : []),
      ...(loc !== "archive"
        ? [{ ...act("deletePermanent"), label: many(t("action.deletePermanent")) }]
        : []),
    ];

    return [open, work, tools, clipboard, danger].filter((sec) => sec.length > 0);
  }, [tab, side, kind, keymap, showHidden, config, apps, t]);

  // Keep the menu inside the window.
  useLayoutEffect(() => {
    const rect = ref.current?.getBoundingClientRect();
    if (!rect) return;
    setPos({
      x: Math.max(4, Math.min(x, window.innerWidth - rect.width - 4)),
      y: Math.max(4, Math.min(y, window.innerHeight - rect.height - 4)),
    });
  }, [x, y]);

  useLayoutEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) close();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        close();
      }
    };
    const onAway = () => close();
    // Placing the cursor on the clicked row may nudge the virtual list;
    // ignore that initial scroll instead of closing the menu it just opened.
    const openedAt = Date.now();
    const onScroll = () => {
      if (Date.now() - openedAt > 150) close();
    };
    window.addEventListener("mousedown", onDown, true);
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("blur", onAway);
    window.addEventListener("resize", onAway);
    window.addEventListener("scroll", onScroll, true);
    return () => {
      window.removeEventListener("mousedown", onDown, true);
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("blur", onAway);
      window.removeEventListener("resize", onAway);
      window.removeEventListener("scroll", onScroll, true);
    };
  }, [close]);

  if (sections.length === 0) return null;
  return (
    <div ref={ref} className="menu ctx-menu" style={{ left: pos.x, top: pos.y }}>
      {sections.map((section, i) => (
        <Fragment key={i}>
          {i > 0 && <div className="sep" />}
          {section.map((item, j) =>
            item.sub ? (
              <div key={`${j}-${item.label}`} className="item has-sub">
                <span className="ctx-label">{item.label}</span>
                <span className="ctx-chord">▸</span>
                <div className="menu submenu">
                  {item.sub.map((si, k) => (
                    <button
                      key={`${k}-${si.label}`}
                      className="item"
                      onClick={() => {
                        close();
                        si.run();
                      }}
                    >
                      <span className="ctx-label">{si.label}</span>
                    </button>
                  ))}
                </div>
              </div>
            ) : (
              <button
                key={`${j}-${item.label}`}
                className="item"
                onClick={() => {
                  close();
                  item.run?.();
                }}
              >
                <span className="ctx-label">{item.label}</span>
                {item.chord && <span className="ctx-chord">{item.chord}</span>}
              </button>
            ),
          )}
        </Fragment>
      ))}
    </div>
  );
}
