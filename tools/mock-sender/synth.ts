// Synthetic 3-band waveforms for Milestone 0, in the real wire format:
// 150 frames per second, 3 bytes per frame (low, mid, high), plus a
// 1,200-point overview. The shape imitates a dance track (kick on the beat,
// 16-bar sections, a breakdown) so scrolling and tempo are easy to judge by eye.
// The Rust twin is crates/tandem-core/src/waveform/synth.rs.

import { BYTES_PER_FRAME, DETAIL_FPS, OVERVIEW_POINTS } from '../../app/src/protocol/messages.ts';

export interface SynthWaveform {
  frames: number;
  detail: Uint8Array;
  overview: Uint8Array;
}

/** FNV-1a, 32 bit, over the UTF-8 bytes. Used for mock ids and seeds. */
export function fnv1a32(text: string): number {
  let h = 0x811c9dc5;
  for (const b of new TextEncoder().encode(text)) {
    h ^= b;
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h >>> 0;
}

export function hex8(n: number): string {
  return (n >>> 0).toString(16).padStart(8, '0');
}

// [kick, mid, high] gain per 16-bar section: intro, build, drop, break, drop, outro.
const SECTIONS: readonly (readonly [number, number, number])[] = [
  [0.8, 0.25, 0.3],
  [0.85, 0.5, 0.6],
  [1.0, 0.9, 0.8],
  [0.0, 0.7, 0.35],
  [1.0, 1.0, 0.9],
  [0.8, 0.3, 0.4],
];

function fract(x: number): number {
  return x - Math.floor(x);
}

export function synthWaveform(durationS: number, bpm: number, seed: number): SynthWaveform {
  const frames = Math.max(1, Math.round(durationS * DETAIL_FPS));
  const beatLen = 60 / (bpm > 0 ? bpm : 120);
  const values = new Float32Array(frames * BYTES_PER_FRAME);
  let rng = seed >>> 0 || 0x9e3779b9; // xorshift32 must not start at 0
  let peak = 0;

  for (let i = 0; i < frames; i++) {
    const beat = i / DETAIL_FPS / beatLen;
    const [kickG, midG, highG] = SECTIONS[Math.floor(beat / 64) % SECTIONS.length]!;

    rng ^= rng << 13;
    rng >>>= 0;
    rng ^= rng >>> 17;
    rng ^= rng << 5;
    rng >>>= 0;
    const noise = rng / 4294967296;

    const kick = Math.exp(-fract(beat) * 7);
    const bass = Math.exp(-fract(beat + 0.5) * 5);
    const hat = Math.exp(-fract(beat * 4) * 9);

    const low = kickG * kick + 0.15 * midG * bass;
    const mid = midG * (0.55 * bass + 0.25 + 0.2 * noise);
    const high = highG * (0.6 * hat + 0.4 * noise);

    values[i * 3] = low;
    values[i * 3 + 1] = mid;
    values[i * 3 + 2] = high;
    peak = Math.max(peak, low, mid, high);
  }

  // Normalise so the loudest band value in the track is 255 (brief 7.5).
  const scale = peak > 0 ? 255 / peak : 0;
  const detail = new Uint8Array(values.length);
  for (let i = 0; i < values.length; i++) detail[i] = Math.round(values[i]! * scale);

  return { frames, detail, overview: buildOverview(detail, frames) };
}

/** Overview point j holds the per-band maximum of the frames it covers. */
export function buildOverview(detail: Uint8Array, frames: number): Uint8Array {
  const overview = new Uint8Array(OVERVIEW_POINTS * BYTES_PER_FRAME);
  for (let j = 0; j < OVERVIEW_POINTS; j++) {
    const start = Math.min(frames - 1, Math.floor((j * frames) / OVERVIEW_POINTS));
    const end = Math.min(frames, Math.max(start + 1, Math.floor(((j + 1) * frames) / OVERVIEW_POINTS)));
    for (let band = 0; band < BYTES_PER_FRAME; band++) {
      let m = 0;
      for (let f = start; f < end; f++) m = Math.max(m, detail[f * 3 + band]!);
      overview[j * 3 + band] = m;
    }
  }
  return overview;
}
