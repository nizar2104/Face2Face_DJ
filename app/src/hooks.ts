import { useEffect, useRef, useSyncExternalStore } from 'react';
import type { ViewerStore } from './state/store.ts';

/** Re-renders the component whenever the store announces a visible change. */
export function useStoreVersion(store: ViewerStore): number {
  return useSyncExternalStore(store.subscribe, store.getVersion);
}

/** Calls `draw` once per display frame while the component is mounted. */
export function useAnimationFrame(draw: () => void): void {
  const latest = useRef(draw);
  useEffect(() => {
    latest.current = draw;
  });
  useEffect(() => {
    let id = requestAnimationFrame(function loop() {
      latest.current();
      id = requestAnimationFrame(loop);
    });
    return () => cancelAnimationFrame(id);
  }, []);
}
