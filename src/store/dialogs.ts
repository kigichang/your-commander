import { create } from "zustand";
import type { FsEntry, Location } from "../ipc/types";
import type { Side } from "./panels";

export type Dialog =
  | { type: "connect"; protocol: "smb" | "sftp" }
  | { type: "copyMove"; move: boolean; side: Side; items: FsEntry[]; destDir: string }
  | { type: "newFolder"; side: Side }
  | { type: "rename"; side: Side; entry: FsEntry }
  | { type: "multiRename"; side: Side; items: FsEntry[] }
  | { type: "find"; location: Location; dir: string; mask?: string }
  | { type: "settings" }
  | { type: "palette" }
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
  | { type: "confirmEmptyTrash" }
  | { type: "repairVideo"; original: string; tmp: string }
  | { type: "compress"; items: FsEntry[]; destDir: string }
  | {
      type: "compare";
      left: { location: Location; path: string; label: string };
      right: { location: Location; path: string; label: string };
    };

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
