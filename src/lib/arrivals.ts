/** Which entries of a destination listing did a finished copy/move put
 *  there? The backend only reports a count, so this is worked out from
 *  names: an arrival keeps its source name, unless a collision was
 *  resolved as "keep both" (or it was copied onto its own folder), in
 *  which case it landed under the backend's "name 2.ext" style name.
 *
 *  `before` is the destination's listing when the operation started, or
 *  null when no pane was showing it; without it a renamed arrival cannot
 *  be told from a file that was already there, so only exact names count. */
export function arrivedNames(
  sourceNames: string[],
  before: ReadonlySet<string> | null,
  after: string[],
): Set<string> {
  // Some volumes (HFS+, SMB servers) hand names back in another Unicode
  // normalization than the one they were written with.
  const nfc = (s: string) => s.normalize("NFC");
  const wanted = new Set(sourceNames.map(nfc));
  const known = before && new Set([...before].map(nfc));

  const exact = new Map<string, string>();
  const renamed = new Map<string, string[]>();
  for (const name of after) {
    const key = nfc(name);
    if (wanted.has(key)) {
      exact.set(key, name);
      continue;
    }
    const origin = known && !known.has(key) ? keepBothOrigin(key) : null;
    if (origin !== null && wanted.has(origin)) {
      renamed.set(origin, [...(renamed.get(origin) ?? []), name]);
    }
  }

  const arrived = new Set<string>();
  for (const key of wanted) {
    // A fresh "name 2" means the entry called "name" is the one that was
    // already there, not the arrival.
    const copies = renamed.get(key);
    if (copies) copies.forEach((n) => arrived.add(n));
    else if (exact.has(key)) arrived.add(exact.get(key)!);
  }
  return arrived;
}

/** Undo the backend's keep-both naming (`keep_both_path` in queue.rs):
 *  "report 2.pdf" → "report.pdf", "notes 3" → "notes". Null when the name
 *  does not have that shape. */
function keepBothOrigin(name: string): string | null {
  const m = /^(.+) \d+(\.[^.]*)?$/.exec(name);
  return m ? m[1] + (m[2] ?? "") : null;
}
