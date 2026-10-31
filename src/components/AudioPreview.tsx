import { useEffect, useRef, useState, type MouseEvent } from "react";
import { useTranslation } from "react-i18next";

/** Internal canvas resolutions; CSS stretches them to the pane. */
const WAVE_W = 1000;
const WAVE_H = 120;
const SPEC_W = 800;
const SPEC_H = 128;
const FFT_SIZE = 1024;

/** In-place iterative radix-2 Cooley-Tukey. */
function fft(re: Float32Array, im: Float32Array) {
  const n = re.length;
  for (let i = 1, j = 0; i < n; i++) {
    let bit = n >> 1;
    for (; j & bit; bit >>= 1) j ^= bit;
    j ^= bit;
    if (i < j) {
      const tr = re[i];
      re[i] = re[j];
      re[j] = tr;
      const ti = im[i];
      im[i] = im[j];
      im[j] = ti;
    }
  }
  for (let len = 2; len <= n; len <<= 1) {
    const ang = (-2 * Math.PI) / len;
    const wr = Math.cos(ang);
    const wi = Math.sin(ang);
    const half = len >> 1;
    for (let i = 0; i < n; i += len) {
      let cr = 1;
      let ci = 0;
      for (let k = 0; k < half; k++) {
        const ur = re[i + k];
        const ui = im[i + k];
        const vr = re[i + k + half] * cr - im[i + k + half] * ci;
        const vi = re[i + k + half] * ci + im[i + k + half] * cr;
        re[i + k] = ur + vr;
        im[i + k] = ui + vi;
        re[i + k + half] = ur - vr;
        im[i + k + half] = ui - vi;
        const nr = cr * wr - ci * wi;
        ci = cr * wi + ci * wr;
        cr = nr;
      }
    }
  }
}

function accentColor(): string {
  const v = getComputedStyle(document.documentElement)
    .getPropertyValue("--color-accent")
    .trim();
  return v || "#8b5cf6";
}

function drawWaveform(canvas: HTMLCanvasElement, data: Float32Array) {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  ctx.clearRect(0, 0, WAVE_W, WAVE_H);
  ctx.fillStyle = accentColor();
  const mid = WAVE_H / 2;
  const perCol = data.length / WAVE_W;
  // Stride keeps a multi-hour file at ≤ ~2000 samples per column.
  const stride = Math.max(1, Math.floor(perCol / 2000));
  for (let x = 0; x < WAVE_W; x++) {
    const start = Math.floor(x * perCol);
    const end = Math.min(data.length, Math.floor((x + 1) * perCol) + 1);
    let min = 1;
    let max = -1;
    for (let i = start; i < end; i += stride) {
      const v = data[i];
      if (v < min) min = v;
      if (v > max) max = v;
    }
    if (min > max) continue;
    const y1 = mid - max * (mid - 2);
    const y2 = mid - min * (mid - 2);
    ctx.fillRect(x, y1, 1, Math.max(1, y2 - y1));
  }
  ctx.globalAlpha = 0.5;
  ctx.fillRect(0, mid, WAVE_W, 1);
  ctx.globalAlpha = 1;
}

/** Inferno-ish ramp: dark → violet → magenta → orange → pale yellow. */
const RAMP: Array<[number, [number, number, number]]> = [
  [0.0, [12, 10, 24]],
  [0.35, [76, 29, 149]],
  [0.6, [192, 38, 211]],
  [0.8, [251, 146, 60]],
  [1.0, [253, 230, 138]],
];

function rampColor(t: number): [number, number, number] {
  for (let i = 1; i < RAMP.length; i++) {
    if (t <= RAMP[i][0]) {
      const [t0, c0] = RAMP[i - 1];
      const [t1, c1] = RAMP[i];
      const f = (t - t0) / (t1 - t0);
      return [
        c0[0] + (c1[0] - c0[0]) * f,
        c0[1] + (c1[1] - c0[1]) * f,
        c0[2] + (c1[2] - c0[2]) * f,
      ];
    }
  }
  return RAMP[RAMP.length - 1][1];
}

function drawSpectrogram(canvas: HTMLCanvasElement, data: Float32Array) {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  const bins = FFT_SIZE / 2;
  const img = ctx.createImageData(SPEC_W, SPEC_H);
  const window = new Float32Array(FFT_SIZE);
  for (let i = 0; i < FFT_SIZE; i++) {
    window[i] = 0.5 - 0.5 * Math.cos((2 * Math.PI * i) / (FFT_SIZE - 1));
  }
  const re = new Float32Array(FFT_SIZE);
  const im = new Float32Array(FFT_SIZE);
  const span = Math.max(1, data.length - FFT_SIZE);
  for (let x = 0; x < SPEC_W; x++) {
    const start = Math.floor((x / SPEC_W) * span);
    for (let i = 0; i < FFT_SIZE; i++) {
      re[i] = (data[start + i] ?? 0) * window[i];
      im[i] = 0;
    }
    fft(re, im);
    for (let y = 0; y < SPEC_H; y++) {
      // Low frequencies at the bottom.
      const bin = 1 + Math.floor(((SPEC_H - 1 - y) / SPEC_H) * (bins - 1));
      const mag = Math.hypot(re[bin], im[bin]) / FFT_SIZE;
      const db = 20 * Math.log10(mag + 1e-9);
      const t = Math.min(1, Math.max(0, (db + 100) / 70)); // -100…-30 dB
      const [r, g, b] = rampColor(t);
      const o = (y * SPEC_W + x) * 4;
      img.data[o] = r;
      img.data[o + 1] = g;
      img.data[o + 2] = b;
      img.data[o + 3] = 255;
    }
  }
  ctx.putImageData(img, 0, 0);
}

function channelLabel(i: number, count: number): string {
  if (count === 1) return "";
  if (count === 2) return i === 0 ? "L" : "R";
  return `Ch ${i + 1}`;
}

/** Playbar on top, then per-channel waveforms, then per-channel
 *  spectrograms; a vertical playhead tracks playback on both blocks.
 *  `data` is the file's bytes (null for files too big to analyze). */
export function AudioPreview({
  url,
  data,
  onDuration,
}: {
  url: string;
  data: ArrayBuffer | null;
  onDuration: (secs: number) => void;
}) {
  const { t } = useTranslation();
  const audioRef = useRef<HTMLAudioElement>(null);
  const waveheadRef = useRef<HTMLDivElement>(null);
  const specheadRef = useRef<HTMLDivElement>(null);
  const [buffer, setBuffer] = useState<AudioBuffer | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let stale = false;
    setBuffer(null);
    setFailed(data === null);
    if (!data) return;
    const ctx = new AudioContext();
    // decodeAudioData detaches its input; hand it a copy.
    ctx
      .decodeAudioData(data.slice(0))
      .then((buf) => {
        if (!stale) setBuffer(buf);
      })
      .catch(() => {
        if (!stale) setFailed(true);
      })
      .finally(() => void ctx.close());
    return () => {
      stale = true;
    };
  }, [data]);

  // The playhead follows playback with rAF; left is a percentage so the
  // canvases can stretch freely.
  useEffect(() => {
    let raf = 0;
    const tick = () => {
      const a = audioRef.current;
      if (a && Number.isFinite(a.duration) && a.duration > 0) {
        const pct = `${(a.currentTime / a.duration) * 100}%`;
        if (waveheadRef.current) waveheadRef.current.style.left = pct;
        if (specheadRef.current) specheadRef.current.style.left = pct;
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, []);

  const seek = (e: MouseEvent<HTMLDivElement>) => {
    const a = audioRef.current;
    if (!a || !Number.isFinite(a.duration)) return;
    const rect = e.currentTarget.getBoundingClientRect();
    a.currentTime = ((e.clientX - rect.left) / rect.width) * a.duration;
  };

  const channels = buffer
    ? Array.from({ length: buffer.numberOfChannels }, (_, i) => i)
    : [];

  return (
    <div className="qv-audio">
      <audio
        ref={audioRef}
        src={url}
        controls
        onLoadedMetadata={(e) => {
          const a = e.currentTarget;
          if (Number.isFinite(a.duration)) onDuration(a.duration);
        }}
      />
      <div className="qa-block" onClick={seek}>
        <div className="qa-title">{t("quickview.waveform")}</div>
        {buffer ? (
          channels.map((c) => (
            <div className="qa-ch" key={c}>
              <canvas
                width={WAVE_W}
                height={WAVE_H}
                ref={(el) => {
                  // Guard so parent re-renders don't redraw (ref callbacks
                  // re-fire every render); a new file remounts the canvas.
                  if (el && !el.dataset.drawn) {
                    el.dataset.drawn = "1";
                    drawWaveform(el, buffer.getChannelData(c));
                  }
                }}
              />
              <span className="qa-label">{channelLabel(c, channels.length)}</span>
            </div>
          ))
        ) : (
          <div className="qa-pending">
            {failed ? t("quickview.noAnalysis") : "…"}
          </div>
        )}
        <div className="qa-playhead" ref={waveheadRef} style={{ left: 0 }} />
      </div>
      <div className="qa-block" onClick={seek}>
        <div className="qa-title">{t("quickview.spectrogram")}</div>
        {buffer ? (
          channels.map((c) => (
            <div className="qa-ch" key={c}>
              <canvas
                width={SPEC_W}
                height={SPEC_H}
                ref={(el) => {
                  if (el && !el.dataset.drawn) {
                    el.dataset.drawn = "1";
                    drawSpectrogram(el, buffer.getChannelData(c));
                  }
                }}
              />
              <span className="qa-label">{channelLabel(c, channels.length)}</span>
            </div>
          ))
        ) : (
          <div className="qa-pending">
            {failed ? t("quickview.noAnalysis") : "…"}
          </div>
        )}
        <div className="qa-playhead" ref={specheadRef} style={{ left: 0 }} />
      </div>
    </div>
  );
}
