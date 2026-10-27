/** Minimal POSIX path helpers (backend paths are always absolute POSIX). */

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
