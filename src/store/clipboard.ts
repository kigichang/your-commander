import { create } from "zustand";
import type { FsEntry, Location } from "../ipc/types";

/** In-app file clipboard (⌘C/⌘X → ⌘V). Per window, like the panels. */
export interface ClipEntry {
  items: FsEntry[];
  location: Location;
  /** true = cut: the paste moves instead of copying, then clears the clip. */
  move: boolean;
}

interface ClipboardStore {
  clip: ClipEntry | null;
  set: (clip: ClipEntry) => void;
  clear: () => void;
}

export const useClipboard = create<ClipboardStore>((set) => ({
  clip: null,
  set: (clip) => set({ clip }),
  clear: () => set({ clip: null }),
}));
