import { useEffect, useState } from 'react';
import { NoBridgeError, localSenderUrl, mockCommand, senderStatus, startSender, stopSender, stopSenderOnExit } from '../tauri.ts';
import type { SenderInfo } from '../tauri.ts';
import { Viewer } from './Viewer.tsx';

const PITCH_STEP = 0.5;

interface Props {
  /**
   * Two-way ("Link up"): share my decks and show the partner's.
   * Otherwise "Share only": share my decks and preview what the partner sees.
   */
  link: boolean;
  /** Which "laptop" this is, for the development bridge in a browser (see tauri.ts). */
  session: string;
  onSwitchRole: () => void;
}

/**
 * Milestone 0 Sender: starts a Sender with mock decks (the Rust core in the
 * desktop app, the development bridge in a browser) and offers buttons for
 * the mock commands. Below them is a Viewer on the real WebSocket
 * path: of the partner's decks when linked, of my own when only sharing.
 */
export function Sender({ link, session, onSwitchRole }: Props) {
  const [info, setInfo] = useState<SenderInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [deck, setDeck] = useState(1);
  const [pitch, setPitch] = useState<Record<number, number>>({});
  const [unavailable, setUnavailable] = useState(false);

  useEffect(() => {
    let cancelled = false;
    const fail = (e: unknown): void => {
      if (cancelled) return;
      if (e instanceof NoBridgeError) setUnavailable(true);
      else setError(e instanceof Error ? e.message : String(e));
    };
    const started = (i: SenderInfo): void => {
      if (cancelled) return;
      setInfo(i);
      setError(null);
    };
    startSender(session, link).then(started, fail);
    const poll = setInterval(() => {
      // No Sender any more (it was stopped from elsewhere): start it again.
      senderStatus(session).then((i) => (i !== null ? started(i) : startSender(session, link).then(started, fail)), fail);
    }, 1000);
    const onExit = (): void => stopSenderOnExit(session);
    window.addEventListener('pagehide', onExit);
    return () => {
      cancelled = true;
      clearInterval(poll);
      window.removeEventListener('pagehide', onExit);
    };
  }, [session, link]);

  const run = (command: string): void => {
    setError(null);
    mockCommand(session, command).catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  };
  const nudge = (delta: number): void => {
    const next = Math.round(((pitch[deck] ?? 0) + delta) * 100) / 100;
    setPitch({ ...pitch, [deck]: next });
    run(`pitch ${deck} ${next}`);
  };
  const leave = (): void => {
    stopSender(session).catch(() => undefined);
    onSwitchRole();
  };

  if (unavailable) {
    return (
      <div className="viewer">
        <div className="fullscreen-notice">
          <p className="notice-title">Sharing decks needs the desktop app</p>
          <p className="notice-detail">
            This page can only view. Start a stand-in Sender with <code>npm run mock-sender</code> and choose View only.
          </p>
          <button className="text-button" onClick={onSwitchRole}>Back</button>
        </div>
      </div>
    );
  }

  if (info === null && error !== null) {
    return (
      <div className="viewer">
        <div className="fullscreen-notice">
          <p className="notice-title">Could not start sharing</p>
          <p className="notice-detail">{error}</p>
          <button className="text-button" onClick={onSwitchRole}>Back</button>
        </div>
      </div>
    );
  }

  const deckCount = info?.decks ?? 2;
  let status = 'Starting…';
  if (info !== null && link) {
    // The first of a pair to link is Laptop A (the lower port); it is the name the partner sees.
    const side = info.partner_port !== null && info.port > info.partner_port ? 'B' : 'A';
    status = `Laptop ${side} · ${info.viewers > 0 ? 'partner connected' : 'no partner yet'}`;
  }
  else if (info !== null) status = `Sharing mock decks · port ${info.port} · ${info.viewers} viewer${info.viewers === 1 ? '' : 's'}`;
  // Linked, the Viewer below shows the partner's decks; otherwise it previews my own.
  const viewPort = info === null ? null : link ? info.partner_port : info.port;
  return (
    <div className="sender">
      <div className="sender-bar">
        <span className="sender-status" title={info !== null ? `${status} · sharing my mock decks on port ${info.port}` : status}>
          {status}
        </span>
        <span className="controls">
          <span className="seg" role="group" aria-label="Deck">
            {Array.from({ length: deckCount }, (_, i) => i + 1).map((n) => (
              <button key={n} className={deck === n ? 'on' : ''} onClick={() => setDeck(n)}>D{n}</button>
            ))}
          </span>
          <span className="seg" role="group" aria-label="Controls for my mock decks">
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
      {viewPort !== null && (
        <Viewer senderUrl={localSenderUrl(viewPort)} partner={link} onSwitchRole={leave} />
      )}
    </div>
  );
}
