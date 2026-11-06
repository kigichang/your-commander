import { convertFileSrc } from "@tauri-apps/api/core";
import { useMemo } from "react";

/** asset:// URL of a local folder, ending in "/", that relative links
 *  resolve against. Only the leading "/" is escaped: the asset protocol
 *  drops one leading slash before decoding, and keeping the other
 *  separators literal lets "../img.png" climb folders. */
function assetBase(dir: string): string {
  const root = convertFileSrc("", "asset");
  const rest = dir.replace(/^\/+/, "").split("/").filter(Boolean).map(encodeURIComponent);
  return `${root}%2F${rest.map((s) => s + "/").join("")}`;
}

/** Inserts `<base>` where the parser still treats it as a head element:
 *  after <head>, else after the doctype (ahead of it would force quirks
 *  mode), else at the very start. */
function withBase(html: string, href: string): string {
  const tag = `<base href="${href.replace(/"/g, "&quot;")}">`;
  const at = /<head\b[^>]*>/i.exec(html) ?? /<!doctype[^>]*>/i.exec(html);
  if (!at) return tag + html;
  const end = at.index + at[0].length;
  return html.slice(0, end) + tag + html.slice(end);
}

/** Renders an HTML file the way a browser would, inside a sandbox with no
 *  scripts, forms, popups or same-origin access — the page can't reach the
 *  app or its IPC. The app's CSP still applies (srcdoc inherits it), so
 *  remote images, scripts and fonts stay blocked; local relative images and
 *  stylesheets load when `baseDir` is given. */
export function HtmlPreview({ html, baseDir }: { html: string; baseDir: string | null }) {
  const doc = useMemo(
    () => (baseDir ? withBase(html, assetBase(baseDir)) : html),
    [html, baseDir],
  );
  return <iframe className="qv-html" sandbox="" srcDoc={doc} title="HTML preview" />;
}
