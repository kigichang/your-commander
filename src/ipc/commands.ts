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
  SshConfigHost,
  SyncRow,
  VolumeInfo,
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
export const openPath = (path: string) => invoke<void>("open_path", { path });
export const revealInFinder = (path: string) =>
  invoke<void>("reveal_in_finder", { path });
export const isArchiveName = (name: string) =>
  invoke<boolean>("is_archive", { name });
export const fileTags = (path: string) =>
  invoke<string[]>("file_tags", { path });

export const readPreview = async (
  location: Location,
  path: string,
  maxBytes: number,
): Promise<ArrayBuffer> =>
  invoke("read_preview", { location, path, maxBytes });

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
}

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

export const smbFindMount = (server: string, share: string) =>
  invoke<string | null>("smb_find_mount", { server, share });
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

export const sshSavedPassword = (username: string, host: string) =>
  invoke<string | null>("ssh_saved_password", { username, host });
export const sshStorePassword = (username: string, host: string, password: string) =>
  invoke<void>("ssh_store_password", { username, host, password });
export const sshForgetPassword = (username: string, host: string) =>
  invoke<void>("ssh_forget_password", { username, host });

export const getConfig = () => invoke<Config>("get_config");
export const setConfig = (config: Config) =>
  invoke<void>("set_config", { config });

export const fileIcon = async (key: string): Promise<ArrayBuffer> =>
  invoke("file_icon", { key });

export const renderPlantuml = (code: string) =>
  invoke<string>("render_plantuml", { code });

export const fkeysSet = (enabled: boolean) => invoke<void>("fkeys_set", { enabled });
export const fkeysAvailable = () => invoke<boolean>("fkeys_available");
export const newWindow = () => invoke<void>("new_window");
export const openIn = (tool: "terminal" | "vscode" | "zed", dir: string) =>
  invoke<void>("open_in", { tool, dir });
export const openWith = (app: string, paths: string[]) =>
  invoke<void>("open_with", { app, paths });
export const getInfo = (paths: string[]) => invoke<void>("get_info", { paths });
export const mediaInfo = (path: string) =>
  invoke<import("./types").MediaInfo>("media_info", { path });
export const isNetworkPath = (path: string) =>
  invoke<boolean>("is_network_path", { path });
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
