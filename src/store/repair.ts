import { create } from "zustand";
import * as ipc from "../ipc/commands";
import { baseName } from "../lib/path";
import { useDialogs } from "./dialogs";
import { useOps } from "./ops";

/** The video repair lives here (not in QuickView) so closing the Quick
 *  View pane doesn't lose the running ffmpeg job: the FKeyBar keeps
 *  showing its progress and the keep/overwrite dialog opens on finish. */
interface RepairStore {
  /** The one video currently being re-encoded, or null. */
  job: { path: string; name: string; fraction: number | null } | null;
  start: (path: string) => void;
}

export const useRepair = create<RepairStore>((set, get) => ({
  job: null,
  start: (path) => {
    if (get().job) return;
    set({ job: { path, name: baseName(path), fraction: null } });
    void ipc
      .repairVideo(path, (fraction) =>
        set((s) => (s.job?.path === path ? { job: { ...s.job!, fraction } } : s)),
      )
      .then((tmp) =>
        useDialogs.getState().open({ type: "repairVideo", original: path, tmp }),
      )
      .catch((e) => useOps.getState().toast(String(e), "error"))
      .finally(() => set({ job: null }));
  },
}));
