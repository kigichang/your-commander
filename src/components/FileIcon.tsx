import { File, Folder } from "lucide-react";
import { useEffect, useState } from "react";
import { fileIcon } from "../ipc/commands";
import { isMac } from "../lib/platform";
import { Icon } from "./icons";

/** Finder icons as blob URLs. Type icons ("folder", "jpg"…) are few and
 *  kept for the app's lifetime; an item's own icon ("path:…", one per app
 *  bundle or favorite) is kept for the `PATH_ICONS` most recently used, and
 *  its blob URL released when dropped. */
const urls = new Map<string, string>();
const PATH_ICONS = 300;
let pathIcons = 0;
const pending = new Map<string, Promise<string | null>>();

function cachedUrl(key: string): string | undefined {
  const url = urls.get(key);
  if (url && key.startsWith("path:")) {
    // Re-insert: Map order doubles as recency.
    urls.delete(key);
    urls.set(key, url);
  }
  return url;
}

function storeUrl(key: string, url: string) {
  if (key.startsWith("path:") && !urls.has(key)) {
    if (pathIcons >= PATH_ICONS) {
      for (const [k, u] of urls) {
        if (k.startsWith("path:")) {
          urls.delete(k);
          URL.revokeObjectURL(u);
          pathIcons--;
          break;
        }
      }
    }
    pathIcons++;
  }
  urls.set(key, url);
}

function iconUrl(key: string): Promise<string | null> {
  const cached = cachedUrl(key);
  if (cached) return Promise.resolve(cached);
  let inFlight = pending.get(key);
  if (!inFlight) {
    inFlight = fileIcon(key)
      .then((buf) => {
        const url = URL.createObjectURL(new Blob([buf], { type: "image/png" }));
        storeUrl(key, url);
        return url;
      })
      .catch(() => null)
      .finally(() => pending.delete(key));
    pending.set(key, inFlight);
  }
  return inFlight;
}

/** The system-icon key for an entry (also the native drag image). macOS
 *  app bundles — and any item asked for with `own` — are keyed by path so
 *  each shows its own icon (a folder's custom icon, Downloads' glyph…). */
export function iconKey(isDir: boolean, ext: string, path: string, own = false): string {
  if (isMac && (own || (isDir && /\.app$/i.test(path)))) return `path:${path}`;
  return isDir ? "folder" : ext || "generic";
}

export function FileIcon({
  isDir,
  ext,
  path,
  large = false,
  own = false,
}: {
  isDir: boolean;
  ext: string;
  path: string;
  /** Icon view's tile size: asks the backend for a 128 px rendition. */
  large?: boolean;
  /** This item's own icon rather than its type's (macOS). */
  own?: boolean;
}) {
  const key = iconKey(isDir, ext, path, own) + (large ? "#128" : "");
  const [url, setUrl] = useState<string | null>(urls.get(key) ?? null);

  useEffect(() => {
    const cached = cachedUrl(key);
    if (cached) {
      setUrl(cached);
      return;
    }
    setUrl(null);
    let on = true;
    void iconUrl(key).then((u) => {
      if (on) setUrl(u);
    });
    return () => {
      on = false;
    };
  }, [key]);

  return url ? (
    <img className="icon icon-img" src={url} alt="" draggable={false} />
  ) : (
    <span className="icon">
      <Icon of={isDir ? Folder : File} size={large ? 40 : 14} />
    </span>
  );
}
