import type { FsEntry } from "../ipc/types";
import { wildcardMatcher } from "./wildcard";
import type { SortKey, TabState } from "../store/panels";

const collator = new Intl.Collator(undefined, {
  numeric: true,
  sensitivity: "base",
});

function compare(a: FsEntry, b: FsEntry, key: SortKey): number {
  switch (key) {
    case "name":
      return collator.compare(a.name, b.name);
    case "ext":
      return collator.compare(a.ext, b.ext) || collator.compare(a.name, b.name);
    case "size":
      return a.size - b.size || collator.compare(a.name, b.name);
    case "modified":
      return (a.modified ?? 0) - (b.modified ?? 0) || collator.compare(a.name, b.name);
  }
}

/** The filter+sort only depends on these inputs — NOT on cursor or marks —
 *  so the result is cached per entries array. Every caller (render, store,
 *  actions, status line) shares it; a cursor move costs nothing. Callers
 *  must treat the returned array as read-only. */
const visibleCache = new WeakMap<readonly FsEntry[], { key: string; result: FsEntry[] }>();

/** Folders first, then the chosen order; hidden files and the wildcard mask
 *  applied. The cursor indexes into THIS list. */
export function visibleEntries(tab: TabState, showHidden: boolean): FsEntry[] {
  const key = `${showHidden}|${tab.sortKey}|${tab.sortDir}|${tab.filter}`;
  const hit = visibleCache.get(tab.entries);
  if (hit && hit.key === key) return hit.result;
  const match = wildcardMatcher(tab.filter);
  const kept = tab.entries.filter(
    (e) => (showHidden || !e.isHidden) && (e.isDir || match(e.name)),
  );
  kept.sort((a, b) => {
    if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;
    return compare(a, b, tab.sortKey) * tab.sortDir;
  });
  visibleCache.set(tab.entries, { key, result: kept });
  return kept;
}

/** Entries an operation acts on: the marked set, or the cursor item. */
export function targetEntries(
  tab: TabState,
  visible: FsEntry[],
): FsEntry[] {
  if (tab.marked.size > 0) {
    return visible.filter((e) => tab.marked.has(e.name));
  }
  const at = visible[tab.cursor];
  return at ? [at] : [];
}
