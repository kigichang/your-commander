import { convertFileSrc } from "@tauri-apps/api/core";
import { Folder, X } from "lucide-react";
import { lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import * as ipc from "../ipc/commands";
import { formatDate, humanSize } from "../lib/format";
import { baseName } from "../lib/path";
import { visibleEntries } from "../lib/selectors";
import { AudioPreview } from "./AudioPreview";
import { useOps } from "../store/ops";
import { usePanels } from "../store/panels";
import { useRepair } from "../store/repair";
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
const AUDIO_EXTS = new Set([
  "mp3", "m4a", "m4b", "aac", "wav", "flac", "aiff", "aif", "ogg", "oga",
  "opus", "caf", "wma",
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
// The asset protocol answers ranges in ~1 MB chunks; AVFoundation drops the
// video track of files that fit in a single chunk (black picture, audio
// fine) and starves on network-mount latency. Those cases play from a Blob.
const VIDEO_BLOB_MIN = 2 * 1024 * 1024;
const VIDEO_BLOB_MAX = 256 * 1024 * 1024;
const VIDEO_MIME: Record<string, string> = {
  mov: "video/quicktime",
  webm: "video/webm",
  avi: "video/x-msvideo",
};
const AUDIO_MIME: Record<string, string> = {
  m4a: "audio/mp4",
  m4b: "audio/mp4",
  aac: "audio/aac",
  wav: "audio/wav",
  flac: "audio/flac",
  aiff: "audio/aiff",
  aif: "audio/aiff",
  ogg: "audio/ogg",
  oga: "audio/ogg",
  opus: "audio/ogg",
  caf: "audio/x-caf",
};

type Preview =
  | { kind: "image"; url: string }
  | { kind: "video"; url: string }
  | { kind: "video-failed" }
  | { kind: "audio"; url: string; data: ArrayBuffer | null }
  | { kind: "text"; text: string }
  | { kind: "markdown"; text: string }
  | { kind: "plantuml"; text: string }
  | { kind: "pdf"; data: ArrayBuffer }
  | { kind: "none" };

/** "Open With" + an upward app list from LaunchServices, lazy-loaded on
 *  first click. */
function OpenWithButton({ path }: { path: string }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [apps, setApps] = useState<import("../ipc/types").OpenApp[] | null>(null);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setOpen(false);
    setApps(null);
  }, [path]);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", onDown, true);
    return () => window.removeEventListener("mousedown", onDown, true);
  }, [open]);

  return (
    <div className="qv-openwith" ref={ref}>
      <button
        className="btn"
        onClick={() => {
          setOpen((o) => !o);
          if (apps === null) {
            void ipc.appsForPath(path).then(setApps).catch(() => setApps([]));
          }
        }}
      >
        {t("ctx.openWith")} ▴
      </button>
      {open && (
        <div className="menu qv-openwith-menu">
          {apps === null ? (
            <div className="item disabled">…</div>
          ) : apps.length === 0 ? (
            <div className="item disabled">—</div>
          ) : (
            apps.map((a) => (
              <button
                key={a.path}
                className="item"
                onClick={() => {
                  setOpen(false);
                  void ipc
                    .openWith(a.path, [path])
                    .catch((e) => useOps.getState().toast(String(e), "error"));
                }}
              >
                <span className="ctx-label">
                  {a.default ? t("ctx.defaultApp", { name: a.name }) : a.name}
                </span>
              </button>
            ))
          )}
        </div>
      )}
    </div>
  );
}

/** The pane this replaces shows the entry under the OTHER panel's cursor. */
export function QuickView() {
  const { t } = useTranslation();
  // The copy/move destination is still this pane's own folder.
  const ownTab = usePanels((s) => {
    const side = s.quickViewSide ?? 0;
    return s.panels[side].tabs[s.panels[side].active];
  });
  const config = useSettings((s) => s.config);
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
  /** Local helpers for undecodable videos (player app, ffmpeg). */
  const [tools, setTools] = useState<{ player: string | null; ffmpeg: boolean }>({
    player: null,
    ffmpeg: false,
  });
  const repairJob = useRepair((s) => s.job);

  useEffect(() => {
    void ipc.videoTools().then(setTools).catch(() => {});
  }, []);

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
        const isVideo = VIDEO_EXTS.has(entry.ext);
        if (isVideo || AUDIO_EXTS.has(entry.ext)) {
          void ipc
            .mediaInfo(entry.path)
            .then((info) => {
              if (stale) return;
              if (info.duration != null) setDuration(formatDuration(info.duration));
              if (info.width && info.height) setDims(`${info.width} × ${info.height}`);
            })
            .catch(() => {});
          try {
            if (!isVideo) {
              // Waveform/spectrogram need the whole file, so audio loads
              // into memory (capped); bigger files play from the asset
              // protocol without analysis.
              if (entry.size <= VIDEO_BLOB_MAX) {
                const buf = await ipc.readPreview(tab.location, entry.path, VIDEO_BLOB_MAX);
                if (stale) return;
                const mime = AUDIO_MIME[entry.ext] ?? "audio/mpeg";
                const url = URL.createObjectURL(new Blob([buf], { type: mime }));
                revoked = url;
                setPreview({ kind: "audio", url, data: buf });
              } else {
                setPreview({ kind: "audio", url: convertFileSrc(entry.path), data: null });
              }
              return;
            }
            const needsBlob =
              entry.size < VIDEO_BLOB_MIN ||
              (entry.size <= VIDEO_BLOB_MAX &&
                (await ipc.isNetworkPath(entry.path).catch(() => false)));
            if (stale) return;
            if (needsBlob) {
              const buf = await ipc.readPreview(tab.location, entry.path, VIDEO_BLOB_MAX);
              if (stale) return;
              const mime = VIDEO_MIME[entry.ext] ?? "video/mp4";
              const url = URL.createObjectURL(new Blob([buf], { type: mime }));
              revoked = url;
              setPreview({ kind: "video", url });
            } else {
              // Local disks stream through the asset protocol (range
              // requests, nothing read into memory). Formats the webview
              // can't decode fall back to "none" via onError.
              setPreview({ kind: "video", url: convertFileSrc(entry.path) });
            }
          } catch {
            if (!stale) setPreview({ kind: "none" });
          }
          return;
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
        ) : preview.kind === "video" ? (
          <video
            src={preview.url}
            controls
            playsInline
            autoPlay={config?.qv_autoplay ?? false}
            muted={config?.qv_mute ?? false}
            onLoadedMetadata={(e) => {
              const v = e.currentTarget;
              if (v.videoWidth > 0 && v.videoHeight > 0) {
                setDims((d) => d ?? `${v.videoWidth} × ${v.videoHeight}`);
              }
              if (Number.isFinite(v.duration)) {
                setDuration((d) => d ?? formatDuration(v.duration));
              }
            }}
            onError={() => setPreview({ kind: "none" })}
            ref={(v) => {
              if (!v) return;
              // React doesn't reliably sync the muted attribute; set the
              // property directly.
              v.muted = useSettings.getState().config?.qv_mute ?? false;
              // requestVideoFrameCallback fires only when a frame is really
              // presented — the reliable signal that video (not just audio)
              // is rendering.
              if (v && typeof v.requestVideoFrameCallback === "function") {
                v.requestVideoFrameCallback(() => {
                  v.dataset.framed = "1";
                });
              }
            }}
            onTimeUpdate={(e) => {
              // Some sloppily encoded files decode audio but silently yield
              // zero video frames (AVFoundation reports no error). After a
              // second of playback without a single presented frame, swap
              // the black rectangle for an explanation.
              const v = e.currentTarget;
              if (
                v.currentTime > 1 &&
                typeof v.requestVideoFrameCallback === "function" &&
                !v.dataset.framed
              ) {
                setPreview({ kind: "video-failed" });
              }
            }}
          />
        ) : preview.kind === "video-failed" ? (
          <div className="qv-fail">
            <span className="none">{t("quickview.videoUnsupported")}</span>
            <div className="qv-fail-actions">
              {tools.player && tab.location.kind === "local" && (
                <button
                  className="btn"
                  onClick={() =>
                    void ipc
                      .openWith(tools.player!, [entry.path])
                      .catch((e) => useOps.getState().toast(String(e), "error"))
                  }
                >
                  {t("quickview.openWith", { app: tools.player })}
                </button>
              )}
              {tools.ffmpeg && tab.location.kind === "local" && (
                <button
                  className="btn"
                  disabled={repairJob !== null}
                  onClick={() => useRepair.getState().start(entry.path)}
                >
                  {repairJob?.path === entry.path
                    ? repairJob.fraction !== null
                      ? `${t("quickview.repairing")} ${Math.round(repairJob.fraction * 100)}%`
                      : t("quickview.repairing")
                    : t("quickview.repair")}
                </button>
              )}
            </div>
          </div>
        ) : preview.kind === "audio" ? (
          <AudioPreview
            url={preview.url}
            data={preview.data}
            onDuration={(secs) => setDuration((d) => d ?? formatDuration(secs))}
          />
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
              {!entry.isDir && <OpenWithButton path={entry.path} />}
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
