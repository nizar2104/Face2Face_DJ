// A stand-in Sender: simulated decks behind the real Face2Face DJ protocol
// (docs/PROTOCOL.md). The Viewer cannot tell it from the desktop app's Sender.

import { WebSocketServer } from 'ws';
import type { WebSocket } from 'ws';
import {
  DETAIL_FPS,
  PROTOCOL_VERSION,
  chunkWaveform,
  parseMessage,
} from '../../app/src/protocol/messages.ts';
import type {
  DeckLoaded,
  DeckState,
  DeckUnloaded,
  ErrorMsg,
  Heartbeat,
  Hello,
  Pong,
  SourceStatus,
} from '../../app/src/protocol/messages.ts';
import { MockEngine, SessionPlayer, parseCommand } from './engine.ts';
import type { MockCommand, MockEvent, MockTrack, SessionStep } from './engine.ts';
import { fnv1a32, hex8, synthWaveform } from './synth.ts';

export const TICK_HZ = 60;
const HELLO_TIMEOUT_MS = 5000;
/** A Viewer this far behind is dropped; it reconnects and gets a fresh snapshot. */
const MAX_BUFFERED_BYTES = 8 * 1024 * 1024;

export interface MockSenderOptions {
  port: number;
  /** Milestone 0 has no pairing yet, so the default is loopback only. */
  host?: string;
  decks?: number;
  name?: string;
  /** Session steps for the scripted demo; no demo if omitted. */
  session?: SessionStep[];
  demo?: boolean;
  log?: (line: string) => void;
}

interface LoadedDeck {
  loaded: DeckLoaded;
  chunks: Uint8Array[];
  lastState: DeckState | null;
}

export class MockSender {
  private readonly opts: MockSenderOptions;
  private readonly engine: MockEngine;
  private readonly player: SessionPlayer | null;
  private readonly decks = new Map<number, LoadedDeck>();
  private readonly ready = new Set<WebSocket>();
  private readonly deviceId = crypto.randomUUID();
  private wss: WebSocketServer | null = null;
  private tickTimer: ReturnType<typeof setInterval> | null = null;
  private heartbeatTimer: ReturnType<typeof setInterval> | null = null;
  private lastTick = 0;
  private demo: boolean;

  constructor(opts: MockSenderOptions) {
    this.opts = opts;
    this.engine = new MockEngine(opts.decks ?? 2);
    this.player = opts.session !== undefined ? new SessionPlayer(opts.session) : null;
    this.demo = (opts.demo ?? true) && this.player !== null;
  }

  /** Sender monotonic clock, seconds. */
  private now(): number {
    return performance.now() / 1000;
  }

  get viewerCount(): number {
    return this.ready.size;
  }

  /** Starts listening. Resolves with the bound port (useful with port 0). */
  start(): Promise<number> {
    return new Promise((resolve, reject) => {
      const wss = new WebSocketServer({ host: this.opts.host ?? '127.0.0.1', port: this.opts.port });
      this.wss = wss;
      wss.once('error', reject);
      wss.once('listening', () => {
        wss.off('error', reject);
        wss.on('error', (err) => this.log(`server error: ${err.message}`));
        this.lastTick = this.now();
        this.tickTimer = setInterval(() => this.tick(), 1000 / TICK_HZ);
        this.heartbeatTimer = setInterval(() => {
          const hb: Heartbeat = { type: 'heartbeat', ts: this.now() };
          this.broadcastJson(hb);
        }, 1000);
        const addr = wss.address();
        resolve(typeof addr === 'object' && addr !== null ? addr.port : this.opts.port);
      });
      wss.on('connection', (ws, req) => {
        req.socket.setNoDelay(true);
        this.accept(ws);
      });
    });
  }

  stop(): Promise<void> {
    if (this.tickTimer !== null) clearInterval(this.tickTimer);
    if (this.heartbeatTimer !== null) clearInterval(this.heartbeatTimer);
    this.tickTimer = null;
    this.heartbeatTimer = null;
    const wss = this.wss;
    this.wss = null;
    if (wss === null) return Promise.resolve();
    for (const ws of wss.clients) ws.terminate();
    this.ready.clear();
    return new Promise((resolve) => wss.close(() => resolve()));
  }

  /** Runs one command line. Returns an error message, or null on success. */
  command(line: string): string | null {
    const cmd = parseCommand(line);
    if (typeof cmd === 'string') return cmd;
    return this.run(cmd);
  }

  private run(cmd: MockCommand): string | null {
    if (cmd.kind === 'demo') {
      if (this.player === null) return 'no session loaded';
      this.demo = cmd.on;
      if (cmd.on) this.player.reset();
      return null;
    }
    const result = this.engine.apply(cmd);
    this.emit(result.events);
    return result.error;
  }

  private tick(): void {
    const now = this.now();
    const dt = now - this.lastTick;
    this.lastTick = now;
    if (this.demo && this.player !== null) {
      for (const cmd of this.player.advance(dt)) {
        const error = this.run(cmd);
        if (error !== null) this.log(`demo: ${error}`);
      }
    }
    this.emit(this.engine.tick(dt));
  }

  private emit(events: MockEvent[]): void {
    for (const ev of events) {
      switch (ev.kind) {
        case 'loaded': {
          const deck = this.buildLoaded(ev.deck, ev.track);
          this.decks.set(ev.deck, deck);
          this.broadcastJson(deck.loaded);
          for (const chunk of deck.chunks) this.broadcastBinary(chunk);
          break;
        }
        case 'state': {
          const deck = this.decks.get(ev.deck);
          if (deck === undefined) break;
          const state: DeckState = {
            type: 'deck_state',
            deck: ev.deck,
            ts: this.now(),
            pos_s: ev.pos_s,
            bpm_current: ev.bpm_current,
            playing: ev.playing,
            is_master: ev.is_master,
            on_air: ev.on_air,
          };
          deck.lastState = state;
          this.broadcastJson(state);
          break;
        }
        case 'unloaded': {
          this.decks.delete(ev.deck);
          const msg: DeckUnloaded = { type: 'deck_unloaded', deck: ev.deck };
          this.broadcastJson(msg);
          break;
        }
      }
    }
  }

  private buildLoaded(deck: number, track: MockTrack): LoadedDeck {
    const hash = fnv1a32(`${track.artist}\u0000${track.title}`);
    const waveformId = `mockwf-${hex8(hash)}`;
    const wf = synthWaveform(track.duration_s, track.bpm, hash);
    const loaded: DeckLoaded = {
      type: 'deck_loaded',
      deck,
      track: {
        track_id: `mock-${hex8(hash)}`,
        title: track.title,
        artist: track.artist,
        album: track.album,
        duration_s: track.duration_s,
        bpm_original: track.bpm,
        key: { camelot: track.camelot, musical: track.musical },
        file_name: track.file_name,
        local: true,
        resolve_status: 'ok',
      },
      waveform: { waveform_id: waveformId, detail_fps: DETAIL_FPS, frames: wf.frames, status: 'ready' },
    };
    // Overview first, so the Viewer can draw the whole track immediately.
    const chunks = [...chunkWaveform(waveformId, 'overview', wf.overview), ...chunkWaveform(waveformId, 'detail', wf.detail)];
    return { loaded, chunks, lastState: null };
  }

  private accept(ws: WebSocket): void {
    const helloTimer = setTimeout(() => ws.close(1008, 'hello timeout'), HELLO_TIMEOUT_MS);
    ws.on('error', () => ws.terminate());
    ws.on('close', () => {
      clearTimeout(helloTimer);
      if (this.ready.delete(ws)) this.log(`viewer disconnected (${this.ready.size} connected)`);
    });
    ws.on('message', (data, isBinary) => {
      if (isBinary) return;
      const msg = parseMessage(data.toString());
      if (msg === null) return;
      if (msg.type === 'ping') {
        const pong: Pong = { type: 'pong', t_viewer: msg.t_viewer, t_sender: this.now() };
        ws.send(JSON.stringify(pong));
        return;
      }
      if (msg.type !== 'hello' || msg.role !== 'viewer' || this.ready.has(ws)) return;
      clearTimeout(helloTimer);
      if (msg.protocol !== PROTOCOL_VERSION) {
        const err: ErrorMsg = {
          type: 'error',
          code: 'protocol_mismatch',
          message: `Sender speaks protocol ${PROTOCOL_VERSION}, Viewer sent ${msg.protocol}.`,
        };
        ws.send(JSON.stringify(err));
        ws.close(1002, 'protocol mismatch');
        return;
      }
      this.sendSnapshot(ws);
      this.ready.add(ws);
      this.log(`viewer connected: ${msg.name} (${this.ready.size} connected)`);
    });
  }

  private sendSnapshot(ws: WebSocket): void {
    const hello: Hello = {
      type: 'hello',
      protocol: PROTOCOL_VERSION,
      device_id: this.deviceId,
      name: this.opts.name ?? 'Mock Sender',
      role: 'sender',
      app_version: '0.1.0',
      os: process.platform === 'darwin' ? 'macos' : process.platform === 'win32' ? 'windows' : process.platform,
    };
    const status: SourceStatus = {
      type: 'source_status',
      software: 'mock',
      software_version: '0.1.0',
      connected: true,
      decks: this.engine.deckCount,
      reader: 'mock',
      reader_status: 'running',
    };
    ws.send(JSON.stringify(hello));
    ws.send(JSON.stringify(status));
    for (const deck of this.decks.values()) {
      ws.send(JSON.stringify(deck.loaded));
      for (const chunk of deck.chunks) ws.send(chunk, { binary: true });
      if (deck.lastState !== null) ws.send(JSON.stringify(deck.lastState));
    }
  }

  private broadcastJson(msg: object): void {
    if (this.ready.size === 0) return;
    const text = JSON.stringify(msg);
    for (const ws of this.ready) this.sendTo(ws, text, false);
  }

  private broadcastBinary(data: Uint8Array): void {
    for (const ws of this.ready) this.sendTo(ws, data, true);
  }

  private sendTo(ws: WebSocket, data: string | Uint8Array, binary: boolean): void {
    if (ws.readyState !== ws.OPEN) return;
    if (ws.bufferedAmount > MAX_BUFFERED_BYTES) {
      ws.terminate();
      return;
    }
    ws.send(data, { binary });
  }

  private log(line: string): void {
    this.opts.log?.(line);
  }
}
