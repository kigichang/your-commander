import { selectFontScale, useSettings } from "../store/settings";

/** A designed pixel size (row height, tile size) at the current font
 *  scale, rounded so rows stay on whole pixels. Virtual lists use it for
 *  both their size estimate and each row's height, keeping the two equal. */
export function useScaled(base: number): number {
  const scale = useSettings(selectFontScale);
  return Math.round(base * scale);
}
