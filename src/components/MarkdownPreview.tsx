import katexPlugin from "@vscode/markdown-it-katex";
import hljs from "highlight.js/lib/common";
import MarkdownIt from "markdown-it";
import taskLists from "markdown-it-task-lists";
import { useEffect, useMemo, useRef } from "react";
import * as ipc from "../ipc/commands";
import "github-markdown-css/github-markdown-dark.css";
import "highlight.js/styles/github-dark.css";
import "katex/dist/katex.min.css";

/** MPE-style markdown rendering: markdown-it (raw HTML escaped), GFM task
 *  lists, KaTeX math, highlighted code, and on-demand Mermaid diagrams. */
const md = new MarkdownIt({
  html: false,
  linkify: true,
  highlight: (code, lang) => {
    if (lang && hljs.getLanguage(lang)) {
      try {
        return hljs.highlight(code, { language: lang, ignoreIllegals: true }).value;
      } catch {
        /* fall through to escaping */
      }
    }
    return "";
  },
});
md.use(taskLists, { enabled: false });
// The katex plugin ships as transpiled CJS; some bundlers hand over the
// namespace object instead of the function.
const katexFn =
  (katexPlugin as unknown as { default?: typeof katexPlugin }).default ?? katexPlugin;
md.use(katexFn);

// ```mermaid fences become diagram containers rendered after mount.
const defaultFence =
  md.renderer.rules.fence ??
  ((tokens, idx, options, _env, self) => self.renderToken(tokens, idx, options));
md.renderer.rules.fence = (tokens, idx, options, env, self) => {
  const token = tokens[idx];
  if (token.info.trim().toLowerCase() === "mermaid") {
    return `<pre class="mermaid">${md.utils.escapeHtml(token.content)}</pre>`;
  }
  return defaultFence(tokens, idx, options, env, self);
};

let mermaidReady: Promise<typeof import("mermaid").default> | null = null;

function loadMermaid() {
  mermaidReady ??= import("mermaid").then((m) => {
    m.default.initialize({
      startOnLoad: false,
      theme: "dark",
      securityLevel: "strict",
    });
    return m.default;
  });
  return mermaidReady;
}

export function MarkdownPreview({ text }: { text: string }) {
  const boxRef = useRef<HTMLDivElement>(null);
  const html = useMemo(() => {
    try {
      return md.render(text);
    } catch (e) {
      return `<pre>${md.utils.escapeHtml(String(e))}</pre>`;
    }
  }, [text]);

  // Render mermaid blocks only when the document has any.
  useEffect(() => {
    const box = boxRef.current;
    if (!box) return;
    const nodes = box.querySelectorAll<HTMLElement>("pre.mermaid");
    if (nodes.length === 0) return;
    let stale = false;
    void loadMermaid()
      .then(async (mermaid) => {
        if (stale) return;
        await mermaid.run({ nodes: Array.from(nodes), suppressErrors: true });
      })
      .catch(() => {
        /* diagrams stay as source text */
      });
    return () => {
      stale = true;
    };
  }, [html]);

  // Links open in the system browser; nothing navigates the webview.
  const onClick = (e: React.MouseEvent) => {
    const anchor = (e.target as HTMLElement).closest("a");
    if (!anchor) return;
    e.preventDefault();
    const href = anchor.getAttribute("href") ?? "";
    if (/^https?:\/\//i.test(href)) void ipc.openPath(href);
  };

  return (
    <div
      ref={boxRef}
      className="markdown-body"
      onClick={onClick}
      dangerouslySetInnerHTML={{ __html: html }}
    />
  );
}
