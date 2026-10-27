import { useEffect, useState } from "react";
import { fileIcon } from "../ipc/commands";

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

export function FileIcon({ isDir, ext }: { isDir: boolean; ext: string }) {
  const key = isDir ? "folder" : ext || "generic";
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
    <span className="icon">{isDir ? "▸" : "·"}</span>
  );
}
