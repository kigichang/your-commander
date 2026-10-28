import { Pause, Play, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useOps } from "../store/ops";
import { Icon } from "./icons";

function formatEta(sec: number | null): string {
  if (sec === null) return "";
  if (sec >= 3600) return `${Math.round(sec / 3600)}h`;
  if (sec >= 60) return `${Math.round(sec / 60)}m`;
  return `${Math.max(1, Math.round(sec))}s`;
}

export function OpsUI() {
  const { t } = useTranslation();
  const jobs = useOps((s) => s.jobs);
  const pausedAll = useOps((s) => s.pausedAll);
  const toasts = useOps((s) => s.toasts);
  const ops = useOps.getState;

  return (
    <>
      {jobs.length > 0 && (
        <div className="queue">
          <div className="q-head">
            <span className="q-title">{t("queue.title")}</span>
            <button className="q-btn" onClick={() => ops().togglePauseAll()}>
              <Icon of={pausedAll ? Play : Pause} size={12} />{" "}
              {pausedAll ? t("queue.resumeAll") : t("queue.pauseAll")}
            </button>
          </div>
          {jobs.map((job) => (
            <div key={job.key} className="op-row">
              <span className="q-label">{job.label}</span>
              <span className="q-desc">{job.desc}</span>
              {job.state === "running" ? (
                <>
                  <div className="bar">
                    <i style={{ width: `${Math.round(job.fraction * 100)}%` }} />
                  </div>
                  <span className="q-eta">
                    {Math.round(job.fraction * 100)}%
                    {job.etaSec !== null && !job.paused
                      ? ` · ${t("queue.left", { eta: formatEta(job.etaSec) })}`
                      : ""}
                  </span>
                  <span className="detail">{job.detail}</span>
                  <button
                    className="q-btn"
                    onClick={() => ops().togglePauseJob(job.key)}
                  >
                    <Icon of={job.paused ? Play : Pause} size={12} />
                  </button>
                </>
              ) : (
                <span className="q-waiting">{t("queue.waiting")}</span>
              )}
              <button className="cancel" onClick={() => ops().cancel(job.key)}>
                <Icon of={X} size={12} />
              </button>
            </div>
          ))}
        </div>
      )}
      {toasts.length > 0 && (
        <div className="toasts">
          {toasts.map((toast) => (
            <div
              key={toast.id}
              className={`toast ${toast.kind}`}
              onClick={() => ops().dismissToast(toast.id)}
            >
              {toast.text}
            </div>
          ))}
        </div>
      )}
    </>
  );
}
