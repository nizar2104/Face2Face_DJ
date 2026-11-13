import { useEffect, useState } from 'react';
import { formatCode, loadPartner, myCode, partnerUrl, savePartner } from '../pairing.ts';
import type { Partner } from '../pairing.ts';
import { isTauri, mockCommand, networkAddresses, senderStatus, SOURCES, startSender, stopSender } from '../tauri.ts';
import type { LocalAddress, SenderInfo, SourceKind } from '../tauri.ts';
import { PartnerSetup } from './PartnerSetup.tsx';
import { Viewer } from './Viewer.tsx';

const PITCH_STEP = 0.5;

interface Props {
  /**
   * Two-way ("Link up"): share my decks and show the partner's.
   * Otherwise "Share only": share my decks and preview what the partner sees.
   */
  link: boolean;
  /** The source to start with. Not remembered between launches, like the role (docs/DECISIONS.md, D8). */
  initialSource: SourceKind;
  /** Linked only: a partner given in the URL, used instead of the remembered one (development). */
  initialPartner?: Partner | null;
  onSwitchRole: () => void;
}

/**
 * The Sender: starts the Rust core reading the chosen source (mock decks, or
 * the DJ software on this laptop). For mock decks it offers buttons for the
 * mock commands. Below is a Viewer on the real WebSocket path: of the
 * partner's decks when linked, of my own when only sharing.
 *
 * Linked, the partner has to be named first: their laptop's address and
 * pairing code, entered once in the set-up panel and remembered.
 */
export function Sender({ link, initialSource, initialPartner, onSwitchRole }: Props) {
  const [source, setSource] = useState<SourceKind>(initialSource);
  const [code] = useState(myCode);
  const [partner, setPartner] = useState<Partner | null>(() => initialPartner ?? loadPartner());
  const [settingUp, setSettingUp] = useState(false);
  const [addresses, setAddresses] = useState<LocalAddress[] | null>(null);
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
    setInfo(null);
    setError(null);
    const started = (i: SenderInfo): void => {
      if (cancelled) return;
      setInfo(i);
      setError(null);
    };
    const start = (): void => {
      startSender(link, source, code).then(started, fail);
    };
    start();
    const poll = setInterval(() => {
      // No Sender (starting it failed, e.g. the port was busy): try again.
      senderStatus().then((i) => (i !== null ? started(i) : start()), fail);
    }, 1000);
    return () => {
      cancelled = true;
      clearInterval(poll);
    };
  }, [desktop, link, source, code]);

  // My addresses, for the partner to type. Looked up again while the set-up
  // panel is open, because joining a network is part of setting up.
  const showSetup = link && (partner === null || settingUp);
  useEffect(() => {
    if (!desktop) return;
    let cancelled = false;
    const look = (): void => {
      networkAddresses().then((found) => !cancelled && setAddresses(found), () => undefined);
    };
    look();
    const again = showSetup ? setInterval(look, 3000) : null;
    return () => {
      cancelled = true;
      if (again !== null) clearInterval(again);
    };
  }, [desktop, showSetup]);

  const choosePartner = (chosen: Partner): void => {
    savePartner(chosen);
    setPartner(chosen);
    setSettingUp(false);
  };

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
          <p className="notice-title">Sharing decks needs the desktop app</p>
          <p className="notice-detail">
            A browser can only view. Start a stand-in Sender with <code>npm run mock-sender</code> and choose View only.
          </p>
          <button className="text-button" onClick={onSwitchRole}>Back</button>
        </div>
      </div>
    );
  }

  const mock = source === 'mock';
  const deckCount = info?.decks ?? 2;
  const label = SOURCES.find((s) => s.kind === source)?.label ?? source;
  let status = 'Starting…';
  if (info !== null) {
    // A reader says in its own words what it is waiting for, or what went wrong.
    const mine = mock ? (link ? 'My decks' : 'Sharing mock decks') : `${label}: ${info.reader_status}`;
    // What a partner needs to type, where there is room for it: when only sharing.
    const reach = addresses !== null && addresses.length > 0 ? `${addresses[0]!.address} · code ${formatCode(code)}` : `port ${info.port}`;
    const others = link
      ? info.viewers > 0 ? 'partner connected' : 'no partner yet'
      : `${reach} · ${info.viewers} viewer${info.viewers === 1 ? '' : 's'}`;
    status = `${mine} · ${others}`;
  }
  // Linked, the Viewer below shows the partner's decks; otherwise it previews my own.
  // A partner on this computer has my code too: both windows share their storage.
  let view: { url: string; code: string } | null = null;
  if (info !== null && !link) view = { url: `ws://127.0.0.1:${info.port}`, code };
  else if (info !== null && partner !== null && info.partner_port !== null) {
    view = { url: partnerUrl(partner, info.partner_port), code: partner.kind === 'network' ? partner.code : code };
  }
  return (
    <div className="sender">
      <div className="sender-bar">
        <span className="sender-status" title={info !== null ? `${status} · port ${info.port}` : status}>
          {status}
        </span>
        <span className="controls">
          <select
            className="source-select"
            aria-label="Where my decks come from"
            title="Where my decks come from"
            value={source}
            onChange={(e) => setSource(e.target.value as SourceKind)}
          >
            {SOURCES.map((s) => <option key={s.kind} value={s.kind}>{s.label}</option>)}
          </select>
          {mock && (<>
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
          </>)}
        </span>
      </div>
      {error !== null && <div className="banner banner-warn">{error}</div>}
      {showSetup ? (
        <PartnerSetup
          mine={{ addresses, code }}
          current={partner}
          localLabel="My partner is on this computer"
          onChoose={choosePartner}
          onCancel={partner !== null ? () => setSettingUp(false) : undefined}
          onHome={leave}
        />
      ) : view !== null && (
        <Viewer
          senderUrl={view.url}
          pairCode={view.code}
          partner={link}
          onSwitchRole={leave}
          onSetUp={link ? () => setSettingUp(true) : undefined}
        />
      )}
    </div>
  );
}
