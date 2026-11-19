/** Does `name` match the glob `pattern` (`*` any run, `?` any one
 *  character)? Both already lowercased. Walks the two strings with one
 *  fallback point, the last `*`: at worst pattern × name steps. A mask
 *  compiled to a regular expression instead backtracks without bound on
 *  something like `*a*a*a*a*b`, and this runs on every keystroke for
 *  every name in the folder. */
function globMatch(pattern: string, name: string): boolean {
  let p = 0;
  let s = 0;
  let star = -1;
  let mark = 0;
  while (s < name.length) {
    if (p < pattern.length && (pattern[p] === "?" || pattern[p] === name[s])) {
      p++;
      s++;
    } else if (p < pattern.length && pattern[p] === "*") {
      star = p++;
      mark = s;
    } else if (star !== -1) {
      p = star + 1;
      s = ++mark;
    } else {
      return false;
    }
  }
  while (pattern[p] === "*") p++;
  return p === pattern.length;
}

/** Compile a `*.txt;*.md`-style mask into a predicate over file names.
 *  `*.*` and the empty mask match everything. Case-insensitive. */
export function wildcardMatcher(mask: string): (name: string) => boolean {
  const trimmed = mask.trim();
  if (!trimmed || trimmed === "*" || trimmed === "*.*") return () => true;
  const parts = trimmed
    .split(";")
    .map((p) => p.trim().toLowerCase())
    .filter(Boolean);
  if (parts.length === 0) return () => true;
  return (name) => {
    const lower = name.toLowerCase();
    return parts.some((p) => globMatch(p, lower));
  };
}

/** Does this path-bar segment look like a wildcard mask, not a folder name? */
export function isMask(segment: string): boolean {
  return segment.includes("*") || segment.includes("?");
}
