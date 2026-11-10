import { useVirtualizer } from "@tanstack/react-virtual";
import clsx from "clsx";
import { renderAsync } from "docx-preview";
import JSZip from "jszip";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { Sheet, Workbook } from "../ipc/types";
import { useScaled } from "../lib/fontScale";
import { HtmlPreview } from "./HtmlPreview";

/* ── Word ──────────────────────────────────────────────────────────── */

/** Page look inside the sandboxed frame: grey desk, white sheets. */
const DOCX_FRAME_CSS = `
html, body { margin: 0; background: #e8e8ea; }
.docx-wrapper { background: transparent !important; padding: 24px !important; }
.docx-wrapper > section.docx { box-shadow: 0 1px 6px rgba(0,0,0,.25) !important; margin-bottom: 24px !important; }
`;

/** CSS length of the first page ("612pt", "8.5in", "816px") in px. */
function pageWidthPx(width: string | undefined): number | null {
  const m = /^([\d.]+)(pt|px|in|cm|mm)$/.exec(width?.trim() ?? "");
  if (!m) return null;
  const n = Number(m[1]);
  const per = { pt: 96 / 72, px: 1, in: 96, cm: 96 / 2.54, mm: 96 / 25.4 }[m[2] as "pt"];
  return n * per;
}

/** A .docx laid out as pages by docx-preview. It renders into detached
 *  elements; only the resulting markup is shown, in HtmlPreview's sandbox
 *  (no scripts, no same-origin access, nothing remote). Images and fonts
 *  are inlined as data: URLs, the one kind of URL the sandbox can load.
 *  The frame keeps the page's real width and is scaled down to fit. */
export function DocxPreview({ data }: { data: ArrayBuffer }) {
  const { t } = useTranslation();
  const [doc, setDoc] = useState<{ html: string; width: number } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const boxRef = useRef<HTMLDivElement>(null);
  const [box, setBox] = useState({ w: 0, h: 0 });

  useEffect(() => {
    let gone = false;
    setDoc(null);
    setError(null);
    const body = document.createElement("div");
    const style = document.createElement("div");
    renderAsync(data.slice(0), body, style, {
      className: "docx",
      inWrapper: true,
      breakPages: true,
      ignoreLastRenderedPageBreak: true,
      useBase64URL: true,
      experimental: false,
      renderChanges: false,
      renderComments: false,
      // altChunks are embedded HTML/RTF parts; nothing to gain from them.
      renderAltChunks: false,
    }).then(
      () => {
        if (gone) return;
        const page = body.querySelector<HTMLElement>("section.docx");
        const width = (pageWidthPx(page?.style.width) ?? 816) + 48;
        setDoc({
          html: `<!doctype html><html><head>${style.innerHTML}<style>${DOCX_FRAME_CSS}</style></head><body>${body.innerHTML}</body></html>`,
          width,
        });
      },
      (e) => !gone && setError(String(e?.message ?? e)),
    );
    return () => {
      gone = true;
    };
  }, [data]);

  useLayoutEffect(() => {
    const el = boxRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setBox({ w: el.clientWidth, h: el.clientHeight }));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const scale = doc && box.w > 0 ? Math.min(1, box.w / doc.width) : 1;
  return (
    <div className="qv-docx" ref={boxRef}>
      {error ? (
        <span className="none">{t("quickview.officeError", { error })}</span>
      ) : !doc ? (
        <span className="none">…</span>
      ) : (
        // The outer box takes the scaled size in layout (so it centers); the
        // inner one is the real page width, scaled into it.
        <div className="qv-docx-frame" style={{ width: doc.width * scale, height: box.h }}>
          <div
            className="qv-docx-page"
            style={{ width: doc.width, height: box.h / scale, transform: `scale(${scale})` }}
          >
            <HtmlPreview html={doc.html} baseDir={null} />
          </div>
        </div>
      )}
    </div>
  );
}

/* ── PowerPoint ────────────────────────────────────────────────────── */

const NS_A = "http://schemas.openxmlformats.org/drawingml/2006/main";
const NS_P = "http://schemas.openxmlformats.org/presentationml/2006/main";
const NS_R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

const IMAGE_MIME: Record<string, string> = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
};
/** Inlined picture bytes across the whole deck. */
const PPTX_IMAGE_BUDGET = 20 * 1024 * 1024;
const PPTX_SLIDES_MAX = 300;

interface Slide {
  title: string;
  paragraphs: string[];
  images: string[];
  hidden: boolean;
}

/** "slides/slide1.xml" seen from "ppt/presentation.xml" → "ppt/slides/slide1.xml". */
function resolvePart(from: string, target: string): string {
  if (target.startsWith("/")) return target.slice(1);
  const parts = from.split("/").slice(0, -1);
  for (const piece of target.split("/")) {
    if (piece === "..") parts.pop();
    else if (piece && piece !== ".") parts.push(piece);
  }
  return parts.join("/");
}

async function readXml(zip: JSZip, part: string): Promise<Document | null> {
  const file = zip.file(part);
  if (!file) return null;
  const doc = new DOMParser().parseFromString(await file.async("string"), "application/xml");
  return doc.getElementsByTagName("parsererror").length ? null : doc;
}

/** Relationship id → target part of `part` (from its _rels sibling). */
async function readRels(zip: JSZip, part: string): Promise<Map<string, string>> {
  const slash = part.lastIndexOf("/");
  const relsPart = `${part.slice(0, slash)}/_rels/${part.slice(slash + 1)}.rels`;
  const doc = await readXml(zip, relsPart);
  const map = new Map<string, string>();
  if (!doc) return map;
  for (const rel of Array.from(doc.getElementsByTagName("Relationship"))) {
    if (rel.getAttribute("TargetMode") === "External") continue;
    const id = rel.getAttribute("Id");
    const target = rel.getAttribute("Target");
    if (id && target) map.set(id, resolvePart(part, target));
  }
  return map;
}

/** Text and pictures of each slide, in presentation order. Layout and
 *  master content (logos, footers) is left out; so is anything remote. */
async function readDeck(data: ArrayBuffer): Promise<Slide[]> {
  const zip = await JSZip.loadAsync(data);
  const presPart = "ppt/presentation.xml";
  const pres = await readXml(zip, presPart);
  if (!pres) throw new Error("not a PowerPoint presentation");
  const presRels = await readRels(zip, presPart);
  const ids = Array.from(pres.getElementsByTagNameNS(NS_P, "sldId"))
    .map((el) => el.getAttributeNS(NS_R, "id"))
    .filter((id): id is string => !!id);

  let budget = PPTX_IMAGE_BUDGET;
  const slides: Slide[] = [];
  for (const id of ids.slice(0, PPTX_SLIDES_MAX)) {
    const part = presRels.get(id);
    const doc = part ? await readXml(zip, part) : null;
    if (!part || !doc) continue;
    const slide: Slide = {
      title: "",
      paragraphs: [],
      images: [],
      hidden: doc.documentElement.getAttribute("show") === "0",
    };
    for (const shape of Array.from(doc.getElementsByTagNameNS(NS_P, "sp"))) {
      const ph = shape.getElementsByTagNameNS(NS_P, "ph")[0]?.getAttribute("type");
      const isTitle = ph === "title" || ph === "ctrTitle";
      for (const p of Array.from(shape.getElementsByTagNameNS(NS_A, "p"))) {
        const text = Array.from(p.getElementsByTagNameNS(NS_A, "t"))
          .map((t) => t.textContent ?? "")
          .join("")
          .trim();
        if (!text) continue;
        if (isTitle) slide.title = slide.title ? `${slide.title} ${text}` : text;
        else slide.paragraphs.push(text);
      }
    }
    // Table cells live in graphic frames, not shapes.
    for (const cell of Array.from(doc.getElementsByTagNameNS(NS_A, "tc"))) {
      const text = cell.textContent?.trim();
      if (text) slide.paragraphs.push(text);
    }
    const rels = await readRels(zip, part);
    const seen = new Set<string>();
    for (const blip of Array.from(doc.getElementsByTagNameNS(NS_A, "blip"))) {
      const target = rels.get(blip.getAttributeNS(NS_R, "embed") ?? "");
      if (!target || seen.has(target)) continue;
      seen.add(target);
      const mime = IMAGE_MIME[target.split(".").pop()?.toLowerCase() ?? ""];
      const file = zip.file(target);
      if (!mime || !file) continue;
      const b64 = await file.async("base64");
      budget -= b64.length;
      if (budget < 0) break;
      slide.images.push(`data:${mime};base64,${b64}`);
    }
    slides.push(slide);
  }
  return slides;
}

/** A .pptx as one card per slide: title, text and pictures. Not the
 *  slide's layout (LibreOffice in Settings renders that). */
export function PptxPreview({
  data,
  onSlides,
}: {
  data: ArrayBuffer;
  onSlides: (n: number) => void;
}) {
  const { t } = useTranslation();
  const [slides, setSlides] = useState<Slide[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let gone = false;
    setSlides(null);
    setError(null);
    readDeck(data).then(
      (s) => {
        if (gone) return;
        setSlides(s);
        onSlides(s.length);
      },
      (e) => !gone && setError(String(e?.message ?? e)),
    );
    return () => {
      gone = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [data]);

  if (error) return <span className="none">{t("quickview.officeError", { error })}</span>;
  if (!slides) return <span className="none">…</span>;
  if (slides.length === 0) return <span className="none">{t("quickview.noSlides")}</span>;
  return (
    <div className="qv-slides">
      {slides.map((s, i) => (
        <section key={i} className={clsx("qv-slide", s.hidden && "hidden")}>
          <div className="qv-slide-no">
            {t("quickview.slideNo", { n: i + 1 })}
            {s.hidden && ` · ${t("quickview.slideHidden")}`}
          </div>
          {s.title && <h3>{s.title}</h3>}
          {s.paragraphs.map((p, j) => (
            <p key={j}>{p}</p>
          ))}
          {s.images.length > 0 && (
            <div className="qv-slide-images">
              {s.images.map((src, j) => (
                <img key={j} src={src} alt="" draggable={false} />
              ))}
            </div>
          )}
        </section>
      ))}
    </div>
  );
}

/* ── Spreadsheets ──────────────────────────────────────────────────── */

const SHEET_ROW = 22;
const HEAD_COL = 52;

/** 0 → "A", 25 → "Z", 26 → "AA". */
function colName(i: number): string {
  let s = "";
  for (let n = i + 1; n > 0; n = Math.floor((n - 1) / 26)) {
    s = String.fromCharCode(65 + ((n - 1) % 26)) + s;
  }
  return s;
}

/** East Asian wide characters (CJK, kana, Hangul, full-width forms). */
const WIDE = /[ᄀ-ᅟ⺀-꓏가-힣豈-﫿︰-﹏＀-｠￠-￦]/g;

/** Text width in narrow-character units: wide characters count twice. */
const textUnits = (s: string) => s.length + (s.match(WIDE)?.length ?? 0);

/** Column widths from the longest text in the first rows. */
function columnWidths(sheet: Sheet, scale: number): number[] {
  const cols = sheet.rows.reduce((m, r) => Math.max(m, r.length), 0);
  const widths = new Array<number>(cols).fill(0);
  for (const row of sheet.rows.slice(0, 300)) {
    row.forEach((cell, i) => {
      widths[i] = Math.max(widths[i], textUnits(cell));
    });
  }
  return widths.map((len) => Math.round(Math.min(320, Math.max(56, len * 7.2 + 16)) * scale));
}

function SheetGrid({ sheet }: { sheet: Sheet }) {
  const { t } = useTranslation();
  const rowH = useScaled(SHEET_ROW);
  const headW = useScaled(HEAD_COL);
  const scale = rowH / SHEET_ROW;
  const widths = useMemo(() => columnWidths(sheet, scale), [sheet, scale]);
  const total = headW + widths.reduce((a, b) => a + b, 0);
  const scrollRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: sheet.rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => rowH,
    overscan: 12,
  });
  useEffect(() => virtualizer.measure(), [rowH, virtualizer]);
  useEffect(() => scrollRef.current?.scrollTo(0, 0), [sheet]);

  if (sheet.error) {
    return <span className="none">{t("quickview.officeError", { error: sheet.error })}</span>;
  }
  if (sheet.rows.length === 0) {
    return <span className="none">{t("quickview.sheetEmpty")}</span>;
  }
  const cut =
    sheet.totalRows > sheet.rows.length || sheet.totalCols > widths.length
      ? t("quickview.sheetCut", {
          rows: sheet.rows.length,
          cols: widths.length,
          totalRows: sheet.totalRows,
          totalCols: sheet.totalCols,
        })
      : null;

  const cells = (row: string[]) =>
    widths.map((w, c) => (
      <div key={c} className="sh-cell" style={{ width: w }} title={row[c] || undefined}>
        {row[c]}
      </div>
    ));

  return (
    <>
      {cut && <div className="sheet-cut">{cut}</div>}
      <div className="sheet-scroll" ref={scrollRef}>
        <div className="sheet-head" style={{ width: total, height: rowH }}>
          <div className="sh-corner" style={{ width: headW }} />
          {widths.map((w, c) => (
            <div key={c} className="sh-col" style={{ width: w }}>
              {colName(sheet.firstCol + c)}
            </div>
          ))}
        </div>
        <div style={{ position: "relative", width: total, height: virtualizer.getTotalSize() }}>
          {virtualizer.getVirtualItems().map((v) => (
            <div
              key={v.index}
              className="sheet-row"
              style={{ height: rowH, transform: `translateY(${v.start}px)` }}
            >
              <div className="sh-num" style={{ width: headW }}>
                {sheet.firstRow + v.index + 1}
              </div>
              {cells(sheet.rows[v.index])}
            </div>
          ))}
        </div>
      </div>
    </>
  );
}

/** Workbook as a grid with Excel-style sheet tabs along the bottom. */
export function SheetPreview({ book }: { book: Workbook }) {
  const { t } = useTranslation();
  const [active, setActive] = useState(0);
  useEffect(() => setActive(0), [book]);
  const sheet = book.sheets[Math.min(active, book.sheets.length - 1)];
  if (!sheet) return <span className="none">{t("quickview.sheetEmpty")}</span>;
  return (
    <div className="qv-sheet">
      <SheetGrid sheet={sheet} />
      {book.sheets.length > 1 && (
        <div className="sheet-tabs" role="tablist">
          {book.sheets.map((s, i) => (
            <button
              key={i}
              role="tab"
              aria-selected={i === active}
              className={clsx(i === active && "on")}
              onClick={() => setActive(i)}
            >
              {s.name}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
