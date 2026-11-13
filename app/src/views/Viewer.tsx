import { useEffect, useState } from 'react';
import { DeckCard } from '../components/DeckCard.tsx';
import { StatusBar } from '../components/StatusBar.tsx';
import { useStoreVersion } from '../hooks.ts';
import { ViewerConnection } from '../net/connection.ts';
import { isLocalUrl } from '../pairing.ts';
import { useSettings } from '../settings.ts';
import type { Layout } from '../settings.ts';
import { ViewerStore } from '../state/store.ts';
import { APP_VERSION } from '../version.ts';

function deviceId(): string {
  const key = 'face2face-dj.deviceId';
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
  /** The Sender's pairing code, if it needs one. */
  pairCode?: string;
  /** Opens the partner set-up (address and code), where there is one. */
  onSetUp?: () => void;
  /** Linked (two-way): the Sender is the partner's laptop, which also views this one. */
  partner?: boolean;
  onSwitchRole?: () => void;
}

export function Viewer({ senderUrl, pairCode, partner = false, onSwitchRole, onSetUp }: Props) {
  const [store] = useState(() => new ViewerStore());
  const [settings, updateSettings] = useSettings();
  useStoreVersion(store);

  useEffect(() => {
    const conn = new ViewerConnection(store, {
      url: senderUrl,
      deviceId: deviceId(),
      name: 'Face2Face DJ Viewer',
      appVersion: APP_VERSION,
      os: osName(),
      pairCode,
    });
    conn.start();
    return () => conn.stop();
  }, [store, senderUrl, pairCode]);

  const bar = (
    <StatusBar
      store={store}
      settings={settings}
      remote={!isLocalUrl(senderUrl)}
      onChange={updateSettings}
      onSwitchRole={onSwitchRole}
      onSetUp={onSetUp}
    />
  );
  // The Sender turned us away: retrying will not help until the code is corrected.
  const refused = store.error?.code === 'pairing_failed';

  // The decks are on screen from the start, empty until the Sender fills them:
  // the window looks the same before and after linking, and the wait is a banner.
  const waiting = !store.everConnected;
  const stale = !waiting && store.connection !== 'connected';
  const decks = decksToShow(store, settings.layout);
  const who = partner ? 'your partner' : 'a Sender';
  const waitingFor = store.error !== null ? store.error.message : `Waiting for ${who} at ${senderUrl.replace(/^ws:\/\//, '')}…`;
  return (
    <div className={`viewer${stale ? ' viewer-stale' : ''}`}>
      {bar}
      {waiting && (
        <div className={`banner ${store.error !== null ? 'banner-warn' : 'banner-wait'}`}>
          {waitingFor}
          {onSetUp && <button className="banner-action" onClick={onSetUp}>{refused ? 'Correct the code' : 'Partner set-up'}</button>}
        </div>
      )}
      {stale && <div className="banner">{refused ? store.error!.message : 'Disconnected – showing last known. Reconnecting…'}</div>}
      {!waiting && !stale && store.error !== null && <div className="banner banner-warn">{store.error.message}</div>}
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
