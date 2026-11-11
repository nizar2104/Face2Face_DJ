import { useRef } from 'react';
import { useAnimationFrame } from '../hooks.ts';
import type { ViewerStore } from '../state/store.ts';
import { drawOverview, drawZoomed } from '../waveform/render.ts';

/** Sizes the canvas backing store to its CSS box at the display's pixel ratio. */
function fit(canvas: HTMLCanvasElement): CanvasRenderingContext2D | null {
  const dpr = window.devicePixelRatio || 1;
  const w = Math.max(1, Math.round(canvas.clientWidth * dpr));
  const h = Math.max(1, Math.round(canvas.clientHeight * dpr));
  if (canvas.width !== w || canvas.height !== h) {
    canvas.width = w;
    canvas.height = h;
  }
  return canvas.getContext('2d');
}

interface Props {
  store: ViewerStore;
  deck: number;
}

/** Zoomed waveform with a fixed centre playhead, redrawn every display frame. */
export function WaveformView({ store, deck, zoomS }: Props & { zoomS: number }) {
  const ref = useRef<HTMLCanvasElement>(null);
  useAnimationFrame(() => {
    const canvas = ref.current;
    const d = store.decks.get(deck);
    if (canvas === null || d === undefined || d.waveform === null) return;
    const wf = store.waveforms.peek(d.waveform.waveform_id);
    const pos = store.position(deck);
    const ctx = fit(canvas);
    if (ctx === null || wf === null || wf.detail === null) return;
    drawZoomed(ctx, canvas.width, canvas.height, wf.detail, wf.detailFilled, d.waveform.detail_fps, pos ?? 0, zoomS);
  });
  return <canvas ref={ref} className="wave-canvas" />;
}

/** Whole-track overview with the played part dimmed. */
export function OverviewBar({ store, deck }: Props) {
  const ref = useRef<HTMLCanvasElement>(null);
  useAnimationFrame(() => {
    const canvas = ref.current;
    const d = store.decks.get(deck);
    if (canvas === null) return;
    const ctx = fit(canvas);
    if (ctx === null) return;
    const wf = d !== undefined && d.waveform !== null ? store.waveforms.peek(d.waveform.waveform_id) : null;
    if (d === undefined || d.track === null || wf === null || wf.overview === null) {
      ctx.clearRect(0, 0, canvas.width, canvas.height);
      return;
    }
    const pos = store.position(deck) ?? 0;
    const progress = d.track.duration_s > 0 ? pos / d.track.duration_s : 0;
    drawOverview(ctx, canvas.width, canvas.height, wf.overview, wf.overviewFilled, progress);
  });
  return <canvas ref={ref} className="overview-canvas" />;
}
