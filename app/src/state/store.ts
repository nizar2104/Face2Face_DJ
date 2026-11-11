// Viewer-side state: what the Sender last told us, plus the waveform cache and
// the clock/interpolation needed to draw between messages. Framework-free so it
// runs under Node in tests; React subscribes through subscribe()/getVersion().

import type {
  DeckState,
  ErrorMsg,
  Hello,
  Message,
  SourceStatus,
  TrackInfo,
  WaveformChunk,
  WaveformInfo,
} from '../protocol/messages.ts';
import { decodeWaveformChunk } from '../protocol/messages.ts';
import { ClockSync } from '../net/clock.ts';
import { DeckInterpolator } from '../net/interpolate.ts';

export type ConnectionStatus = 'connecting' | 'connected' | 'disconnected';

export interface WaveformData {
  id: string;
  overview: Uint8Array | null;
  overviewFilled: number;
  detail: Uint8Array | null;
  detailFilled: number;
}

/** In-memory waveform cache on the Viewer, least recently used entries dropped first. */
export class WaveformCache {
  private map = new Map<string, WaveformData>();
  private readonly max: number;

  constructor(max = 16) {
    this.max = max;
  }

  get(id: string): WaveformData | null {
    const w = this.map.get(id);
    if (w === undefined) return null;
    this.map.delete(id);
    this.map.set(id, w);
    return w;
  }

  /** Reads without touching the LRU order (used every frame by the renderer). */
  peek(id: string): WaveformData | null {
    return this.map.get(id) ?? null;
  }

  get size(): number {
    return this.map.size;
  }

  addChunk(c: WaveformChunk): WaveformData {
    let w = this.get(c.waveformId);
    if (w === null) {
      w = { id: c.waveformId, overview: null, overviewFilled: 0, detail: null, detailFilled: 0 };
      this.map.set(c.waveformId, w);
      while (this.map.size > this.max) {
        const oldest = this.map.keys().next().value;
        if (oldest === undefined) break;
        this.map.delete(oldest);
      }
    }
    if (c.kind === 'overview') {
      if (w.overview === null || w.overview.length !== c.total) {
        w.overview = new Uint8Array(c.total);
        w.overviewFilled = 0;
      }
      // Chunks arrive in order over TCP; a gap means a bug, so the chunk is dropped.
      if (c.offset <= w.overviewFilled) {
        w.overview.set(c.payload, c.offset);
        w.overviewFilled = Math.max(w.overviewFilled, c.offset + c.payload.length);
      }
    } else {
      if (w.detail === null || w.detail.length !== c.total) {
        w.detail = new Uint8Array(c.total);
        w.detailFilled = 0;
      }
      if (c.offset <= w.detailFilled) {
        w.detail.set(c.payload, c.offset);
        w.detailFilled = Math.max(w.detailFilled, c.offset + c.payload.length);
      }
    }
    return w;
  }
}

export interface DeckModel {
  deck: number;
  track: TrackInfo | null;
  waveform: WaveformInfo | null;
  /** 0..1 while the Sender is analysing. */
  waveformProgress: number;
  state: DeckState | null;
  interp: DeckInterpolator;
}

function emptyDeck(deck: number): DeckModel {
  return { deck, track: null, waveform: null, waveformProgress: 0, state: null, interp: new DeckInterpolator() };
}

/** Text shown in place of the waveform, or null when the waveform can be drawn. */
export function deckNotice(d: DeckModel, wf: WaveformData | null): string | null {
  if (d.track === null) return 'Deck empty';
  switch (d.track.resolve_status) {
    case 'not_local':
      return 'Local files only';
    case 'file_not_found':
      return 'File not found';
    case 'ambiguous':
      return 'Ambiguous track';
    case 'ok':
      break;
  }
  if (d.waveform === null || d.waveform.status === 'unavailable') return 'No waveform';
  if (d.waveform.status === 'analysing') {
    return `Analysing waveform… ${Math.round(Math.min(Math.max(d.waveformProgress, 0), 1) * 100)}%`;
  }
  if (wf === null || wf.detail === null || wf.detailFilled === 0) return 'Loading waveform…';
  return null;
}

export class ViewerStore {
  connection: ConnectionStatus = 'connecting';
  /** True once a Sender has answered at least once in this run. */
  everConnected = false;
  sender: Hello | null = null;
  source: SourceStatus | null = null;
  error: ErrorMsg | null = null;
  readonly decks = new Map<number, DeckModel>();
  readonly waveforms = new WaveformCache();
  readonly clock = new ClockSync();

  private readonly now: () => number;
  /** Viewer-clock time at which the link was lost, while disconnected. */
  private frozenAt: number | null = null;
  private version = 0;
  private listeners = new Set<() => void>();

  /** @param now viewer monotonic clock in seconds */
  constructor(now: () => number = () => performance.now() / 1000) {
    this.now = now;
  }

  subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    return () => {
      this.listeners.delete(fn);
    };
  };

  getVersion = (): number => this.version;

  notify(): void {
    this.version++;
    for (const fn of this.listeners) fn();
  }

  deck(n: number): DeckModel {
    let d = this.decks.get(n);
    if (d === undefined) {
      d = emptyDeck(n);
      this.decks.set(n, d);
    }
    return d;
  }

  /** Deck numbers to show: what the DJ software reports, 2 until we know. */
  deckNumbers(): number[] {
    const count = this.source?.decks ?? 2;
    return Array.from({ length: count }, (_, i) => i + 1);
  }

  masterDeck(): number | null {
    for (const d of this.decks.values()) {
      if (d.track !== null && d.state?.is_master === true) return d.deck;
    }
    return null;
  }

  /** The time positions are drawn for: now, or the moment the link was lost. */
  private drawTime(): number {
    return this.frozenAt ?? this.now();
  }

  /** Estimated track position of a deck in seconds, or null if unknown. */
  position(deck: number): number | null {
    const d = this.decks.get(deck);
    if (d === undefined || d.track === null) return null;
    return d.interp.position(this.drawTime());
  }

  setConnection(status: ConnectionStatus): void {
    if (status === this.connection) return;
    if (status === 'connected') {
      this.everConnected = true;
      this.frozenAt = null;
      this.error = null;
    } else if (this.connection === 'connected') {
      // "Disconnected - showing last known": hold every deck where it was.
      this.frozenAt = this.now();
    }
    this.connection = status;
    this.notify();
  }

  setError(error: ErrorMsg | null): void {
    this.error = error;
    this.notify();
  }

  handleMessage(msg: Message): void {
    switch (msg.type) {
      case 'hello':
        // A (re)connected Sender follows this with a full snapshot.
        this.sender = msg;
        this.source = null;
        this.decks.clear();
        this.notify();
        return;
      case 'source_status':
        this.source = msg;
        this.notify();
        return;
      case 'deck_loaded': {
        const d = this.deck(msg.deck);
        d.track = msg.track;
        d.waveform = msg.waveform;
        d.waveformProgress = 0;
        d.state = null;
        d.interp.reset();
        d.interp.setDuration(msg.track.duration_s);
        this.notify();
        return;
      }
      case 'deck_state': {
        const d = this.decks.get(msg.deck);
        if (d === undefined || d.track === null) return;
        const prev = d.state;
        const rate = d.track.bpm_original > 0 ? msg.bpm_current / d.track.bpm_original : 1;
        // The timestamp is moved onto the viewer clock here, once, with the best
        // offset known right now. When the clock estimate later improves, only
        // new states use it, and the interpolator blends the difference away
        // like any other jitter instead of the picture stepping.
        // Before the first pong there is no offset yet: assume no delay.
        const now = this.now();
        const ts = this.clock.synced ? this.clock.toViewer(msg.ts) : now;
        d.interp.update({ ts, pos_s: msg.pos_s, playing: msg.playing, rate }, now);
        d.state = msg;
        // deck_state arrives ~60x a second; only wake React when something it renders changed.
        if (
          prev === null ||
          prev.playing !== msg.playing ||
          prev.is_master !== msg.is_master ||
          prev.on_air !== msg.on_air ||
          prev.bpm_current.toFixed(2) !== msg.bpm_current.toFixed(2)
        ) {
          this.notify();
        }
        return;
      }
      case 'deck_unloaded': {
        const d = this.deck(msg.deck);
        d.track = null;
        d.waveform = null;
        d.state = null;
        d.interp.reset();
        this.notify();
        return;
      }
      case 'waveform_status': {
        let changed = false;
        for (const d of this.decks.values()) {
          if (d.waveform !== null && d.waveform.waveform_id === msg.waveform_id) {
            d.waveform = { ...d.waveform, status: msg.status };
            d.waveformProgress = msg.progress;
            changed = true;
          }
        }
        if (changed) this.notify();
        return;
      }
      case 'error':
        this.setError(msg);
        return;
      case 'ping':
      case 'pong':
      case 'heartbeat':
        return; // handled by the connection
    }
  }

  handleBinary(data: ArrayBuffer | Uint8Array): void {
    const chunk = decodeWaveformChunk(data);
    if (chunk === null) return;
    const before = this.waveforms.peek(chunk.waveformId);
    const hadDetail = before !== null && before.detailFilled > 0;
    const w = this.waveforms.addChunk(chunk);
    // The canvases read the cache every frame; React only needs to know when
    // the "Loading waveform..." notice can go away.
    if (!hadDetail && w.detailFilled > 0) this.notify();
  }
}
