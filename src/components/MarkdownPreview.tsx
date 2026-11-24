import katexPlugin from "@vscode/markdown-it-katex";
import DOMPurify from "dompurify";
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
 *  lists, KaTeX math, highlighted code, and on-demand Mermaid diagrams.
 *  The page it lands in can call every backend command, so what the
 *  renderers made is sanitized once more before it is shown. */
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
// A size cap keeps `\rule{9999em}{9999em}` and the like from blowing the
// layout up; `trust` stays off (no \href, \includegraphics, \htmlStyle).
// The plugin hands its options on to katex.renderToString; its own type
// lists only the plugin's.
md.use(katexFn, { trust: false, maxSize: 50, maxExpand: 500 } as object);

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

/** What markdown-it and its plugins produced, with anything that could run
 *  or restyle the page taken out: a second, independent check on them.
 *  KaTeX's MathML and inline `style` sizes stay, as do the disabled
 *  checkboxes of task lists. */
function sanitize(html: string): string {
  return DOMPurify.sanitize(html, {
    FORBID_TAGS: ["style", "form"],
    // KaTeX's MathML copy of a formula (what a screen reader gets).
    ADD_TAGS: ["semantics", "annotation"],
  });
}

/** Render every `pre.plantuml` under `box` through the backend, replacing
 *  each block with an <img> (never inline SVG — nothing executes). Mode
 *  "off" appends a settings hint and leaves the source visible. Caching is
 *  the backend's alone (keyed by source and the PlantUML settings), so a
 *  revisited diagram is one cheap IPC and a settings change re-renders. */
async function renderPlantumlBlocks(
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
        const svg = await ipc.renderPlantuml(code);
        if (isStale()) return;
        const url = `data:image/svg+xml,${encodeURIComponent(svg)}`;
        const img = document.createElement("img");
        img.src = url;
        img.alt = "PlantUML diagram";
        // Not a drag source: a drop is another way a URL reaches a
        // navigation (MarkdownPreview guards this for its own content).
        img.draggable = false;
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
      // Diagrams are shown as images, where an HTML label (a
      // <foreignObject>) has no place; and a diagram that fails leaves
      // no error drawing behind in the page.
      htmlLabels: false,
      suppressErrorRendering: true,
      // What a diagram's own `%%{init}%%` or front matter may not set:
      // mermaid's defaults, plus the keys that put the file's text into
      // the page's CSS (a fixed, full-window overlay otherwise).
      secure: [
        "secure",
        "securityLevel",
        "startOnLoad",
        "maxTextSize",
        "suppressErrorRendering",
        "maxEdges",
        "themeCSS",
        "fontFamily",
        "altFontFamily",
      ],
    });
    return m.default;
  });
  return mermaidReady;
}

let mermaidSeq = 0;

/** Render every `pre.mermaid` under `box`, replacing each block with an
 *  <img> of its SVG, like PlantUML's: never inline SVG, so nothing a
 *  diagram carries becomes part of this page. A diagram that fails stays
 *  as its source text. */
async function renderMermaidBlocks(box: HTMLElement, isStale: () => boolean): Promise<void> {
  const nodes = Array.from(box.querySelectorAll<HTMLElement>("pre.mermaid:not(.rendered)"));
  if (nodes.length === 0) return;
  let mermaid: Awaited<ReturnType<typeof loadMermaid>>;
  try {
    mermaid = await loadMermaid();
  } catch {
    return;
  }
  // One at a time: mermaid measures each in a scratch element of its own.
  for (const node of nodes) {
    if (isStale()) return;
    try {
      const { svg } = await mermaid.render(`yc-mermaid-${++mermaidSeq}`, node.textContent ?? "");
      if (isStale()) return;
      const img = document.createElement("img");
      img.src = `data:image/svg+xml,${encodeURIComponent(svg)}`;
      img.alt = "Mermaid diagram";
      img.draggable = false;
      // The SVG says width="100%", which as an image means no width at
      // all: give it the diagram's own (the CSS caps it at the pane's).
      const width = /\bviewBox="[-\d.e]+[ ,]+[-\d.e]+[ ,]+([\d.]+)/.exec(svg)?.[1];
      if (width) img.width = Math.ceil(Number(width));
      node.replaceChildren(img);
      node.classList.add("rendered");
    } catch {
      /* the diagram stays as source text */
    }
  }
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
      return sanitize(md.render(text));
    } catch (e) {
      return `<pre>${md.utils.escapeHtml(String(e))}</pre>`;
    }
  }, [text]);

  // Render mermaid and PlantUML blocks only when the document has any.
  useEffect(() => {
    const box = boxRef.current;
    if (!box) return;
    let stale = false;
    void renderMermaidBlocks(box, () => stale);
    void renderPlantumlBlocks(box, () => stale, t("preview.plantumlOff"));
    return () => {
      stale = true;
    };
  }, [html, t]);

  // Links open in the system browser; nothing navigates the webview. A
  // middle click opens them the same way rather than asking the webview
  // for a new window.
  const onClick = (e: React.MouseEvent) => {
    if (e.button !== 0 && e.button !== 1) return;
    const anchor = (e.target as Element).closest("a");
    if (!anchor) return;
    e.preventDefault();
    const href = anchor.getAttribute("href") ?? "";
    if (/^https?:\/\//i.test(href)) void ipc.openUrl(href);
  };
  // Links and images are not drag sources: a drop is the other way a URL
  // could reach a navigation. Selected text still drags.
  const onDragStart = (e: React.DragEvent) => {
    if ((e.target as Element).closest?.("a, img")) e.preventDefault();
  };

  return (
    <div
      ref={boxRef}
      className="markdown-body"
      onClick={onClick}
      onAuxClick={onClick}
      onDragStart={onDragStart}
      dangerouslySetInnerHTML={{ __html: html }}
    />
  );
}
