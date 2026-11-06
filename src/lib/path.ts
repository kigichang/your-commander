import type { Location } from "../ipc/types";

/** Minimal POSIX path helpers (backend paths are always absolute POSIX). */

export function sameLocation(a: Location, b: Location): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === "sftp" && b.kind === "sftp") return a.connId === b.connId;
  if (a.kind === "archive" && b.kind === "archive") return a.archive === b.archive;
  return true;
}

/** Same folder, however the trailing slash was typed. */
export function sameDir(a: string, b: string): boolean {
  const trim = (p: string) => p.replace(/\/+$/, "") || "/";
  return trim(a) === trim(b);
}

/** Is any of these entries already inside `destDir`? Moving it there
 *  would go nowhere. */
export function alreadyIn(
  src: Location,
  paths: string[],
  dest: Location,
  destDir: string,
): boolean {
  return (
    sameLocation(src, dest) && paths.some((p) => sameDir(parentPath(p) ?? "/", destDir))
  );
}

export function parentPath(path: string): string | null {
  const trimmed = path.replace(/\/+$/, "");
  if (!trimmed || trimmed === "/") return null;
  const idx = trimmed.lastIndexOf("/");
  return idx <= 0 ? "/" : trimmed.slice(0, idx);
}

export function baseName(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  if (!trimmed || trimmed === "/") return "/";
  return trimmed.slice(trimmed.lastIndexOf("/") + 1);
}

export function joinPath(dir: string, name: string): string {
  return dir.endsWith("/") ? dir + name : `${dir}/${name}`;
}

/** Shorten a home-rooted path for display: /Users/x/Documents → ~/Documents */
export function displayPath(path: string, home: string | null): string {
  if (home && path === home) return "~";
  if (home && path.startsWith(home + "/")) return "~" + path.slice(home.length);
  return path;
}

export function expandHome(path: string, home: string | null): string {
  if (home && path === "~") return home;
  if (home && path.startsWith("~/")) return home + path.slice(1);
  return path;
}

/** True when a local panel at `path` is showing the Trash folder. */
export function isTrashDir(location: Location, path: string, trashDir: string | null): boolean {
  return location.kind === "local" && !!trashDir && sameDir(path, trashDir);
}
