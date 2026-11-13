import { ZOOM_PRESETS_S } from '../settings.ts';
import type { Layout, Settings } from '../settings.ts';
import type { ViewerStore } from '../state/store.ts';

interface Props {
  store: ViewerStore;
  settings: Settings;
  /** The Sender is on another computer. */
  remote?: boolean;
  onChange: (patch: Partial<Settings>) => void;
  onSwitchRole?: () => void;
  /** Opens the partner set-up, where there is one. */
  onSetUp?: () => void;
}

const LAYOUTS: { value: Layout; label: string; title: string }[] = [
  { value: 'auto', label: 'A', title: 'Layout: follow the DJ software (2 or 4 decks)' },
  { value: 'compact', label: 'M', title: 'Layout: master deck only' },
  { value: '2', label: '2', title: 'Layout: 2 decks' },
  { value: '4', label: '4', title: 'Layout: 4 decks' },
];

export function StatusBar({ store, settings, remote = false, onChange, onSwitchRole, onSetUp }: Props) {
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
        {onSetUp && (
          <button className="seg-single" title="Partner set-up: address and pairing code" onClick={onSetUp}>
            Partner
          </button>
        )}
        {onSwitchRole && (
          <button className="seg-single" title="Back to the start screen" onClick={onSwitchRole}>
            Home
          </button>
        )}
      </span>
      <span className={`link ${connected ? 'link-up' : 'link-down'}`}>
        <span className="dot">●</span>
        {/* "Local" is a Sender on this computer. Telling cable from Wi-Fi is still to come. */}
        {connected ? ` ${remote ? 'Network' : 'Local'} ${latency !== null ? `${latency.toFixed(1)} ms` : '…'}` : ' Offline'}
      </span>
    </header>
  );
}
