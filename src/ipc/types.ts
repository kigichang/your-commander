/** Mirrors `state::Location` on the Rust side. */
export type Location =
  | { kind: "local" }
  | { kind: "sftp"; connId: number }
  | { kind: "archive"; archive: string };

/** Mirrors `fs::provider::FsEntry` (camelCase, modified = epoch ms). */
export interface FsEntry {
  name: string;
  path: string;
  isDir: boolean;
  isSymlink: boolean;
  size: number;
  modified: number | null;
  ext: string;
  isHidden: boolean;
}

export interface VolumeInfo {
  name: string;
  path: string;
  free: number;
  total: number;
  ready: boolean;
  isSystem: boolean;
}

export interface SshConfigHost {
  alias: string;
  hostName: string;
  user: string;
  port: number;
  identity: string;
}

/** Persisted config: snake_case to stay compatible with the old TOML file. */
export interface Favorite {
  name: string;
  path: string;
}

export interface SmbConnection {
  name: string;
  server: string;
  share: string;
  username: string;
  start_in: string;
  remember: boolean;
}

export interface SshConnection {
  name: string;
  host: string;
  port: number;
  username: string;
  identity_file: string;
  use_password: boolean;
  use_agent: boolean;
  start_in: string;
  remember: boolean;
}

export interface ExternalTool {
  name: string;
  app: string;
  extensions: string;
  dirs: boolean;
}

export interface Config {
  smb_connections: SmbConnection[];
  ssh_connections: SshConnection[];
  language: string | null;
  theme: string;
  show_hidden: boolean;
  fkeys_without_fn: boolean;
  favorites: Favorite[];
  left_path: string | null;
  right_path: string | null;
  left_view: string | null;
  right_view: string | null;
  left_tabs: string[];
  right_tabs: string[];
  left_active: number;
  right_active: number;
  left_sort: string | null;
  right_sort: string | null;
  hotkey_preset: string | null;
  hotkeys: Record<string, string>;
  recent_dirs: string[];
  plantuml_mode: string;
  plantuml_server: string;
  plantuml_jar: string;
  plantuml_java: string;
  menu_vscode: boolean;
  menu_zed: boolean;
  external_tools: ExternalTool[];
  left_cols: number[];
  right_cols: number[];
  window_size: [number, number] | null;
  split_ratio: number;
}

export interface FindParams {
  mask: string;
  content: string | null;
  modifiedAfter: number | null;
  minSize: number | null;
  maxSize: number | null;
}

export type FindEvent =
  | { type: "found"; entry: FsEntry }
  | { type: "scanning"; dir: string }
  | { type: "done"; count: number; error: string | null };

export type ConflictChoice =
  | "overwrite"
  | "overwriteAll"
  | "skip"
  | "skipAll"
  | "keepBoth"
  | "cancelOp";

export type OpEvent =
  | {
      type: "progress";
      detail: string;
      fraction: number;
      doneBytes: number;
      totalBytes: number;
    }
  | { type: "conflict"; src: string; dest: string }
  | {
      type: "done";
      ok: boolean;
      cancelled: boolean;
      count: number;
      name: string | null;
      connId: number | null;
      error: string | null;
    };

export interface SourceItem {
  path: string;
  isDir: boolean;
  isSymlink: boolean;
}

export interface SyncSideInfo {
  size: number;
  modified: number | null;
}

export interface SyncRow {
  rel: string;
  left: SyncSideInfo | null;
  right: SyncSideInfo | null;
  equal: boolean;
}

export type SyncDirection = "ltr" | "rtl";

export interface SyncActionItem {
  rel: string;
  direction: SyncDirection;
}

export type IfExists = "ask" | "skip" | "replaceOlder" | "rename";

export interface CopyOptions {
  verify: boolean;
  keepMeta: boolean;
  mask: string | null;
  followSymlinks: boolean;
  ifExists: IfExists;
}

export type OpRequest =
  | {
      op: "copy";
      src: Location;
      sources: SourceItem[];
      dest: Location;
      destDir: string;
      removeSource: boolean;
      options?: CopyOptions;
    }
  | { op: "trash"; items: string[] }
  | { op: "delete"; location: Location; items: string[] }
  | {
      op: "sync";
      left: Location;
      leftDir: string;
      right: Location;
      rightDir: string;
      items: SyncActionItem[];
    }
  | { op: "compress"; sources: string[]; dest: string; format: string }
  | { op: "extract"; archives: string[]; destDir: string }
  | {
      op: "smbMount";
      server: string;
      share: string;
      username: string;
      password: string;
    };
