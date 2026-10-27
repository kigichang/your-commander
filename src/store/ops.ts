import { create } from "zustand";
import i18n from "../i18n";
import * as ipc from "../ipc/commands";
import type { ConflictChoice, OpEvent, OpRequest } from "../ipc/types";
import { baseName } from "../lib/path";
import { usePanels } from "./panels";

export interface RunningOp {
  id: number;
  label: string;
  detail: string;
  fraction: number;
}

export interface PendingConflict {
  opId: number;
  src: string;
  dest: string;
}

interface Toast {
  id: number;
  text: string;
  kind: "info" | "error";
}

interface OpsStore {
  running: RunningOp[];
  conflict: PendingConflict | null;
  toasts: Toast[];
  toast: (text: string, kind?: "info" | "error") => void;
  dismissToast: (id: number) => void;
  /** Start a backend op; `done` runs on success with the Done payload. */
  run: (
    label: string,
    request: OpRequest,
    done?: (e: Extract<OpEvent, { type: "done" }>) => void,
  ) => Promise<void>;
  cancel: (opId: number) => void;
  answerConflict: (choice: ConflictChoice) => void;
}

let toastSeq = 1;

export const useOps = create<OpsStore>((set, get) => ({
  running: [],
  conflict: null,
  toasts: [],

  toast: (text, kind = "info") => {
    const id = toastSeq++;
    set((s) => ({ toasts: [...s.toasts, { id, text, kind }] }));
    setTimeout(() => get().dismissToast(id), kind === "error" ? 8000 : 4000);
  },

  dismissToast: (id) =>
    set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),

  run: async (label, request, done) => {
    // Events can arrive before the invoke's promise resolves with the op id;
    // buffer them until it does.
    let opId: number | null = null;
    const pending: OpEvent[] = [];
    const handle = (event: OpEvent) => {
      if (event.type === "progress") {
        set((s) => ({
          running: s.running.map((op) =>
            op.id === opId
              ? { ...op, detail: baseName(event.detail), fraction: event.fraction }
              : op,
          ),
        }));
      } else if (event.type === "conflict") {
        set({ conflict: { opId: opId!, src: event.src, dest: event.dest } });
      } else if (event.type === "done") {
        set((s) => ({
          running: s.running.filter((op) => op.id !== opId),
          conflict: s.conflict?.opId === opId ? null : s.conflict,
        }));
        if (event.ok) {
          done?.(event);
        } else if (event.cancelled) {
          get().toast(i18n.t("status.cancelled"));
        } else if (event.error) {
          get().toast(event.error, "error");
        }
        // Whatever happened, both panels may have changed on disk.
        void usePanels.getState().refresh(0, { silent: true });
        void usePanels.getState().refresh(1, { silent: true });
      }
    };
    const id = await ipc.startOp(request, (event) => {
      if (opId === null) pending.push(event);
      else handle(event);
    });
    opId = id;
    set((s) => ({
      running: [...s.running, { id, label, detail: "", fraction: 0 }],
    }));
    pending.splice(0).forEach(handle);
  },

  cancel: (opId) => void ipc.cancelOp(opId),

  answerConflict: (choice) => {
    const conflict = get().conflict;
    if (!conflict) return;
    set({ conflict: null });
    void ipc.resolveConflict(conflict.opId, choice);
  },
}));
