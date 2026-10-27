import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import * as ipc from "../ipc/commands";
import { formatDate, humanSize } from "../lib/format";
import { baseName } from "../lib/path";
import { visibleEntries } from "../lib/selectors";
import { usePanels } from "../store/panels";
import { useSettings } from "../store/settings";

const IMAGE_EXTS = new Set(["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "ico"]);
const TEXT_EXTS = new Set([
  "txt", "md", "json", "js", "ts", "tsx", "jsx", "rs", "py", "sh", "zsh", "css",
  "html", "xml", "yml", "yaml", "toml", "ini", "conf", "log", "csv", "swift",
  "c", "h", "cpp", "go", "java", "rb", "sql", "lock",
]);
const IMAGE_MAX = 32 * 1024 * 1024;
const TEXT_MAX = 256 * 1024;

type Preview =
  | { kind: "image"; url: string }
  | { kind: "text"; text: string }
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

  useEffect(() => {
    let revoked: string | null = null;
    let stale = false;
    setPreview({ kind: "none" });
    setTags([]);
    if (!entry || entry.isDir || !tab) return;

    const timer = setTimeout(async () => {
      if (tab.location.kind === "local") {
        void ipc.fileTags(entry.path).then((v) => {
          if (!stale) setTags(v);
        });
      }
      try {
        if (IMAGE_EXTS.has(entry.ext) && entry.size <= IMAGE_MAX) {
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
          ✕ F3
        </button>
      </div>
      <div className="qv-body">
        {!entry ? (
          <span className="none">{t("quickview.noPreview")}</span>
        ) : preview.kind === "image" ? (
          <img src={preview.url} alt={entry.name} />
        ) : preview.kind === "text" ? (
          <pre>{preview.text}</pre>
        ) : (
          <span className="none">{entry.isDir ? "▸ " + entry.name : t("quickview.noPreview")}</span>
        )}
      </div>
      {entry && (
        <>
          <dl className="qv-meta">
            <dt>{t("quickview.kind")}</dt>
            <dd>{kindLabel}</dd>
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
