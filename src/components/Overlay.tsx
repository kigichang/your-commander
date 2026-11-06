import clsx from "clsx";
import { useEffect } from "react";

/** Modal backdrop with its dialog box. A click on the backdrop calls
 *  `onClose`. Escape calls `onEscape`, which defaults to `onClose`; pass it
 *  only when Escape should back out of an inner step before closing. */
export function Overlay({
  children,
  onClose,
  onEscape = onClose,
  className,
}: {
  children: React.ReactNode;
  onClose: () => void;
  onEscape?: () => void;
  className?: string;
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onEscape();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onEscape]);
  return (
    <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className={clsx("dialog", className)}>{children}</div>
    </div>
  );
}
