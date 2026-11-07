import { convertFileSrc } from "@tauri-apps/api/core";
import { useMemo } from "react";

/** asset:// URL of a local folder, ending in "/", that relative links
 *  resolve against and that the preview's CSP is scoped to. Only the
 *  leading "/" is escaped: the asset protocol drops one leading slash
 *  before decoding, and keeping the other separators literal makes the
 *  URL's path pieces line up with the CSP source's for matching. The
 *  characters encodeURIComponent leaves alone that mean something in a
 *  CSP source list are escaped too. */
function assetBase(dir: string): string {
  const root = convertFileSrc("", "asset");
  const piece = (s: string) =>
    encodeURIComponent(s).replace(
      /[!'()*~]/g,
      (c) => `%${c.charCodeAt(0).toString(16).toUpperCase()}`,
    );
  const rest = dir.replace(/^\/+/, "").split("/").filter(Boolean).map(piece);
  return `${root}%2F${rest.map((s) => s + "/").join("")}`;
}

/** The document's own CSP: nothing loads but inline styles, data: URLs
 *  and, when a folder is given, files under that folder — never the rest
 *  of the disk through asset://. A path piece encoded as "..%2F" would
 *  pass the prefix match, but the asset protocol refuses any decoded ".."
 *  itself. Combined with the app policy srcdoc inherits, remote hosts and
 *  scripts stay blocked. */
function policy(base: string | null): string {
  const local = base ? `${base} ` : "";
  return [
    "default-src 'none'",
    `img-src ${local}data:`,
    `style-src ${local}'unsafe-inline'`,
    `font-src ${local}data:`,
    `media-src ${local}data:`,
  ].join("; ");
}

/** Inserts the CSP and `<base>` where the parser still treats them as
 *  head elements: after <head>, else after the doctype (ahead of it would
 *  force quirks mode), else at the very start. The policy goes first so
 *  it governs everything after it, the base included. */
function withHead(html: string, base: string | null): string {
  const attr = (s: string) => s.replace(/"/g, "&quot;");
  let tags = `<meta http-equiv="Content-Security-Policy" content="${attr(policy(base))}">`;
  if (base) tags += `<base href="${attr(base)}">`;
  const at = /<head\b[^>]*>/i.exec(html) ?? /<!doctype[^>]*>/i.exec(html);
  if (!at) return tags + html;
  const end = at.index + at[0].length;
  return html.slice(0, end) + tags + html.slice(end);
}

/** Renders an HTML file the way a browser would, inside a sandbox with no
 *  scripts, forms, popups or same-origin access — the page can't reach the
 *  app or its IPC. Its CSP confines resource loads to `baseDir` (relative
 *  images, stylesheets and fonts next to the file) plus data: URLs; with
 *  no folder nothing external loads at all. */
export function HtmlPreview({ html, baseDir }: { html: string; baseDir: string | null }) {
  const doc = useMemo(() => withHead(html, baseDir ? assetBase(baseDir) : null), [html, baseDir]);
  return <iframe className="qv-html" sandbox="" srcDoc={doc} title="HTML preview" />;
}
