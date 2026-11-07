import { convertFileSrc } from "@tauri-apps/api/core";
import { Folder, X } from "lucide-react";
import { lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import * as ipc from "../ipc/commands";
import { formatDate, humanSize } from "../lib/format";
import { isJson, prettyJson } from "../lib/json";
import { baseName, parentPath } from "../lib/path";
import { isMac } from "../lib/platform";
import { visibleEntries } from "../lib/selectors";
import { decodeText } from "../lib/text";
import { AudioPreview } from "./AudioPreview";
import { HtmlPreview } from "./HtmlPreview";
import { JsonPreview } from "./JsonPreview";
import { useOps } from "../store/ops";
import { usePanels } from "../store/panels";
import { useRepair } from "../store/repair";
import { selectShowHidden, useSettings } from "../store/settings";
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
const HTML_EXTS = new Set(["html", "htm", "xhtml"]);
const PUML_EXTS = new Set(["puml", "plantuml", "pu"]);
const TEXT_EXTS = new Set([
  "txt", "json", "js", "ts", "tsx", "jsx", "rs", "py", "sh", "zsh", "css",
  "html", "htm", "xhtml", "xml", "yml", "yaml", "toml", "ini", "conf", "log", "csv", "swift",
  "c", "h", "cpp", "go", "java", "rb", "sql", "lock",
]);
const IMAGE_MAX = 32 * 1024 * 1024;
const TEXT_MAX = 256 * 1024;
const MD_MAX = 2 * 1024 * 1024;
const HEX_MAX = 256 * 1024;
const HEX_HEAD =
  "          " +
  Array.from({ length: 16 }, (_, i) => i.toString(16).toUpperCase().padStart(2, "0")).join(" ") +
  "  Decoded Text";

/** Classic hex dump: offset, 16 bytes per row, printable ASCII. */
function hexDump(bytes: Uint8Array, truncated: boolean): string {
  const rows: string[] = [];
  for (let off = 0; off < bytes.length; off += 16) {
    const row = bytes.subarray(off, off + 16);
    let hex = "";
    let text = "";
    for (const b of row) {
      hex += b.toString(16).toUpperCase().padStart(2, "0") + " ";
      text += b >= 0x20 && b < 0x7f ? String.fromCharCode(b) : ".";
    }
    rows.push(`${off.toString(16).toUpperCase().padStart(8, "0")}  ${hex.padEnd(48)} ${text}`);
  }
  if (truncated) rows.push("…");
  return rows.join("\n");
}
// pdf.js needs the whole file; past this size show no preview instead of
// freezing the pane on a huge download.
const PDF_MAX = 64 * 1024 * 1024;
// Video streams through the app's stream:// protocol, which serves the
// requested range in 16 MB chunks — unlike the built-in asset protocol's
// ~1 MB cap, which starves AVFoundation on network mounts and left large
// SMB videos with no preview. Sub-chunk files still play from a Blob:
// AVFoundation drops the video track of files served in a single range
// response (black picture, audio fine).
const VIDEO_BLOB_MIN = 2 * 1024 * 1024;
// Waveform/spectrogram need the whole file in memory; cap that.
const AUDIO_BLOB_MAX = 256 * 1024 * 1024;
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
  /** `isStale` turns true once the cursor has moved on from this load. */
  | { kind: "video"; url: string; isStale: () => boolean }
  | { kind: "video-failed" }
  | { kind: "audio"; url: string; data: ArrayBuffer | null }
  | { kind: "text"; text: string }
  | { kind: "hex"; text: string }
  | { kind: "markdown"; text: string }
  | { kind: "plantuml"; text: string }
  | { kind: "pdf"; data: ArrayBuffer }
  /** ffmpeg-extracted still frame for videos the decoder can't render. */
  | { kind: "video-poster"; url: string }
  /** Still reading the file — show nothing, not "no preview". */
  | { kind: "loading" }
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

/** One <video> per URL (render with `key={url}`). A detached media element
 *  keeps its player, decoder and stream:// loads alive until GC, so rapid
 *  switching piled them up; this one releases them on unmount. */
function VideoPreview({
  url,
  autoPlay,
  isStale,
  onMeta,
  onFail,
}: {
  url: string;
  autoPlay: boolean;
  isStale: () => boolean;
  onMeta: (v: HTMLVideoElement) => void;
  onFail: (fallback: Preview) => void;
}) {
  const ref = useRef<HTMLVideoElement>(null);
  const framed = useRef(false);
  const failed = useRef(false);

  useEffect(() => {
    const v = ref.current;
    if (!v) return;
    // React doesn't reliably sync the muted attribute; set the property
    // directly. src is set here (not as a prop) so the release below can
    // clear it and a StrictMode re-run restores it.
    v.muted = useSettings.getState().config?.qv_mute ?? false;
    v.src = url;
    // requestVideoFrameCallback fires only when a frame is really
    // presented — the reliable signal that video (not just audio) is
    // rendering.
    if (typeof v.requestVideoFrameCallback === "function") {
      v.requestVideoFrameCallback(() => {
        framed.current = true;
      });
    }
    return () => {
      v.pause();
      v.removeAttribute("src");
      v.load();
    };
  }, [url]);

  // At most once, and never for a load the cursor already left (e.g. the
  // error a revoked Blob URL raises while this element is still mounted).
  const fail = (fallback: Preview) => {
    if (failed.current || isStale()) return;
    failed.current = true;
    onFail(fallback);
  };

  return (
    <video
      ref={ref}
      controls
      playsInline
      autoPlay={autoPlay}
      onLoadedMetadata={(e) => {
        if (!isStale()) onMeta(e.currentTarget);
      }}
      onError={() => fail({ kind: "none" })}
      onTimeUpdate={(e) => {
        // Some sloppily encoded files decode audio but silently yield zero
        // video frames (AVFoundation reports no error). After a second of
        // playback without a single presented frame, swap the black
        // rectangle for an explanation.
        const v = e.currentTarget;
        if (
          v.currentTime > 1 &&
          typeof v.requestVideoFrameCallback === "function" &&
          !framed.current
        ) {
          fail({ kind: "video-failed" });
        }
      }}
    />
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
  const autoplay = useSettings((s) => s.config?.qv_autoplay ?? false);
  const browseSide = usePanels((s) => s.otherSide(s.quickViewSide ?? 0));
  const tab = usePanels((s) => s.panels[browseSide].tabs[s.panels[browseSide].active]);
  const showHidden = useSettings(selectShowHidden);
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

  // The poster's blob URL lives as long as it is the preview shown.
  useEffect(() => {
    if (preview.kind !== "video-poster") return;
    const url = preview.url;
    return () => URL.revokeObjectURL(url);
  }, [preview]);
  /** Local helpers for undecodable videos (player app, ffmpeg). */
  const [tools, setTools] = useState<{ player: string | null; ffmpeg: boolean }>({
    player: null,
    ffmpeg: false,
  });
  const repairJob = useRepair((s) => s.job);
  /** ⌘F3: show every file as a hex dump instead of its usual preview. */
  const hex = usePanels((s) => s.quickViewHex);
  /** "Pretty" re-indents JSON previews; it stays on while the cursor moves
   *  to other files so a folder of JSON reads the same way. */
  const [pretty, setPretty] = useState(false);
  /** "View" renders HTML previews instead of showing their source; sticky
   *  across files the same way. */
  const [htmlView, setHtmlView] = useState(false);
  /** Path under the cursor right now — guards async results that outlive it. */
  const entryRef = useRef<string | null>(null);
  entryRef.current = entry?.path ?? null;

  useEffect(() => {
    void ipc.videoTools().then(setTools).catch(() => {});
  }, []);

  /** Repair (re-encode) button in the failure pane. */
  const repairButton =
    entry && !entry.isDir && tab?.location.kind === "local" && tools.ffmpeg ? (
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
    ) : null;

  /** The decoder gave up: show `fallback` now and, with the opt-in ffmpeg
   *  setting on, upgrade to a still frame once one is extracted. */
  const failVideo = (fallback: Preview, isStale: () => boolean) => {
    if (isStale()) return;
    setPreview(fallback);
    const path = entry?.path;
    if (
      !path ||
      tab?.location.kind !== "local" ||
      !(useSettings.getState().config?.qv_ffmpeg_poster ?? false)
    ) {
      return;
    }
    void ipc
      .videoPoster(path)
      .then((buf) => {
        if (isStale() || entryRef.current !== path) return; // cursor moved on
        const url = URL.createObjectURL(new Blob([buf], { type: "image/jpeg" }));
        setPreview({ kind: "video-poster", url });
      })
      .catch(() => {});
  };

  useEffect(() => {
    let revoked: string | null = null;
    let stale = false;
    const isStale = () => stale;
    setTags([]);
    setPdfPages(0);
    setDims(null);
    setDuration(null);
    if (!entry || entry.isDir || !tab) {
      setPreview({ kind: "none" });
      return;
    }
    // "none" (and its "no preview" caption) waits until a load actually
    // comes up empty; until then the pane stays quiet.
    setPreview({ kind: "loading" });

    const timer = setTimeout(async () => {
      if (tab.location.kind === "local") {
        void ipc.fileTags(entry.path).then((v) => {
          if (!stale) setTags(v);
        });
        const isVideo = VIDEO_EXTS.has(entry.ext);
        if (!hex && (isVideo || AUDIO_EXTS.has(entry.ext))) {
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
              // into memory (capped); bigger files stream without analysis.
              if (entry.size <= AUDIO_BLOB_MAX) {
                const buf = await ipc.readPreview(tab.location, entry.path, AUDIO_BLOB_MAX);
                if (stale) return;
                const mime = AUDIO_MIME[entry.ext] ?? "audio/mpeg";
                const url = URL.createObjectURL(new Blob([buf], { type: mime }));
                revoked = url;
                setPreview({ kind: "audio", url, data: buf });
              } else {
                const url = isMac
                  ? convertFileSrc(entry.path, "stream")
                  : await ipc.mediaUrl(entry.path);
                if (stale) return;
                setPreview({ kind: "audio", url, data: null });
              }
              return;
            }
            if (entry.size < VIDEO_BLOB_MIN) {
              const buf = await ipc.readPreview(tab.location, entry.path, VIDEO_BLOB_MIN);
              if (stale) return;
              const mime = VIDEO_MIME[entry.ext] ?? "video/mp4";
              const url = URL.createObjectURL(new Blob([buf], { type: mime }));
              revoked = url;
              setPreview({ kind: "video", url, isStale });
            } else if (isMac) {
              // Local disks and network mounts alike stream through
              // stream:// (range requests, nothing read into memory).
              // Formats the webview can't decode fall back to "none" via
              // onError.
              setPreview({
                kind: "video",
                url: convertFileSrc(entry.path, "stream"),
                isStale,
              });
            } else {
              // WebKitGTK won't range through custom schemes; a localhost
              // HTTP server stands in for stream:// there.
              const url = await ipc.mediaUrl(entry.path);
              if (stale) return;
              setPreview({ kind: "video", url, isStale });
            }
          } catch {
            if (!stale) setPreview({ kind: "none" });
          }
          return;
        }
      }
      try {
        if (hex) {
          const buf = await ipc.readPreview(tab.location, entry.path, HEX_MAX);
          if (stale) return;
          setPreview({ kind: "hex", text: hexDump(new Uint8Array(buf), entry.size > HEX_MAX) });
        } else if (entry.ext === "pdf" && entry.size <= PDF_MAX) {
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
          const text = decodeText(buf);
          setPreview(text === null ? { kind: "none" } : { kind: "text", text });
        } else {
          // Nothing knows how to preview this type.
          setPreview({ kind: "none" });
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
  }, [entry?.path, entry?.modified, tab?.location, hex]);

  const isJsonText = entry?.ext === "json" && preview.kind === "text";
  const isHtmlText = !!entry && HTML_EXTS.has(entry.ext) && preview.kind === "text";
  // A file past the read cap is cut off and can't parse; format it anyway.
  const jsonOk = useMemo(
    () =>
      isJsonText && preview.kind === "text"
        ? (entry?.size ?? 0) > TEXT_MAX || isJson(preview.text)
        : false,
    [isJsonText, preview, entry?.size],
  );
  /** Formatted JSON while "Pretty" is on, else null (show the raw text). */
  const prettyText = useMemo(
    () =>
      preview.kind === "text" && pretty && isJsonText && jsonOk
        ? prettyJson(preview.text)
        : null,
    [preview, pretty, isJsonText, jsonOk],
  );

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
        <span className="title">
          {t("quickview.title")}
          {hex ? " · Hex" : ""}
        </span>
        {isJsonText && (
          <button
            className={pretty && jsonOk ? "tbtn on" : "tbtn"}
            disabled={!jsonOk}
            aria-pressed={pretty && jsonOk}
            title={jsonOk ? t("quickview.prettyTitle") : t("quickview.invalidJson")}
            onClick={() => setPretty((p) => !p)}
          >
            Pretty
          </button>
        )}
        {isHtmlText && (
          <button
            className={htmlView ? "tbtn on" : "tbtn"}
            aria-pressed={htmlView}
            title={t("quickview.htmlViewTitle")}
            onClick={() => setHtmlView((v) => !v)}
          >
            View
          </button>
        )}
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
          <VideoPreview
            key={preview.url}
            url={preview.url}
            autoPlay={autoplay}
            isStale={preview.isStale}
            onMeta={(v) => {
              if (v.videoWidth > 0 && v.videoHeight > 0) {
                setDims((d) => d ?? `${v.videoWidth} × ${v.videoHeight}`);
              }
              if (Number.isFinite(v.duration)) {
                setDuration((d) => d ?? formatDuration(v.duration));
              }
            }}
            onFail={(fallback) => failVideo(fallback, preview.isStale)}
          />
        ) : preview.kind === "video-failed" || preview.kind === "video-poster" ? (
          <div className="qv-fail">
            {preview.kind === "video-poster" ? (
              <img className="qv-poster" src={preview.url} alt={entry.name} draggable={false} />
            ) : null}
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
              {repairButton}
            </div>
          </div>
        ) : preview.kind === "audio" ? (
          <AudioPreview
            url={preview.url}
            data={preview.data}
            onDuration={(secs) => setDuration((d) => d ?? formatDuration(secs))}
          />
        ) : preview.kind === "image" ? (
          // Not draggable: the blob: URL is same-origin with the app, and
          // nothing should ever carry it to a navigation (an SVG there would
          // be a document of the app's own origin).
          <img
            src={preview.url}
            alt={entry.name}
            draggable={false}
            onLoad={(e) => {
              const img = e.currentTarget;
              // SVGs without intrinsic size report 0 — show nothing then.
              if (img.naturalWidth > 0 && img.naturalHeight > 0) {
                setDims(`${img.naturalWidth} × ${img.naturalHeight}`);
              }
            }}
          />
        ) : preview.kind === "text" ? (
          prettyText !== null ? (
            <JsonPreview text={prettyText} />
          ) : isHtmlText && htmlView ? (
            <HtmlPreview
              html={preview.text}
              baseDir={tab.location.kind === "local" ? parentPath(entry.path) : null}
            />
          ) : (
            <pre>{preview.text}</pre>
          )
        ) : preview.kind === "hex" ? (
          <div className="qv-hex">
            <div className="qv-hex-head">{HEX_HEAD}</div>
            <pre>{preview.text}</pre>
          </div>
        ) : preview.kind === "loading" ? (
          <span className="none">…</span>
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
