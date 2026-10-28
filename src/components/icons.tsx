import clsx from "clsx";
import type { LucideIcon } from "lucide-react";

export const ICON_SIZE = 16;
export const ICON_STROKE = 1.75;

/** Every UI icon goes through here so size and stroke stay uniform.
 *  (File-row icons are the real macOS ones and don't use this.) */
export function Icon({
  of: I,
  size = ICON_SIZE,
  fill = false,
  className,
}: {
  of: LucideIcon;
  size?: number;
  /** Solid variant (e.g. an active favorite star). */
  fill?: boolean;
  className?: string;
}) {
  return (
    <I
      size={size}
      strokeWidth={ICON_STROKE}
      fill={fill ? "currentColor" : "none"}
      className={clsx("li", className)}
      aria-hidden
    />
  );
}
