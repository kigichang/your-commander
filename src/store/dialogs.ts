import { create } from "zustand";
import type { FsEntry, Location } from "../ipc/types";
import type { Side } from "./panels";

export type Dialog =
  | { type: "connect"; protocol: "smb" | "sftp" }
  | { type: "copyMove"; move: boolean; side: Side; items: FsEntry[]; destDir: string }
  | { type: "newFolder"; side: Side }
  | { type: "rename"; side: Side; entry: FsEntry }
  | { type: "multiRename"; side: Side; items: FsEntry[] }
  | {
      type: "sync";
      left: Location;
      leftDir: string;
      leftLabel: string;
      right: Location;
      rightDir: string;
      rightLabel: string;
    }
  | { type: "confirmDelete"; side: Side; items: FsEntry[]; remote: boolean }
  | { type: "compress"; side: Side; items: FsEntry[]; destDir: string };

interface DialogStore {
  dialog: Dialog | null;
  open: (dialog: Dialog) => void;
  close: () => void;
}

export const useDialogs = create<DialogStore>((set) => ({
  dialog: null,
  open: (dialog) => set({ dialog }),
  close: () => set({ dialog: null }),
}));
