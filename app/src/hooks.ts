import { useEffect, useRef, useSyncExternalStore } from 'react';
import type { ViewerStore } from './state/store.ts';

/** Re-renders the component whenever the store announces a visible change. */
export function useStoreVersion(store: ViewerStore): number {
  return useSyncExternalStore(store.subscribe, store.getVersion);
}

/**
 * How often the waveforms and time readouts are redrawn. Displays refresh 60
 * to 120 times a second; redrawing every waveform that often keeps a laptop
 * that is also running DJ software busier than a monitoring window should.
 */
export const DRAW_FPS = 30;

/** Calls `draw` up to `DRAW_FPS` times a second while the component is mounted. */
export function useAnimationFrame(draw: () => void): void {
  const latest = useRef(draw);
  useEffect(() => {
    latest.current = draw;
  });
  useEffect(() => {
    // Display frames arrive a little early or late, hence the 2 ms of slack:
    // without it a 60 Hz display would draw at 20 a second instead of 30.
    const minGapMs = 1000 / DRAW_FPS - 2;
    let last = -Infinity;
    let id = requestAnimationFrame(function loop(now: number) {
      if (now - last >= minGapMs) {
        last = now;
        latest.current();
      }
      id = requestAnimationFrame(loop);
    });
    return () => cancelAnimationFrame(id);
  }, []);
}
