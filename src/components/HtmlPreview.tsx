import { convertFileSrc } from "@tauri-apps/api/core";
import { useEffect, useMemo, useState } from "react";
import * as ipc from "../ipc/commands";

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

/** Puts the CSP and `<base>` ahead of everything in the file, so no
 *  element of the page can load before the policy is in force. Searching
 *  for the page's own <head> is not safe: an <img> ahead of it (or a
 *  "<head>" inside a comment) would place the policy in <body>, where it
 *  is ignored. A <meta> as the first tag opens the head itself. Only a
 *  leading doctype stays in front (ahead of it the page would render in
 *  quirks mode); the tokenizer ends a doctype at its first ">", as this
 *  pattern does. */
function withHead(html: string, base: string | null): string {
  const attr = (s: string) => s.replace(/"/g, "&quot;");
  let tags = `<meta http-equiv="Content-Security-Policy" content="${attr(policy(base))}">`;
  if (base) tags += `<base href="${attr(base)}">`;
  // A byte-order mark the decoder left (a file can start with several) is
  // text to the parser: it would open <body> ahead of everything and the
  // policy would be ignored.
  const text = html.replace(/^\uFEFF+/, "");
  // Only HTML's own whitespace may precede the doctype. JS's \s also
  // matches NBSP, VT, U+2028, U+3000 and more, which the parser takes as
  // text: it opens <body> there, the doctype after it is dropped, and a
  // policy placed behind that doctype would land in <body>, ignored.
  const doctype = /^[\t\n\f\r ]*<!doctype[^>]*>/i.exec(text);
  const end = doctype ? doctype[0].length : 0;
  return text.slice(0, end) + tags + text.slice(end);
}

/** Renders an HTML file the way a browser would, inside a sandbox with no
 *  scripts, forms, popups or same-origin access — the page can't reach the
 *  app or its IPC. Its CSP confines resource loads to `baseDir` (relative
 *  images, stylesheets and fonts next to the file) plus data: URLs; with
 *  no folder nothing external loads at all. The asset protocol's scope
 *  starts empty, so the folder is opened to it before the frame mounts;
 *  if that fails the page renders without its folder. */
export function HtmlPreview({ html, baseDir }: { html: string; baseDir: string | null }) {
  // Outcome of opening a folder to asset://, keyed by the folder so a
  // stale answer for the previous file never applies to this one.
  const [grant, setGrant] = useState<{ dir: string; ok: boolean } | null>(null);
  useEffect(() => {
    if (!baseDir) return;
    let stale = false;
    ipc.allowAssetDir(baseDir).then(
      () => !stale && setGrant({ dir: baseDir, ok: true }),
      () => !stale && setGrant({ dir: baseDir, ok: false }),
    );
    return () => {
      stale = true;
    };
  }, [baseDir]);
  // undefined: still asking; null: no folder to load from.
  const base = !baseDir
    ? null
    : grant?.dir !== baseDir
      ? undefined
      : grant.ok
        ? assetBase(baseDir)
        : null;
  const doc = useMemo(() => (base === undefined ? null : withHead(html, base)), [html, base]);
  if (doc === null) return <div className="qv-html" />;
  return <iframe className="qv-html" sandbox="" srcDoc={doc} title="HTML preview" />;
}
