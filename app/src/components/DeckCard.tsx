import { useRef } from 'react';
import { useAnimationFrame } from '../hooks.ts';
import type { KeyNotation } from '../settings.ts';
import { deckNotice } from '../state/store.ts';
import type { ViewerStore } from '../state/store.ts';
import { formatRemaining } from '../waveform/render.ts';
import { KeyBadge } from './KeyBadge.tsx';
import { OverviewBar, WaveformView } from './WaveformView.tsx';

interface Props {
  store: ViewerStore;
  deck: number;
  notation: KeyNotation;
  zoomS: number;
}

export function DeckCard({ store, deck, notation, zoomS }: Props) {
  const d = store.deck(deck);
  const track = d.track;
  const state = d.state;
  const wf = d.waveform !== null ? store.waveforms.peek(d.waveform.waveform_id) : null;
  const notice = deckNotice(d, wf);

  // Remaining time changes once a second; write it straight to the DOM from
  // the frame loop instead of re-rendering the card 60 times a second.
  const remainRef = useRef<HTMLSpanElement>(null);
  useAnimationFrame(() => {
    const el = remainRef.current;
    if (el === null) return;
    const model = store.decks.get(deck);
    const pos = store.position(deck);
    // A length of 0 means the source does not know it (docs/PROTOCOL.md, 3.3).
    const known = model?.track != null && model.track.duration_s > 0 && pos !== null;
    const text = known ? formatRemaining(model.track!.duration_s, pos) : '--:--';
    if (el.textContent !== text) el.textContent = text;
  });

  if (track === null) {
    return (
      <section className="deck deck-empty" data-deck={deck}>
        <span className="deck-no">DECK {deck}</span>
        <span className="notice">Deck empty</span>
      </section>
    );
  }

  const playing = state?.playing === true;
  return (
    <section className={`deck${state?.is_master ? ' deck-master' : ''}`} data-deck={deck}>
      <div className="deck-head">
        <span className="deck-no">DECK {deck}</span>
        <span className={`play-state ${playing ? 'is-playing' : 'is-paused'}`} title={playing ? 'Playing' : 'Paused'}>
          {playing ? '▶' : '⏸'}
        </span>
        {state?.is_master && <span className="badge badge-master">MASTER</span>}
        <span className={`badge badge-air${state?.on_air ? ' on' : ''}`} title={state?.on_air ? 'On air' : 'Not on air'}>
          ON AIR
        </span>
        <span className="track" title={`${track.title} – ${track.artist}`}>
          <span className="track-title">{track.title}</span>
          <span className="track-artist"> – {track.artist}</span>
        </span>
      </div>
      <div className="deck-numbers">
        <span className="bpm">
          <span className="bpm-current">{(state?.bpm_current ?? track.bpm_original).toFixed(2)}</span>
          <span className="bpm-unit">BPM</span>
          <span className="bpm-original">orig {track.bpm_original.toFixed(2)}</span>
        </span>
        <KeyBadge keyInfo={track.key} notation={notation} />
        <span className="remain" ref={remainRef}>--:--</span>
      </div>
      <div className="wave">
        {notice === null ? <WaveformView store={store} deck={deck} zoomS={zoomS} /> : <span className="notice">{notice}</span>}
      </div>
      <div className="overview">
        <OverviewBar store={store} deck={deck} />
      </div>
    </section>
  );
}
