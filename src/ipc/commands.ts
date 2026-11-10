import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  Config,
  ConflictChoice,
  FindEvent,
  FindParams,
  FsEntry,
  Location,
  OpEvent,
  OpRequest,
  SourceItem,
  SshConfigHost,
  SyncRow,
  VolumeInfo,
  Workbook,
} from "./types";

export const listDir = (location: Location, path: string) =>
  invoke<FsEntry[]>("list_dir", { location, path });

export const mkdir = (location: Location, dir: string, name: string) =>
  invoke<void>("mkdir", { location, dir, name });

export const rename = (location: Location, path: string, newName: string) =>
  invoke<void>("rename", { location, path, newName });

export const batchRename = (
  location: Location,
  items: Array<{ path: string; newName: string }>,
) => invoke<number>("batch_rename", { location, items });

export const volumes = () => invoke<VolumeInfo[]>("volumes");
export const homeDir = () => invoke<string>("home_dir");
export const trashDir = () => invoke<string>("trash_dir");
export const desktopDir = () => invoke<string>("desktop_dir");
export const openPath = (path: string) => invoke<void>("open_path", { path });
export const revealInFinder = (path: string) =>
  invoke<void>("reveal_in_finder", { path });
export const fileTags = (path: string) =>
  invoke<string[]>("file_tags", { path });
/** Hand local files to a native drag session (drop on Finder, Mail, …).
 *  Call while the mouse button is still down. */
export const startDrag = (paths: string[], iconKey: string) =>
  invoke<void>("start_drag", { paths, iconKey });
/** Local paths dropped in from another app, as copy-op sources. */
export const statPaths = (paths: string[]) =>
  invoke<SourceItem[]>("stat_paths", { paths });

export const readPreview = async (
  location: Location,
  path: string,
  maxBytes: number,
): Promise<ArrayBuffer> =>
  invoke("read_preview", { location, path, maxBytes });

/** JPEG frame extracted with ffmpeg: the static video-preview fallback. */
/** Every sheet of a workbook as display strings (capped per sheet). */
export const officeSheets = (location: Location, path: string) =>
  invoke<Workbook>("office_sheets", { location, path });

/** The LibreOffice binary Office previews would use, if installed. */
export const officeTools = (sofficePath: string) =>
  invoke<{ soffice: string | null }>("office_tools", { sofficePath });

/** An Office file converted to PDF by LibreOffice. Rejects with
 *  "superseded" when a newer conversion was asked for meanwhile. */
export const officeToPdf = async (
  location: Location,
  path: string,
  size: number,
  modified: number,
): Promise<ArrayBuffer> => invoke("office_to_pdf", { location, path, size, modified });

export const videoPoster = async (path: string): Promise<ArrayBuffer> =>
  invoke("video_poster", { path });

/** True for a cloud file (Google Drive, iCloud…) not downloaded yet. */
export const cloudPlaceholder = (path: string): Promise<boolean> =>
  invoke("cloud_placeholder", { path });

/** Download a cloud placeholder; resolves once it is on disk. */
export const cloudDownload = (path: string): Promise<void> =>
  invoke("cloud_download", { path });

export const startOp = (
  request: OpRequest,
  onEvent: (event: OpEvent) => void,
): Promise<number> => {
  const channel = new Channel<OpEvent>();
  channel.onmessage = onEvent;
  return invoke<number>("start_op", { request, onEvent: channel });
};

export const cancelOp = (opId: number) => invoke<void>("cancel_op", { opId });
export const pauseOp = (opId: number, paused: boolean) =>
  invoke<void>("pause_op", { opId, paused });
export const resolveConflict = (opId: number, choice: ConflictChoice) =>
  invoke<void>("resolve_conflict", { opId, choice });

export const startFind = (
  location: Location,
  dir: string,
  params: FindParams,
  onEvent: (event: FindEvent) => void,
): Promise<number> => {
  const channel = new Channel<FindEvent>();
  channel.onmessage = onEvent;
  return invoke<number>("start_find", { location, dir, params, onEvent: channel });
};

export const cancelFind = (findId: number) => invoke<void>("cancel_find", { findId });

export const syncCompare = (
  left: Location,
  leftDir: string,
  right: Location,
  rightDir: string,
  byContent: boolean,
) =>
  invoke<SyncRow[]>("sync_compare", { left, leftDir, right, rightDir, byContent });

export const sshHosts = () => invoke<SshConfigHost[]>("ssh_hosts");

export interface SftpParams {
  host: string;
  port: number;
  username: string;
  usePassword: boolean;
  useAgent?: boolean;
  password?: string;
  identityFile?: string;
  passphrase?: string;
  startIn?: string;
  /** Fingerprint of the first-seen host key the user confirmed: a server
   *  showing that key has it recorded in known_hosts. */
  trustedFingerprint?: string;
}

/** Rejection value of sftp_connect / sftp_test. */
export type ConnectError =
  | { kind: "unknownHostKey"; keyType: string; fingerprint: string }
  | { kind: "hostKeyChanged"; keyType: string; fingerprint: string }
  | { kind: "other"; message: string };

export const isConnectError = (e: unknown): e is ConnectError =>
  typeof e === "object" && e !== null && typeof (e as { kind?: unknown }).kind === "string";

export const sftpConnect = (params: SftpParams) =>
  invoke<{ connId: number; label: string; home: string }>("sftp_connect", {
    params,
  });

export const sftpTest = (params: SftpParams) => invoke<void>("sftp_test", { params });
export const smbTest = (server: string) => invoke<void>("smb_test", { server });
export const smbShares = (server: string, username: string, password: string) =>
  invoke<string[]>("smb_shares", { server, username, password });

export const sftpDisconnect = (connId: number) =>
  invoke<void>("sftp_disconnect", { connId });

export const smbUnmount = (path: string) =>
  invoke<void>("smb_unmount", { path });
export const smbSavedPassword = (
  username: string,
  server: string,
  share: string,
) => invoke<string | null>("smb_saved_password", { username, server, share });
export const smbStorePassword = (
  username: string,
  server: string,
  share: string,
  password: string,
) => invoke<void>("smb_store_password", { username, server, share, password });
export const smbForgetPassword = (username: string, server: string, share: string) =>
  invoke<void>("smb_forget_password", { username, server, share });

/** SSH passwords are keyed by user, host and port: two servers behind one
 *  host never share one. */
export const sshSavedPassword = (username: string, host: string, port: number) =>
  invoke<string | null>("ssh_saved_password", { username, host, port });
export const sshStorePassword = (username: string, host: string, port: number, password: string) =>
  invoke<void>("ssh_store_password", { username, host, port, password });
export const sshForgetPassword = (username: string, host: string, port: number) =>
  invoke<void>("ssh_forget_password", { username, host, port });

export const getConfig = () => invoke<Config>("get_config");
/** Save only the changed fields; the backend merges them into the shared
 *  config and tells every window ("config-patched"). */
export const updateConfig = (patch: Partial<Config>) =>
  invoke<void>("update_config", { patch });

export const fileIcon = async (key: string): Promise<ArrayBuffer> =>
  invoke("file_icon", { key });

export const listFontFamilies = () => invoke<string[]>("list_font_families");

export const renderPlantuml = (code: string) =>
  invoke<string>("render_plantuml", { code });

export const fkeysSet = (enabled: boolean) => invoke<void>("fkeys_set", { enabled });
export const fkeysAvailable = () => invoke<boolean>("fkeys_available");
export const newWindow = () => invoke<void>("new_window");
export const openIn = (tool: "terminal" | "vscode" | "zed", dir: string) =>
  invoke<void>("open_in", { tool, dir });
export const openWith = (app: string, paths: string[]) =>
  invoke<void>("open_with", { app, paths });
export const appsForPath = (path: string) =>
  invoke<import("./types").OpenApp[]>("apps_for_path", { path });
export const getInfo = (paths: string[]) => invoke<void>("get_info", { paths });
export const mediaInfo = (path: string) =>
  invoke<import("./types").MediaInfo>("media_info", { path });
/** Localhost URL streaming the file with Range support (Linux media). */
export const mediaUrl = (path: string) => invoke<string>("media_url", { path });
export const videoTools = () =>
  invoke<{ player: string | null; ffmpeg: boolean }>("video_tools");
export const repairVideo = (path: string, onProgress: (fraction: number) => void) => {
  const channel = new Channel<number>();
  channel.onmessage = onProgress;
  return invoke<string>("repair_video", { path, onProgress: channel });
};
export const finishRepair = (
  tmp: string,
  original: string,
  action: "overwrite" | "save" | "discard",
) => invoke<string | null>("finish_repair", { tmp, original, action });
export const quitApp = () => invoke<void>("quit_app");

export const watchDir = (watchId: number, path: string) =>
  invoke<void>("watch_dir", { watchId, path });
export const unwatch = (watchId: number) => invoke<void>("unwatch", { watchId });
