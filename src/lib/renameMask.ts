import type { FsEntry } from "../ipc/types";

export type CaseMode = "unchanged" | "lower" | "upper";

export interface RenameRules {
  nameMask: string;
  extMask: string;
  find: string;
  replace: string;
  regex: boolean;
  caseMode: CaseMode;
  counterStart: number;
  counterStep: number;
}

export const DEFAULT_RULES: RenameRules = {
  nameMask: "[N]",
  extMask: "[E]",
  find: "",
  replace: "",
  regex: false,
  caseMode: "unchanged",
  counterStart: 1,
  counterStep: 1,
};

function stemOf(entry: FsEntry): string {
  if (entry.isDir || !entry.ext) return entry.name;
  const dot = entry.name.length - entry.ext.length - 1;
  return dot > 0 ? entry.name.slice(0, dot) : entry.name;
}

/** Expand `[N]` (name), `[E]` (ext), `[C]`/`[C:width]` (counter), and
 *  `[YMD]`/`[Y]`/`[M]`/`[D]` (file date) placeholders, case-insensitively. */
function expand(mask: string, stem: string, ext: string, counter: number, date: Date): string {
  const pad = (n: number, w: number) => String(n).padStart(w, "0");
  return mask.replace(/\[(N|E|C(?::(\d+))?|YMD|Y|M|D)\]/gi, (_whole, tag: string, width?: string) => {
    const upper = tag.toUpperCase();
    if (upper === "N") return stem;
    if (upper === "E") return ext;
    if (upper.startsWith("C")) return pad(counter, width ? Number(width) : 1);
    if (upper === "YMD") return `${date.getFullYear()}${pad(date.getMonth() + 1, 2)}${pad(date.getDate(), 2)}`;
    if (upper === "Y") return String(date.getFullYear());
    if (upper === "M") return pad(date.getMonth() + 1, 2);
    if (upper === "D") return pad(date.getDate(), 2);
    return "";
  });
}

export interface RenamePreview {
  entry: FsEntry;
  newName: string;
  /** i18n key when the row is invalid, null when fine. */
  problem: string | null;
  changed: boolean;
}

export function previewRenames(
  entries: FsEntry[],
  rules: RenameRules,
  /** Names currently in the folder, to detect collisions outside the batch. */
  existingNames: Set<string>,
): RenamePreview[] {
  let findRe: RegExp | null = null;
  if (rules.find) {
    try {
      findRe = rules.regex
        ? new RegExp(rules.find, "g")
        : new RegExp(rules.find.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "gi");
    } catch {
      findRe = null; // bad regex: rows get flagged below
    }
  }
  const badRegex = rules.regex && rules.find !== "" && findRe === null;

  const out: RenamePreview[] = [];
  const seen = new Map<string, number>();
  entries.forEach((entry, i) => {
    const counter = rules.counterStart + i * rules.counterStep;
    const date = new Date(entry.modified ?? Date.now());
    const stem = stemOf(entry);
    const name = expand(rules.nameMask, stem, entry.ext, counter, date);
    const ext = entry.isDir ? "" : expand(rules.extMask, stem, entry.ext, counter, date);
    let full = ext ? `${name}.${ext}` : name;
    if (findRe) full = full.replace(findRe, rules.replace);
    if (rules.caseMode === "lower") full = full.toLowerCase();
    if (rules.caseMode === "upper") full = full.toUpperCase();

    let problem: string | null = null;
    if (badRegex) problem = "rename.badRegex";
    else if (!full || full === "." || full.includes("/")) problem = "rename.badName";
    else if (seen.has(full)) problem = "rename.duplicate";
    else if (full !== entry.name && existingNames.has(full)) problem = "rename.taken";
    seen.set(full, i);
    out.push({ entry, newName: full, problem, changed: full !== entry.name });
  });

  // A name "taken" by another batch member that is being renamed away is fine.
  const leaving = new Set(out.filter((r) => r.changed).map((r) => r.entry.name));
  for (const row of out) {
    if (row.problem === "rename.taken" && leaving.has(row.newName)) row.problem = null;
  }
  return out;
}
