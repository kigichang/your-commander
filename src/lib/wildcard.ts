/** Compile a `*.txt;*.md`-style mask into a predicate over file names.
 *  `*.*` and the empty mask match everything. Case-insensitive. */
export function wildcardMatcher(mask: string): (name: string) => boolean {
  const trimmed = mask.trim();
  if (!trimmed || trimmed === "*" || trimmed === "*.*") return () => true;
  const parts = trimmed
    .split(";")
    .map((p) => p.trim())
    .filter(Boolean)
    .map(
      (p) =>
        new RegExp(
          "^" +
            p
              .replace(/[.+^${}()|[\]\\]/g, "\\$&")
              .replace(/\*/g, ".*")
              .replace(/\?/g, ".") +
            "$",
          "i",
        ),
    );
  if (parts.length === 0) return () => true;
  return (name) => parts.some((re) => re.test(name));
}

/** Does this path-bar segment look like a wildcard mask, not a folder name? */
export function isMask(segment: string): boolean {
  return segment.includes("*") || segment.includes("?");
}
