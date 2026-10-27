/** Subsequence fuzzy score: -1 = no match; higher is better. Word-boundary
 *  and consecutive hits score extra; shorter targets win ties. */
export function fuzzyScore(query: string, text: string): number {
  const q = query.toLowerCase();
  const t = text.toLowerCase();
  if (!q) return 0;
  let qi = 0;
  let score = 0;
  let streak = 0;
  for (let ti = 0; ti < t.length && qi < q.length; ti++) {
    if (t[ti] === q[qi]) {
      const boundary = ti === 0 || /[\s_\-./\\:]/.test(t[ti - 1]);
      score += 1 + streak + (boundary ? 3 : 0);
      streak += 1;
      qi += 1;
    } else {
      streak = 0;
    }
  }
  if (qi < q.length) return -1;
  return score - t.length / 100;
}
