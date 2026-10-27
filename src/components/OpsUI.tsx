import { useOps } from "../store/ops";

export function OpsUI() {
  const running = useOps((s) => s.running);
  const toasts = useOps((s) => s.toasts);
  const cancel = useOps((s) => s.cancel);
  const dismiss = useOps((s) => s.dismissToast);

  return (
    <>
      {running.length > 0 && (
        <div className="progress-strip">
          {running.map((op) => (
            <div key={op.id} className="op-row">
              <span>{op.label}</span>
              <div className="bar">
                <i style={{ width: `${Math.round(op.fraction * 100)}%` }} />
              </div>
              <span className="detail">{op.detail}</span>
              <button className="cancel" onClick={() => cancel(op.id)}>
                ✕
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
              onClick={() => dismiss(toast.id)}
            >
              {toast.text}
            </div>
          ))}
        </div>
      )}
    </>
  );
}
