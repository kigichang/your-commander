import type { PDFDocumentProxy } from "pdfjs-dist";
import * as pdfjs from "pdfjs-dist";
import workerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";
import { useEffect, useRef, useState } from "react";

pdfjs.GlobalWorkerOptions.workerSrc = workerUrl;

/** Canvas-rendered PDF with ‹ page › navigation (design screen 3d). */
export function PdfPreview({
  data,
  onPages,
}: {
  data: ArrayBuffer;
  onPages: (n: number) => void;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const boxRef = useRef<HTMLDivElement>(null);
  const docRef = useRef<PDFDocumentProxy | null>(null);
  const [page, setPage] = useState(1);
  const [pages, setPages] = useState(0);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let gone = false;
    setPage(1);
    setPages(0);
    setError(null);
    const task = pdfjs.getDocument({ data: new Uint8Array(data.slice(0)) });
    task.promise.then(
      (doc) => {
        // When already stale, the cleanup's task.destroy() tears it down.
        if (gone) return;
        docRef.current = doc;
        setPages(doc.numPages);
        onPages(doc.numPages);
      },
      (e) => !gone && setError(String(e?.message ?? e)),
    );
    return () => {
      gone = true;
      void task.destroy();
      docRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [data]);

  useEffect(() => {
    const doc = docRef.current;
    const canvas = canvasRef.current;
    if (!doc || !canvas || pages === 0) return;
    let gone = false;
    void (async () => {
      try {
        const pdfPage = await doc.getPage(page);
        if (gone) return;
        const boxWidth = boxRef.current?.clientWidth ?? 600;
        const base = pdfPage.getViewport({ scale: 1 });
        const scale = (boxWidth - 8) / base.width;
        const dpr = window.devicePixelRatio || 1;
        const viewport = pdfPage.getViewport({ scale });
        canvas.width = Math.floor(viewport.width * dpr);
        canvas.height = Math.floor(viewport.height * dpr);
        canvas.style.width = `${Math.floor(viewport.width)}px`;
        canvas.style.height = `${Math.floor(viewport.height)}px`;
        const ctx = canvas.getContext("2d");
        if (!ctx) return;
        await pdfPage.render({
          canvas,
          canvasContext: ctx,
          viewport,
          transform: dpr !== 1 ? [dpr, 0, 0, dpr, 0, 0] : undefined,
        }).promise;
      } catch (e) {
        if (!gone) setError(String(e));
      }
    })();
    return () => {
      gone = true;
    };
  }, [page, pages]);

  if (error) return <span className="none">{error}</span>;

  return (
    <div className="pdf" ref={boxRef}>
      <div className="pdf-page">
        <canvas ref={canvasRef} />
      </div>
      {pages > 1 && (
        <div className="pdf-nav">
          <button disabled={page <= 1} onClick={() => setPage((p) => p - 1)}>
            ‹
          </button>
          <span>
            {page} / {pages}
          </span>
          <button disabled={page >= pages} onClick={() => setPage((p) => p + 1)}>
            ›
          </button>
        </div>
      )}
    </div>
  );
}
