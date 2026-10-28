import katexPlugin from "@vscode/markdown-it-katex";
import hljs from "highlight.js/lib/common";
import MarkdownIt from "markdown-it";
import taskLists from "markdown-it-task-lists";
import { useEffect, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";
import * as ipc from "../ipc/commands";
import { useSettings } from "../store/settings";
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
  const lang = token.info.trim().toLowerCase();
  if (lang === "mermaid") {
    return `<pre class="mermaid">${md.utils.escapeHtml(token.content)}</pre>`;
  }
  if (lang === "plantuml" || lang === "puml") {
    return `<pre class="plantuml">${md.utils.escapeHtml(token.content)}</pre>`;
  }
  return defaultFence(tokens, idx, options, env, self);
};

/** Rendered-diagram cache; keyed by source so cursor moves are instant. */
const plantumlCache = new Map<string, string>();

/** Render every `pre.plantuml` under `box` through the backend, replacing
 *  each block with an <img> (never inline SVG — nothing executes). Mode
 *  "off" appends a settings hint and leaves the source visible. */
export async function renderPlantumlBlocks(
  box: HTMLElement,
  isStale: () => boolean,
  hintText: string,
): Promise<void> {
  const nodes = Array.from(box.querySelectorAll<HTMLElement>("pre.plantuml"));
  if (nodes.length === 0) return;
  const mode = useSettings.getState().config?.plantuml_mode ?? "off";
  if (mode !== "server" && mode !== "jar") {
    for (const node of nodes) {
      const hint = document.createElement("div");
      hint.className = "plantuml-hint";
      hint.textContent = hintText;
      node.appendChild(hint);
    }
    return;
  }
  await Promise.all(
    nodes.map(async (node) => {
      const code = node.textContent ?? "";
      try {
        let url = plantumlCache.get(code);
        if (!url) {
          const svg = await ipc.renderPlantuml(code);
          url = `data:image/svg+xml,${encodeURIComponent(svg)}`;
          plantumlCache.set(code, url);
        }
        if (isStale()) return;
        const img = document.createElement("img");
        img.src = url;
        img.alt = "PlantUML diagram";
        node.replaceChildren(img);
        node.classList.add("rendered");
      } catch (e) {
        if (isStale()) return;
        const err = document.createElement("div");
        err.className = "plantuml-error";
        err.textContent = String(e);
        node.appendChild(err);
      }
    }),
  );
}

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

/** Quick View for a whole .puml/.plantuml file: one centered diagram. */
export function PlantUmlPreview({ text }: { text: string }) {
  const { t } = useTranslation();
  const boxRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const box = boxRef.current;
    if (!box) return;
    let stale = false;
    const pre = document.createElement("pre");
    pre.className = "plantuml";
    pre.textContent = text;
    box.replaceChildren(pre);
    void renderPlantumlBlocks(box, () => stale, t("preview.plantumlOff"));
    return () => {
      stale = true;
    };
  }, [text, t]);

  return <div ref={boxRef} className="markdown-body puml-only" />;
}

export function MarkdownPreview({ text }: { text: string }) {
  const { t } = useTranslation();
  const boxRef = useRef<HTMLDivElement>(null);
  const html = useMemo(() => {
    try {
      return md.render(text);
    } catch (e) {
      return `<pre>${md.utils.escapeHtml(String(e))}</pre>`;
    }
  }, [text]);

  // Render mermaid and PlantUML blocks only when the document has any.
  useEffect(() => {
    const box = boxRef.current;
    if (!box) return;
    let stale = false;
    const nodes = box.querySelectorAll<HTMLElement>("pre.mermaid");
    if (nodes.length > 0) {
      void loadMermaid()
        .then(async (mermaid) => {
          if (stale) return;
          await mermaid.run({ nodes: Array.from(nodes), suppressErrors: true });
        })
        .catch(() => {
          /* diagrams stay as source text */
        });
    }
    void renderPlantumlBlocks(box, () => stale, t("preview.plantumlOff"));
    return () => {
      stale = true;
    };
  }, [html, t]);

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
