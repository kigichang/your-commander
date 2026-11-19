const UNITS = ["B", "KB", "MB", "GB", "TB", "PB"];

export function humanSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value >= 100 ? value.toFixed(0) : value.toFixed(1)} ${UNITS[unit]}`;
}

const dateFmt = new Intl.DateTimeFormat(undefined, {
  year: "numeric",
  month: "2-digit",
  day: "2-digit",
  hour: "2-digit",
  minute: "2-digit",
  hour12: false,
});

export function formatDate(epochMs: number | null): string {
  if (!epochMs) return "--";
  return dateFmt.format(new Date(epochMs));
}

/** Bidi embedding, override and isolate controls (U+202A–202E,
 *  U+2066–2069), the directional marks (U+200E, U+200F, U+061C), and the
 *  C0/C1 control characters. A name holding a bidi control can display
 *  reversed, so "report‮fdp.command" would read as "reportdnammoc.pdf";
 *  a control character can hide or break up part of it. */
// eslint-disable-next-line no-control-regex
const BIDI_CONTROLS = /[\u0000-\u001F\u007F-\u009F\u061C\u200E\u200F\u202A-\u202E\u2066-\u2069]/g;

/** A file name or path as the UI shows it: bidi controls and control
 *  characters appear as U+FFFD, so the name shown is the one that runs.
 *  Plain right-to-left names (Arabic, Hebrew) need no controls and are
 *  left alone. For display only: never for a path handed to the backend. */
export const displayName = (name: string) => name.replace(BIDI_CONTROLS, "�");
