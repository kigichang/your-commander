import { Fragment, useMemo } from "react";
import { highlightJson } from "../lib/json";

/** Colored view of `prettyJson` output: keys, strings, numbers, literals,
 *  depth-colored brackets and indent guides. Stays one <pre> of text so a
 *  selection copies as ordinary JSON. */
export function JsonPreview({ text }: { text: string }) {
  const lines = useMemo(() => highlightJson(text), [text]);
  return (
    <pre className="qv-json">
      {lines.map((line, li) => (
        <Fragment key={li}>
          {/* No guide at column 0 — the outermost level sits flush left. */}
          {line.level > 0 && "  "}
          {Array.from({ length: Math.max(0, line.level - 1) }, (_, g) => (
            <span key={g} className="g">
              {"  "}
            </span>
          ))}
          {line.toks.map((t, ti) =>
            t.kind === "plain" ? (
              <Fragment key={ti}>{t.text}</Fragment>
            ) : (
              <span key={ti} className={t.kind}>
                {t.text}
              </span>
            ),
          )}
          {li < lines.length - 1 && "\n"}
        </Fragment>
      ))}
    </pre>
  );
}
