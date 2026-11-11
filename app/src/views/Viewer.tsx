import { useEffect, useState } from 'react';
import { DeckCard } from '../components/DeckCard.tsx';
import { StatusBar } from '../components/StatusBar.tsx';
import { useStoreVersion } from '../hooks.ts';
import { ViewerConnection } from '../net/connection.ts';
import { useSettings } from '../settings.ts';
import type { Layout } from '../settings.ts';
import { ViewerStore } from '../state/store.ts';
import { APP_VERSION } from '../version.ts';

function deviceId(): string {
  const key = 'tandem.deviceId';
  const make = (): string =>
    typeof crypto.randomUUID === 'function' ? crypto.randomUUID() : `viewer-${Math.random().toString(16).slice(2)}${Date.now().toString(16)}`;
  try {
    const saved = localStorage.getItem(key);
    if (saved !== null && saved !== '') return saved;
    const id = make();
    localStorage.setItem(key, id);
    return id;
  } catch {
    return make();
  }
}

function osName(): string {
  const ua = navigator.userAgent;
  if (ua.includes('Windows')) return 'windows';
  if (ua.includes('Mac')) return 'macos';
  return 'other';
}

function decksToShow(store: ViewerStore, layout: Layout): number[] {
  switch (layout) {
    case 'compact':
      return [store.masterDeck() ?? [...store.decks.values()].find((d) => d.track !== null)?.deck ?? 1];
    case '2':
      return [1, 2];
    case '4':
      return [1, 2, 3, 4];
    case 'auto':
      return store.deckNumbers();
  }
}

interface Props {
  /** ws:// address of the Sender. */
  senderUrl: string;
  onSwitchRole?: () => void;
}

export function Viewer({ senderUrl, onSwitchRole }: Props) {
  const [store] = useState(() => new ViewerStore());
  const [settings, updateSettings] = useSettings();
  useStoreVersion(store);

  useEffect(() => {
    const conn = new ViewerConnection(store, {
      url: senderUrl,
      deviceId: deviceId(),
      name: 'Tandem Viewer',
      appVersion: APP_VERSION,
      os: osName(),
    });
    conn.start();
    return () => conn.stop();
  }, [store, senderUrl]);

  const bar = <StatusBar store={store} settings={settings} onChange={updateSettings} onSwitchRole={onSwitchRole} />;

  if (!store.everConnected) {
    return (
      <div className="viewer">
        {bar}
        <div className="fullscreen-notice">
          <p className="notice-title">Waiting for computer 1</p>
          <p className="notice-detail">{store.error !== null ? store.error.message : `Looking for a Sender at ${senderUrl.replace(/^ws:\/\//, '')}…`}</p>
        </div>
      </div>
    );
  }

  const stale = store.connection !== 'connected';
  const decks = decksToShow(store, settings.layout);
  return (
    <div className={`viewer${stale ? ' viewer-stale' : ''}`}>
      {bar}
      {stale && <div className="banner">Disconnected – showing last known. Reconnecting…</div>}
      {!stale && store.error !== null && <div className="banner banner-warn">{store.error.message}</div>}
      {!stale && store.source !== null && !store.source.connected && (
        <div className="banner banner-warn">Connected – DJ software not detected</div>
      )}
      <main className={`decks decks-${decks.length}`}>
        {decks.map((n) => (
          <DeckCard key={n} store={store} deck={n} notation={settings.notation} zoomS={settings.zoomS} />
        ))}
      </main>
    </div>
  );
}
