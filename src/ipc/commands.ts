import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  Config,
  ConflictChoice,
  FsEntry,
  Location,
  OpEvent,
  OpRequest,
  SshConfigHost,
  VolumeInfo,
} from "./types";

export const listDir = (location: Location, path: string) =>
  invoke<FsEntry[]>("list_dir", { location, path });

export const mkdir = (location: Location, dir: string, name: string) =>
  invoke<void>("mkdir", { location, dir, name });

export const rename = (location: Location, path: string, newName: string) =>
  invoke<void>("rename", { location, path, newName });

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
export const resolveConflict = (opId: number, choice: ConflictChoice) =>
  invoke<void>("resolve_conflict", { opId, choice });

export const sshHosts = () => invoke<SshConfigHost[]>("ssh_hosts");

export const sftpConnect = (params: {
  host: string;
  port: number;
  username: string;
  usePassword: boolean;
  password?: string;
  identityFile?: string;
  passphrase?: string;
}) =>
  invoke<{ connId: number; label: string; home: string }>("sftp_connect", {
    params,
  });

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

export const watchDir = (watchId: number, path: string) =>
  invoke<void>("watch_dir", { watchId, path });
export const unwatch = (watchId: number) => invoke<void>("unwatch", { watchId });
