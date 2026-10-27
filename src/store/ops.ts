import { create } from "zustand";
import i18n from "../i18n";
import * as ipc from "../ipc/commands";
import type { ConflictChoice, OpEvent, OpRequest } from "../ipc/types";
import { baseName } from "../lib/path";
import { usePanels } from "./panels";

type DoneEvent = Extract<OpEvent, { type: "done" }>;

export interface Job {
  key: number;
  label: string;
  /** "photos +2 → web-app" — what is moving where. */
  desc: string;
  request: OpRequest;
  /** Queued jobs run one at a time; direct jobs start immediately. */
  queued: boolean;
  state: "waiting" | "running";
  opId: number | null;
  paused: boolean;
  fraction: number;
  detail: string;
  /** Rolling bytes/sec and the remaining-time estimate from it. */
  rate: number;
  etaSec: number | null;
  lastT: number;
  lastBytes: number;
  done?: (e: DoneEvent) => void;
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
  jobs: Job[];
  pausedAll: boolean;
  conflict: PendingConflict | null;
  toasts: Toast[];
  toast: (text: string, kind?: "info" | "error") => void;
  dismissToast: (id: number) => void;
  /** Start right away (parallel). */
  run: (label: string, request: OpRequest, done?: (e: DoneEvent) => void) => void;
  /** Append to the sequential queue. */
  enqueue: (label: string, request: OpRequest, done?: (e: DoneEvent) => void) => void;
  togglePauseAll: () => void;
  togglePauseJob: (key: number) => void;
  cancel: (key: number) => void;
  answerConflict: (choice: ConflictChoice) => void;
}

let jobSeq = 1;
let toastSeq = 1;

function describe(req: OpRequest): string {
  switch (req.op) {
    case "copy": {
      const first = baseName(req.sources[0]?.path ?? "");
      const extra = req.sources.length > 1 ? ` +${req.sources.length - 1}` : "";
      return `${first}${extra} → ${baseName(req.destDir)}`;
    }
    case "compress":
      return baseName(req.dest);
    case "extract": {
      const first = baseName(req.archives[0] ?? "");
      const extra = req.archives.length > 1 ? ` +${req.archives.length - 1}` : "";
      return `${first}${extra} → ${baseName(req.destDir)}`;
    }
    case "trash":
    case "delete": {
      const first = baseName(req.items[0] ?? "");
      return req.items.length > 1 ? `${first} +${req.items.length - 1}` : first;
    }
    case "sync":
      return `${baseName(req.leftDir)} ⇄ ${baseName(req.rightDir)}`;
    case "smbMount":
      return `smb://${req.server}/${req.share}`;
  }
}

function makeJob(
  label: string,
  request: OpRequest,
  queued: boolean,
  done?: (e: DoneEvent) => void,
): Job {
  return {
    key: jobSeq++,
    label,
    desc: describe(request),
    request,
    queued,
    state: "waiting",
    opId: null,
    paused: false,
    fraction: 0,
    detail: "",
    rate: 0,
    etaSec: null,
    lastT: 0,
    lastBytes: 0,
    done,
  };
}

export const useOps = create<OpsStore>((set, get) => {
  const patchJob = (key: number, patch: Partial<Job> | ((j: Job) => Partial<Job>)) =>
    set((s) => ({
      jobs: s.jobs.map((j) =>
        j.key === key ? { ...j, ...(typeof patch === "function" ? patch(j) : patch) } : j,
      ),
    }));

  /** Start the next waiting queued job when none is running. */
  const pump = () => {
    const s = get();
    if (s.pausedAll) return;
    if (s.jobs.some((j) => j.queued && j.state === "running")) return;
    const next = s.jobs.find((j) => j.queued && j.state === "waiting");
    if (next) void startJob(next.key);
  };

  const startJob = async (key: number) => {
    const job = get().jobs.find((j) => j.key === key);
    if (!job || job.state === "running") return;
    patchJob(key, { state: "running", lastT: performance.now() });
    let opId: number | null = null;
    const pending: OpEvent[] = [];
    const handle = (event: OpEvent) => {
      if (event.type === "progress") {
        patchJob(key, (j) => {
          const now = performance.now();
          const dt = (now - j.lastT) / 1000;
          let { rate, lastT, lastBytes } = j;
          if (dt > 0.25) {
            const inst = (event.doneBytes - lastBytes) / dt;
            rate = rate > 0 ? rate * 0.7 + inst * 0.3 : inst;
            lastT = now;
            lastBytes = event.doneBytes;
          }
          const remaining = event.totalBytes - event.doneBytes;
          return {
            fraction: event.fraction,
            detail: baseName(event.detail),
            rate,
            lastT,
            lastBytes,
            etaSec:
              rate > 1 && event.totalBytes > 0 ? Math.max(1, remaining / rate) : j.etaSec,
          };
        });
      } else if (event.type === "conflict") {
        set({ conflict: { opId: opId!, src: event.src, dest: event.dest } });
      } else if (event.type === "done") {
        const current = get().jobs.find((j) => j.key === key);
        set((s) => ({
          jobs: s.jobs.filter((j) => j.key !== key),
          conflict: s.conflict?.opId === opId ? null : s.conflict,
        }));
        if (event.ok) {
          current?.done?.(event);
        } else if (event.cancelled) {
          get().toast(i18n.t("status.cancelled"));
        } else if (event.error) {
          get().toast(event.error, "error");
        }
        void usePanels.getState().refresh(0, { silent: true });
        void usePanels.getState().refresh(1, { silent: true });
        pump();
      }
    };
    try {
      const id = await ipc.startOp(job.request, (event) => {
        if (opId === null) pending.push(event);
        else handle(event);
      });
      opId = id;
      patchJob(key, { opId: id });
      if (get().pausedAll) void ipc.pauseOp(id, true);
      pending.splice(0).forEach(handle);
    } catch (e) {
      set((s) => ({ jobs: s.jobs.filter((j) => j.key !== key) }));
      get().toast(String(e), "error");
      pump();
    }
  };

  return {
    jobs: [],
    pausedAll: false,
    conflict: null,
    toasts: [],

    toast: (text, kind = "info") => {
      const id = toastSeq++;
      set((s) => ({ toasts: [...s.toasts, { id, text, kind }] }));
      setTimeout(() => get().dismissToast(id), kind === "error" ? 8000 : 4000);
    },

    dismissToast: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),

    run: (label, request, done) => {
      const job = makeJob(label, request, false, done);
      set((s) => ({ jobs: [...s.jobs, job] }));
      void startJob(job.key);
    },

    enqueue: (label, request, done) => {
      const job = makeJob(label, request, true, done);
      set((s) => ({ jobs: [...s.jobs, job] }));
      pump();
    },

    togglePauseAll: () => {
      const next = !get().pausedAll;
      set({ pausedAll: next });
      for (const job of get().jobs) {
        if (job.state === "running" && job.opId !== null) {
          void ipc.pauseOp(job.opId, next);
          patchJob(job.key, { paused: next });
        }
      }
      if (!next) pump();
    },

    togglePauseJob: (key) => {
      const job = get().jobs.find((j) => j.key === key);
      if (!job || job.opId === null) return;
      void ipc.pauseOp(job.opId, !job.paused);
      patchJob(key, { paused: !job.paused });
    },

    cancel: (key) => {
      const job = get().jobs.find((j) => j.key === key);
      if (!job) return;
      if (job.state === "running" && job.opId !== null) {
        void ipc.cancelOp(job.opId);
      } else {
        set((s) => ({ jobs: s.jobs.filter((j) => j.key !== key) }));
      }
    },

    answerConflict: (choice) => {
      const conflict = get().conflict;
      if (!conflict) return;
      set({ conflict: null });
      void ipc.resolveConflict(conflict.opId, choice);
    },
  };
});
