import clsx from "clsx";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { ActionId } from "../keymap/actions";
import { actions } from "../keymap/actions";
import { prettyChord } from "../keymap/keymap";
import { fuzzyScore } from "../lib/fuzzy";
import { displayPath } from "../lib/path";
import { useDialogs } from "../store/dialogs";
import { useKeymap } from "../store/keymapStore";
import { usePanels } from "../store/panels";
import { useSettings } from "../store/settings";

type Item =
  | { kind: "action"; id: ActionId; label: string; chord: string | null }
  | { kind: "folder"; label: string; path: string; tag: "recent" | "favorite" | "home" };

export function PaletteDialog() {
  const { t } = useTranslation();
  const close = useDialogs((s) => s.close);
  const config = useSettings((s) => s.config);
  const home = useSettings((s) => s.home);
  const map = useKeymap((s) => s.map);
  const [query, setQuery] = useState("");
  const [cursor, setCursor] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => inputRef.current?.focus(), []);

  const all = useMemo<Item[]>(() => {
    const chordOfAction = new Map<ActionId, string>();
    for (const [chord, id] of Object.entries(map)) {
      if (!chordOfAction.has(id as ActionId)) chordOfAction.set(id as ActionId, chord);
    }
    const acts: Item[] = (Object.keys(actions) as ActionId[]).map((id) => ({
      kind: "action",
      id,
      label: t(`action.${id}`),
      chord: chordOfAction.get(id) ?? null,
    }));
    const folders: Item[] = [
      ...(home ? [{ kind: "folder", label: "~", path: home, tag: "home" } as Item] : []),
      ...(config?.favorites ?? []).map(
        (f) =>
          ({ kind: "folder", label: displayPath(f.path, home), path: f.path, tag: "favorite" }) as Item,
      ),
      ...(config?.recent_dirs ?? []).map(
        (p) => ({ kind: "folder", label: displayPath(p, home), path: p, tag: "recent" }) as Item,
      ),
    ];
    // Dedupe folders by path, favorites first.
    const seen = new Set<string>();
    const uniqueFolders = folders.filter((f) => {
      if (f.kind !== "folder" || seen.has(f.path)) return false;
      seen.add(f.path);
      return true;
    });
    return [...acts, ...uniqueFolders];
  }, [map, config, home, t]);

  const shown = useMemo(() => {
    const q = query.trim();
    let items: Array<{ item: Item; score: number }>;
    if (!q) {
      items = all.map((item) => ({ item, score: 0 }));
    } else {
      items = all
        .map((item) => ({ item, score: fuzzyScore(q, item.label) }))
        .filter((x) => x.score >= 0);
      items.sort((a, b) => b.score - a.score);
    }
    const acts = items.filter((x) => x.item.kind === "action").slice(0, q ? 9 : 12);
    const folders = items.filter((x) => x.item.kind === "folder").slice(0, 9);
    // Folders first when browsing idly; actions first when searching.
    const ordered = q ? [...acts, ...folders] : [...folders, ...acts];
    return ordered.map((x) => x.item);
  }, [all, query]);

  useEffect(() => setCursor(0), [query]);

  const run = (item: Item | undefined, otherPane: boolean) => {
    if (!item) return;
    close();
    if (item.kind === "action") {
      actions[item.id].run();
    } else {
      const s = usePanels.getState();
      const side = otherPane ? s.otherSide(s.activeSide) : s.activeSide;
      void s.navigate(side, item.path, { kind: "local" }, null);
    }
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      close();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      setCursor((c) => Math.min(c + 1, shown.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setCursor((c) => Math.max(c - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      run(shown[cursor], e.altKey);
    }
  };

  useEffect(() => {
    listRef.current
      ?.querySelector(`[data-idx="${cursor}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [cursor]);

  let lastKind: string | null = null;

  return (
    <div className="overlay palette-overlay" onMouseDown={(e) => e.target === e.currentTarget && close()}>
      <div className="palette">
        <div className="palette-input">
          <span className="prompt">›</span>
          <input
            ref={inputRef}
            value={query}
            spellCheck={false}
            placeholder={t("palette.hint")}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onKeyDown}
          />
        </div>
        <div className="palette-list" ref={listRef}>
          {shown.length === 0 && <div className="none">{t("palette.empty")}</div>}
          {shown.map((item, i) => {
            const header = item.kind !== lastKind ? item.kind : null;
            lastKind = item.kind;
            return (
              <div key={item.kind === "action" ? item.id : item.path}>
                {header && (
                  <div className="palette-head">
                    {t(header === "action" ? "palette.actions" : "palette.folders")}
                  </div>
                )}
                <button
                  data-idx={i}
                  className={clsx("palette-row", i === cursor && "on")}
                  onMouseEnter={() => setCursor(i)}
                  onClick={(e) => run(item, e.altKey)}
                >
                  <span className="plabel">{item.label}</span>
                  {item.kind === "action" ? (
                    item.chord && <kbd>{prettyChord(item.chord)}</kbd>
                  ) : (
                    <em className="ptag">{t(`palette.tag_${item.tag}`)}</em>
                  )}
                </button>
              </div>
            );
          })}
        </div>
        <div className="palette-foot">{t("palette.foot")}</div>
      </div>
    </div>
  );
}
