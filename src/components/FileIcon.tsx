import { File, Folder } from "lucide-react";
import { useEffect, useState } from "react";
import { fileIcon } from "../ipc/commands";
import { isMac } from "../lib/platform";
import { Icon } from "./icons";

/** Finder icons as blob URLs, one fetch per key for the app's lifetime. */
const urls = new Map<string, string>();
const pending = new Map<string, Promise<string | null>>();

function iconUrl(key: string): Promise<string | null> {
  const cached = urls.get(key);
  if (cached) return Promise.resolve(cached);
  let inFlight = pending.get(key);
  if (!inFlight) {
    inFlight = fileIcon(key)
      .then((buf) => {
        const url = URL.createObjectURL(new Blob([buf], { type: "image/png" }));
        urls.set(key, url);
        return url;
      })
      .catch(() => null)
      .finally(() => pending.delete(key));
    pending.set(key, inFlight);
  }
  return inFlight;
}

/** The system-icon key for an entry (also the native drag image). macOS
 *  app bundles are keyed by path so each shows its own icon. */
export function iconKey(isDir: boolean, ext: string, path: string): string {
  if (isMac && isDir && /\.app$/i.test(path)) return `path:${path}`;
  return isDir ? "folder" : ext || "generic";
}

export function FileIcon({
  isDir,
  ext,
  path,
}: {
  isDir: boolean;
  ext: string;
  path: string;
}) {
  const key = iconKey(isDir, ext, path);
  const [url, setUrl] = useState<string | null>(urls.get(key) ?? null);

  useEffect(() => {
    const cached = urls.get(key);
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
      <Icon of={isDir ? Folder : File} size={14} />
    </span>
  );
}
