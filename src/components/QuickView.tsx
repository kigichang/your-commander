import { Folder, X } from "lucide-react";
import { lazy, Suspense, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import * as ipc from "../ipc/commands";
import { formatDate, humanSize } from "../lib/format";
import { baseName } from "../lib/path";
import { visibleEntries } from "../lib/selectors";
import { usePanels } from "../store/panels";
import { useSettings } from "../store/settings";
import { Icon } from "./icons";

// pdf.js is ~450 KB; load it only when the first PDF is previewed.
const PdfPreview = lazy(() =>
  import("./PdfPreview").then((m) => ({ default: m.PdfPreview })),
);
// markdown-it + KaTeX + highlight.js ride their own chunk too (mermaid
// loads on top of that only when a document actually has diagrams).
const MarkdownPreview = lazy(() =>
  import("./MarkdownPreview").then((m) => ({ default: m.MarkdownPreview })),
);
const PlantUmlPreview = lazy(() =>
  import("./MarkdownPreview").then((m) => ({ default: m.PlantUmlPreview })),
);

const IMAGE_EXTS = new Set(["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "ico"]);
// ".ts" is left out: here it is far more likely TypeScript than MPEG-TS.
const VIDEO_EXTS = new Set([
  "mp4", "mov", "m4v", "avi", "mkv", "webm", "mpg", "mpeg", "wmv", "flv",
  "3gp", "mts", "m2ts",
]);

/** 65.4s → "1:05"; 5025s → "1:23:45". */
function formatDuration(secs: number): string {
  const s = Math.round(secs);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}
const MD_EXTS = new Set(["md", "markdown"]);
const PUML_EXTS = new Set(["puml", "plantuml", "pu"]);
const TEXT_EXTS = new Set([
  "txt", "json", "js", "ts", "tsx", "jsx", "rs", "py", "sh", "zsh", "css",
  "html", "xml", "yml", "yaml", "toml", "ini", "conf", "log", "csv", "swift",
  "c", "h", "cpp", "go", "java", "rb", "sql", "lock",
]);
const IMAGE_MAX = 32 * 1024 * 1024;
const TEXT_MAX = 256 * 1024;
const MD_MAX = 2 * 1024 * 1024;
// pdf.js needs the whole file; past this size show no preview instead of
// freezing the pane on a huge download.
const PDF_MAX = 64 * 1024 * 1024;

type Preview =
  | { kind: "image"; url: string }
  | { kind: "text"; text: string }
  | { kind: "markdown"; text: string }
  | { kind: "plantuml"; text: string }
  | { kind: "pdf"; data: ArrayBuffer }
  | { kind: "none" };

/** The pane this replaces shows the entry under the OTHER panel's cursor. */
export function QuickView() {
  const { t } = useTranslation();
  // The copy/move destination is still this pane's own folder.
  const ownTab = usePanels((s) => {
    const side = s.quickViewSide ?? 0;
    return s.panels[side].tabs[s.panels[side].active];
  });
  const browseSide = usePanels((s) => s.otherSide(s.quickViewSide ?? 0));
  const tab = usePanels((s) => s.panels[browseSide].tabs[s.panels[browseSide].active]);
  const showHidden = useSettings((s) => s.showHidden);
  const entry = useMemo(() => {
    if (!tab) return undefined;
    return visibleEntries(tab, showHidden)[tab.cursor];
  }, [tab, showHidden]);

  const [preview, setPreview] = useState<Preview>({ kind: "none" });
  const [tags, setTags] = useState<string[]>([]);
  const [pdfPages, setPdfPages] = useState(0);
  /** Pixel size of the previewed image or video. */
  const [dims, setDims] = useState<string | null>(null);
  /** Video duration, already formatted. */
  const [duration, setDuration] = useState<string | null>(null);

  useEffect(() => {
    let revoked: string | null = null;
    let stale = false;
    setPreview({ kind: "none" });
    setTags([]);
    setPdfPages(0);
    setDims(null);
    setDuration(null);
    if (!entry || entry.isDir || !tab) return;

    const timer = setTimeout(async () => {
      if (tab.location.kind === "local") {
        void ipc.fileTags(entry.path).then((v) => {
          if (!stale) setTags(v);
        });
        if (VIDEO_EXTS.has(entry.ext)) {
          void ipc
            .mediaInfo(entry.path)
            .then((info) => {
              if (stale) return;
              if (info.duration != null) setDuration(formatDuration(info.duration));
              if (info.width && info.height) setDims(`${info.width} × ${info.height}`);
            })
            .catch(() => {});
        }
      }
      try {
        if (entry.ext === "pdf" && entry.size <= PDF_MAX) {
          const buf = await ipc.readPreview(tab.location, entry.path, PDF_MAX);
          if (stale) return;
          setPreview({ kind: "pdf", data: buf });
        } else if (MD_EXTS.has(entry.ext) && entry.size <= MD_MAX) {
          const buf = await ipc.readPreview(tab.location, entry.path, MD_MAX);
          if (stale) return;
          setPreview({ kind: "markdown", text: new TextDecoder().decode(buf) });
        } else if (PUML_EXTS.has(entry.ext) && entry.size <= TEXT_MAX) {
          const buf = await ipc.readPreview(tab.location, entry.path, TEXT_MAX);
          if (stale) return;
          setPreview({ kind: "plantuml", text: new TextDecoder().decode(buf) });
        } else if (IMAGE_EXTS.has(entry.ext) && entry.size <= IMAGE_MAX) {
          const buf = await ipc.readPreview(tab.location, entry.path, IMAGE_MAX);
          if (stale) return;
          const mime = entry.ext === "svg" ? "image/svg+xml" : `image/${entry.ext}`;
          const url = URL.createObjectURL(new Blob([buf], { type: mime }));
          revoked = url;
          setPreview({ kind: "image", url });
        } else if (TEXT_EXTS.has(entry.ext) || (entry.ext === "" && entry.size <= TEXT_MAX)) {
          const buf = await ipc.readPreview(tab.location, entry.path, TEXT_MAX);
          if (stale) return;
          const bytes = new Uint8Array(buf);
          // Crude binary sniff: NUL byte in the first 8 KB means no text.
          const probe = bytes.subarray(0, 8192);
          if (probe.includes(0)) return setPreview({ kind: "none" });
          setPreview({ kind: "text", text: new TextDecoder().decode(bytes) });
        }
      } catch {
        if (!stale) setPreview({ kind: "none" });
      }
    }, 180);

    return () => {
      stale = true;
      clearTimeout(timer);
      if (revoked) URL.revokeObjectURL(revoked);
    };
  }, [entry?.path, entry?.modified, tab?.location]);

  const kindLabel = !entry
    ? ""
    : entry.isDir
      ? t("quickview.folder")
      : entry.ext === "pdf" && pdfPages > 0
        ? t("quickview.pdfPages", { count: pdfPages })
        : entry.ext
          ? entry.ext.toUpperCase()
          : t("quickview.file");

  return (
    <div className="quickview">
      <div className="qv-head">
        <span className="title">{t("quickview.title")}</span>
        <span className="spacer" />
        <span>{entry?.name ?? ""}</span>
        <button
          className="tbtn"
          title="F3"
          onClick={() => usePanels.getState().toggleQuickView()}
        >
          <Icon of={X} size={14} /> F3
        </button>
      </div>
      <div className="qv-body">
        {!entry ? (
          <span className="none">{t("quickview.noPreview")}</span>
        ) : preview.kind === "pdf" ? (
          <Suspense fallback={<span className="none">…</span>}>
            <PdfPreview data={preview.data} onPages={setPdfPages} />
          </Suspense>
        ) : preview.kind === "markdown" ? (
          <Suspense fallback={<span className="none">…</span>}>
            <MarkdownPreview text={preview.text} />
          </Suspense>
        ) : preview.kind === "plantuml" ? (
          <Suspense fallback={<span className="none">…</span>}>
            <PlantUmlPreview text={preview.text} />
          </Suspense>
        ) : preview.kind === "image" ? (
          <img
            src={preview.url}
            alt={entry.name}
            onLoad={(e) => {
              const img = e.currentTarget;
              // SVGs without intrinsic size report 0 — show nothing then.
              if (img.naturalWidth > 0 && img.naturalHeight > 0) {
                setDims(`${img.naturalWidth} × ${img.naturalHeight}`);
              }
            }}
          />
        ) : preview.kind === "text" ? (
          <pre>{preview.text}</pre>
        ) : (
          <span className="none">
            {entry.isDir ? (
              <>
                <Icon of={Folder} size={14} /> {entry.name}
              </>
            ) : (
              t("quickview.noPreview")
            )}
          </span>
        )}
      </div>
      {entry && (
        <>
          <dl className="qv-meta">
            <dt>{t("quickview.kind")}</dt>
            <dd>{kindLabel}</dd>
            {dims && (
              <>
                <dt>{t("quickview.dimensions")}</dt>
                <dd>{dims}</dd>
              </>
            )}
            {duration && (
              <>
                <dt>{t("quickview.duration")}</dt>
                <dd>{duration}</dd>
              </>
            )}
            <dt>{t("quickview.size")}</dt>
            <dd>{entry.isDir ? "—" : humanSize(entry.size)}</dd>
            <dt>{t("quickview.modified")}</dt>
            <dd>{formatDate(entry.modified)}</dd>
            {tags.length > 0 && (
              <>
                <dt>{t("quickview.tags")}</dt>
                <dd>{tags.map((tag) => `● ${tag}`).join("  ")}</dd>
              </>
            )}
          </dl>
          {tab.location.kind === "local" && (
            <div className="qv-actions">
              <button className="btn primary" onClick={() => void ipc.openPath(entry.path)}>
                {t("quickview.open")} ↵
              </button>
              <button className="btn" onClick={() => void ipc.revealInFinder(entry.path)}>
                {t("quickview.reveal")}
              </button>
            </div>
          )}
        </>
      )}
      <div className="statusline">
        {ownTab ? t("quickview.copyTarget", { name: baseName(ownTab.path) }) : ""}
      </div>
    </div>
  );
}
