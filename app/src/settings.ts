// Viewer display settings, kept in localStorage. Milestone 0 has three.

import { useCallback, useState } from 'react';

export type KeyNotation = 'camelot' | 'musical';
/** 'auto' follows the deck count the DJ software reports. */
export type Layout = 'auto' | 'compact' | '2' | '4';
export const ZOOM_PRESETS_S = [4, 8, 16] as const;

export interface Settings {
  notation: KeyNotation;
  layout: Layout;
  /** Half-width of the zoomed waveform in seconds (the "±8 s" preset). */
  zoomS: number;
}

const DEFAULTS: Settings = { notation: 'camelot', layout: 'auto', zoomS: 8 };
const STORAGE_KEY = 'face2face-dj.viewer.settings';

function load(): Settings {
  try {
    const raw: unknown = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '{}');
    if (typeof raw !== 'object' || raw === null) return DEFAULTS;
    const r = raw as Record<string, unknown>;
    return {
      notation: r['notation'] === 'musical' ? 'musical' : 'camelot',
      layout: r['layout'] === 'compact' || r['layout'] === '2' || r['layout'] === '4' ? r['layout'] : 'auto',
      zoomS: ZOOM_PRESETS_S.find((z) => z === r['zoomS']) ?? DEFAULTS.zoomS,
    };
  } catch {
    return DEFAULTS;
  }
}

export function useSettings(): [Settings, (patch: Partial<Settings>) => void] {
  const [settings, setSettings] = useState<Settings>(load);
  const update = useCallback((patch: Partial<Settings>) => {
    setSettings((prev) => {
      const next = { ...prev, ...patch };
      try {
        localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
      } catch {
        // storage unavailable: the setting just does not persist
      }
      return next;
    });
  }, []);
  return [settings, update];
}
