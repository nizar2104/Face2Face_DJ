// Canvas 2D drawing for the two waveform views (brief 7.7). Both take the wire
// format directly: 3 bytes per frame/point = low, mid, high (0-255).
//
// Sizes are in device pixels; the caller has already sized the canvas for the
// display's pixel ratio.

export interface WaveColours {
  low: string;
  mid: string;
  high: string;
  playhead: string;
  baseline: string;
  /** Painted over the part of the overview that has already played. */
  playedDim: string;
}

/** Defaults from the brief: low = blue, mid = orange, high = white. */
export const DEFAULT_COLOURS: WaveColours = {
  low: '#2f6dff',
  mid: '#ff8c1a',
  high: '#f5f5f5',
  playhead: '#ff3b30',
  baseline: '#2a2f38',
  playedDim: 'rgba(11, 13, 16, 0.68)',
};

/** Relative height of each band, so the three stay readable when layered. */
const BAND_SCALE = [1.0, 0.8, 0.55] as const;

// Scratch buffers reused across frames (one per band), grown on demand.
let scratch: [Uint8Array, Uint8Array, Uint8Array] = [new Uint8Array(0), new Uint8Array(0), new Uint8Array(0)];
function buffers(width: number): [Uint8Array, Uint8Array, Uint8Array] {
  if (scratch[0].length < width) {
    scratch = [new Uint8Array(width), new Uint8Array(width), new Uint8Array(width)];
  }
  return scratch;
}

/**
 * Which detail frames a screen column shows. Columns are numbered on a global
 * grid (column 0 starts at track time 0), so a column keeps exactly the same
 * frames, and therefore the same height, as it scrolls across the window.
 */
export function framesForColumn(column: number, pxPerSec: number, fps: number): [number, number] {
  const first = Math.floor((column * fps) / pxPerSec);
  const end = Math.floor(((column + 1) * fps) / pxPerSec);
  return [first, Math.max(end, first + 1)];
}

/**
 * The zoomed, scrolling waveform: `halfSpanS` seconds either side of a fixed
 * centre playhead.
 *
 * @param detail      frames * 3 bytes
 * @param filledBytes how much of `detail` has arrived so far
 */
export function drawZoomed(
  ctx: CanvasRenderingContext2D,
  width: number,
  height: number,
  detail: Uint8Array,
  filledBytes: number,
  fps: number,
  posS: number,
  halfSpanS: number,
  colours: WaveColours = DEFAULT_COLOURS,
): void {
  ctx.clearRect(0, 0, width, height);
  const mid = height / 2;
  ctx.fillStyle = colours.baseline;
  ctx.fillRect(0, Math.floor(mid), width, 1);

  const pxPerSec = width / (2 * halfSpanS);
  const centreX = Math.floor(width / 2);
  const centreColumn = Math.round(posS * pxPerSec);
  const frames = Math.floor(Math.min(filledBytes, detail.length) / 3);
  const [low, midBand, high] = buffers(width);

  for (let x = 0; x < width; x++) {
    let [f0, f1] = framesForColumn(centreColumn + x - centreX, pxPerSec, fps);
    let l = 0;
    let m = 0;
    let h = 0;
    if (f1 > 0 && f0 < frames) {
      if (f0 < 0) f0 = 0;
      if (f1 > frames) f1 = frames;
      for (let f = f0; f < f1; f++) {
        const i = f * 3;
        if (detail[i]! > l) l = detail[i]!;
        if (detail[i + 1]! > m) m = detail[i + 1]!;
        if (detail[i + 2]! > h) h = detail[i + 2]!;
      }
    }
    low[x] = l;
    midBand[x] = m;
    high[x] = h;
  }

  const bands = [low, midBand, high] as const;
  const fills = [colours.low, colours.mid, colours.high] as const;
  for (let b = 0; b < 3; b++) {
    const values = bands[b]!;
    const k = (mid * BAND_SCALE[b]!) / 255;
    ctx.fillStyle = fills[b]!;
    for (let x = 0; x < width; x++) {
      const v = values[x]!;
      if (v === 0) continue;
      const half = Math.max(1, v * k);
      ctx.fillRect(x, mid - half, 1, half * 2);
    }
  }

  const lineW = Math.max(2, Math.round(width / 480));
  ctx.fillStyle = colours.playhead;
  ctx.fillRect(centreX - Math.floor(lineW / 2), 0, lineW, height);
}

/**
 * The whole-track bar: overview points scaled to the width, the played part
 * dimmed, and a playhead.
 *
 * @param progress played fraction of the track, 0..1
 */
export function drawOverview(
  ctx: CanvasRenderingContext2D,
  width: number,
  height: number,
  overview: Uint8Array,
  filledBytes: number,
  progress: number,
  colours: WaveColours = DEFAULT_COLOURS,
): void {
  ctx.clearRect(0, 0, width, height);
  const points = Math.floor(Math.min(filledBytes, overview.length) / 3);
  const total = Math.floor(overview.length / 3);
  const fills = [colours.low, colours.mid, colours.high] as const;

  if (points > 0 && total > 0) {
    for (let b = 0; b < 3; b++) {
      const k = (height * BAND_SCALE[b]!) / 255;
      ctx.fillStyle = fills[b]!;
      for (let x = 0; x < width; x++) {
        const p0 = Math.floor((x * total) / width);
        const p1 = Math.min(points, Math.max(p0 + 1, Math.floor(((x + 1) * total) / width)));
        let v = 0;
        for (let p = p0; p < p1; p++) {
          const s = overview[p * 3 + b]!;
          if (s > v) v = s;
        }
        if (v === 0) continue;
        const h = Math.max(1, v * k);
        ctx.fillRect(x, height - h, 1, h);
      }
    }
  }

  const clamped = Math.min(Math.max(progress, 0), 1);
  const playX = Math.round(clamped * width);
  ctx.fillStyle = colours.playedDim;
  ctx.fillRect(0, 0, playX, height);
  const lineW = Math.max(2, Math.round(width / 480));
  ctx.fillStyle = colours.playhead;
  ctx.fillRect(Math.min(width - lineW, Math.max(0, playX - Math.floor(lineW / 2))), 0, lineW, height);
}

/** Remaining time as the DJ software shows it: "-03:42". */
export function formatRemaining(durationS: number, posS: number): string {
  const left = Math.max(0, Math.ceil(durationS - posS));
  const m = Math.floor(left / 60);
  const s = left % 60;
  return `-${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
}
