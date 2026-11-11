import { useEffect, useState } from 'react';
import { isTauri, mockCommand, senderStatus, startSender, stopSender } from '../tauri.ts';
import type { SenderInfo } from '../tauri.ts';
import { Viewer } from './Viewer.tsx';

const PITCH_STEP = 0.5;

/**
 * Milestone 0 Sender: starts the Rust core with its mock decks, offers buttons
 * for the mock commands, and previews what the partner sees through the real
 * WebSocket path.
 */
export function Sender({ onSwitchRole }: { onSwitchRole: () => void }) {
  const [info, setInfo] = useState<SenderInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [deck, setDeck] = useState(1);
  const [pitch, setPitch] = useState<Record<number, number>>({});
  const desktop = isTauri();

  useEffect(() => {
    if (!desktop) return;
    let cancelled = false;
    const fail = (e: unknown): void => {
      if (!cancelled) setError(String(e));
    };
    startSender().then((i) => !cancelled && setInfo(i), fail);
    const poll = setInterval(() => {
      senderStatus().then((i) => !cancelled && i !== null && setInfo(i), fail);
    }, 1000);
    return () => {
      cancelled = true;
      clearInterval(poll);
    };
  }, [desktop]);

  const run = (command: string): void => {
    setError(null);
    mockCommand(command).catch((e: unknown) => setError(String(e)));
  };
  const nudge = (delta: number): void => {
    const next = Math.round(((pitch[deck] ?? 0) + delta) * 100) / 100;
    setPitch({ ...pitch, [deck]: next });
    run(`pitch ${deck} ${next}`);
  };
  const leave = (): void => {
    if (desktop) stopSender().catch(() => undefined);
    onSwitchRole();
  };

  if (!desktop) {
    return (
      <div className="viewer">
        <div className="fullscreen-notice">
          <p className="notice-title">Share my decks needs the desktop app</p>
          <p className="notice-detail">
            In a browser, start the stand-in Sender with <code>npm run mock-sender</code> and choose View partner.
          </p>
          <button className="text-button" onClick={onSwitchRole}>Back</button>
        </div>
      </div>
    );
  }

  const deckCount = info?.decks ?? 2;
  return (
    <div className="sender">
      <div className="sender-bar">
        <span className="sender-status">
          {info !== null ? `Sharing mock decks · port ${info.port} · ${info.viewers} viewer${info.viewers === 1 ? '' : 's'}` : 'Starting…'}
        </span>
        <span className="controls">
          <span className="seg" role="group" aria-label="Deck">
            {Array.from({ length: deckCount }, (_, i) => i + 1).map((n) => (
              <button key={n} className={deck === n ? 'on' : ''} onClick={() => setDeck(n)}>D{n}</button>
            ))}
          </span>
          <span className="seg" role="group" aria-label="Mock deck controls">
            <button onClick={() => { setPitch({ ...pitch, [deck]: 0 }); run(`load ${deck}`); }}>Load</button>
            <button onClick={() => run(`play ${deck}`)}>Play</button>
            <button onClick={() => run(`pause ${deck}`)}>Pause</button>
            <button title={`Pitch −${PITCH_STEP} %`} onClick={() => nudge(-PITCH_STEP)}>−</button>
            <button title={`Pitch +${PITCH_STEP} %`} onClick={() => nudge(PITCH_STEP)}>+</button>
            <button title="Loop jump: back 4 bars" onClick={() => run(`jump ${deck} -16`)}>↺4</button>
            <button onClick={() => run(`eject ${deck}`)}>Eject</button>
          </span>
          <button
            className={`seg-single${info?.demo ? ' on' : ''}`}
            title="Scripted demo session"
            onClick={() => run(`demo ${info?.demo ? 'off' : 'on'}`)}
          >
            Demo
          </button>
        </span>
      </div>
      {error !== null && <div className="banner banner-warn">{error}</div>}
      {info !== null && <Viewer senderUrl={`ws://127.0.0.1:${info.port}`} onSwitchRole={leave} />}
    </div>
  );
}
