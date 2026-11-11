import { ZOOM_PRESETS_S } from '../settings.ts';
import type { Layout, Settings } from '../settings.ts';
import type { ViewerStore } from '../state/store.ts';

interface Props {
  store: ViewerStore;
  settings: Settings;
  onChange: (patch: Partial<Settings>) => void;
  onSwitchRole?: () => void;
}

const LAYOUTS: { value: Layout; label: string; title: string }[] = [
  { value: 'auto', label: 'A', title: 'Layout: follow the DJ software (2 or 4 decks)' },
  { value: 'compact', label: 'M', title: 'Layout: master deck only' },
  { value: '2', label: '2', title: 'Layout: 2 decks' },
  { value: '4', label: '4', title: 'Layout: 4 decks' },
];

export function StatusBar({ store, settings, onChange, onSwitchRole }: Props) {
  const connected = store.connection === 'connected';
  const latency = store.clock.latencyMs;
  const source = store.source;
  const from = store.sender !== null ? `From ${store.sender.name}` : 'Face2Face DJ';
  const software = source !== null ? ` · ${source.software} ${source.software_version}` : '';

  return (
    <header className="status">
      <span className="status-from" title={from + software}>
        {from}
        <span className="status-software">{software}</span>
      </span>
      <span className="controls">
        <span className="seg" role="group" aria-label="Deck layout">
          {LAYOUTS.map((l) => (
            <button key={l.value} className={settings.layout === l.value ? 'on' : ''} title={l.title} onClick={() => onChange({ layout: l.value })}>
              {l.label}
            </button>
          ))}
        </span>
        <span className="seg" role="group" aria-label="Waveform zoom">
          {ZOOM_PRESETS_S.map((z) => (
            <button key={z} className={settings.zoomS === z ? 'on' : ''} title={`Zoom: ${z} seconds either side of the playhead`} onClick={() => onChange({ zoomS: z })}>
              ±{z}
            </button>
          ))}
        </span>
        <button
          className="seg-single"
          title="Key notation: Camelot or musical"
          onClick={() => onChange({ notation: settings.notation === 'camelot' ? 'musical' : 'camelot' })}
        >
          {settings.notation === 'camelot' ? '8A' : 'Am'}
        </button>
        {onSwitchRole && (
          <button className="seg-single" title="Switch role" onClick={onSwitchRole}>
            ⇄
          </button>
        )}
      </span>
      <span className={`link ${connected ? 'link-up' : 'link-down'}`}>
        <span className="dot">●</span>
        {/* Milestone 0 only ever connects over localhost; cable / Wi-Fi detection is Milestone 1. */}
        {connected ? ` Local ${latency !== null ? `${latency.toFixed(1)} ms` : '…'}` : ' Offline'}
      </span>
    </header>
  );
}
