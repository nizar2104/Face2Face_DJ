// Viewer side of the transport: one WebSocket to the Sender, with hello,
// clock-sync pings, stale detection and reconnect with backoff (brief 7.6).
// Uses only the standard WebSocket API, so it runs in the webview, in a plain
// browser and under Node (tests).

import { PROTOCOL_VERSION, parseMessage } from '../protocol/messages.ts';
import type { Hello, Ping } from '../protocol/messages.ts';
import type { ViewerStore } from '../state/store.ts';

export const RECONNECT_MIN_S = 0.5;
export const RECONNECT_MAX_S = 5;
/** The Sender heartbeats at 1 Hz; this much silence means the link is gone. */
export const STALE_AFTER_S = 3;
export const PING_INTERVAL_S = 2;
/** Quick pings right after connecting, so the clock settles fast. */
export const PING_BURST = 5;
export const PING_BURST_INTERVAL_S = 0.15;

export interface ConnectionOptions {
  url: string;
  deviceId: string;
  name: string;
  appVersion: string;
  os: string;
  /** The Sender's pairing code, if it needs one (a Sender on another laptop does). */
  pairCode?: string;
  /** Viewer monotonic clock in seconds. Must be the same clock the store uses. */
  now?: () => number;
}

type Timer = ReturnType<typeof setTimeout>;

export class ViewerConnection {
  private readonly store: ViewerStore;
  private readonly opts: ConnectionOptions;
  private readonly now: () => number;
  private ws: WebSocket | null = null;
  private stopped = true;
  private backoff = RECONNECT_MIN_S;
  private lastRx = 0;
  private pingsSent = 0;
  private reconnectTimer: Timer | null = null;
  private pingTimer: Timer | null = null;
  private staleTimer: Timer | null = null;

  constructor(store: ViewerStore, opts: ConnectionOptions) {
    this.store = store;
    this.opts = opts;
    this.now = opts.now ?? (() => performance.now() / 1000);
  }

  start(): void {
    if (!this.stopped) return;
    this.stopped = false;
    this.open();
  }

  stop(): void {
    this.stopped = true;
    if (this.reconnectTimer !== null) clearTimeout(this.reconnectTimer);
    this.reconnectTimer = null;
    this.teardown();
  }

  private open(): void {
    if (this.stopped) return;
    let ws: WebSocket;
    try {
      ws = new WebSocket(this.opts.url);
    } catch {
      this.scheduleReconnect();
      return;
    }
    ws.binaryType = 'arraybuffer';
    this.ws = ws;

    ws.onopen = () => {
      if (this.ws !== ws) return;
      this.lastRx = this.now();
      const hello: Hello = {
        type: 'hello',
        protocol: PROTOCOL_VERSION,
        device_id: this.opts.deviceId,
        name: this.opts.name,
        role: 'viewer',
        app_version: this.opts.appVersion,
        os: this.opts.os,
        ...(this.opts.pairCode !== undefined ? { pair_code: this.opts.pairCode } : {}),
      };
      ws.send(JSON.stringify(hello));
      this.staleTimer = setInterval(() => this.checkStale(), 1000);
    };

    ws.onmessage = (ev: MessageEvent) => {
      if (this.ws !== ws) return;
      this.lastRx = this.now();
      if (typeof ev.data === 'string') {
        this.onText(ev.data);
      } else if (ev.data instanceof ArrayBuffer) {
        this.store.handleBinary(ev.data);
      }
    };

    ws.onclose = () => {
      if (this.ws === ws) this.dropped();
    };
    ws.onerror = () => {
      // onclose follows; nothing else to do.
    };
  }

  private onText(text: string): void {
    const msg = parseMessage(text);
    if (msg === null) return;
    switch (msg.type) {
      case 'hello':
        if (msg.role !== 'sender') return;
        if (msg.protocol !== PROTOCOL_VERSION) {
          this.store.setError({
            type: 'error',
            code: 'protocol_mismatch',
            message: `Sender speaks protocol ${msg.protocol}, this Viewer speaks ${PROTOCOL_VERSION}. Update Face2Face DJ on both computers.`,
          });
          this.dropped();
          return;
        }
        this.backoff = RECONNECT_MIN_S;
        this.store.clock.reset();
        this.store.handleMessage(msg);
        this.store.setConnection('connected');
        this.pingsSent = 0;
        this.ping();
        return;
      case 'pong':
        this.store.clock.addSample(msg.t_viewer, msg.t_sender, this.now());
        this.store.notify(); // refresh the latency readout
        return;
      case 'ping':
      case 'heartbeat':
        return;
      case 'error':
        this.store.handleMessage(msg);
        // The same code would be refused again, and each wrong attempt makes
        // the Sender slower to answer: stop until the user corrects it.
        if (msg.code === 'pairing_failed') {
          this.stop();
          this.store.setConnection(this.store.everConnected ? 'disconnected' : 'connecting');
        }
        return;
      default:
        this.store.handleMessage(msg);
    }
  }

  private ping(): void {
    const ws = this.ws;
    if (ws === null || ws.readyState !== WebSocket.OPEN) return;
    const ping: Ping = { type: 'ping', t_viewer: this.now() };
    ws.send(JSON.stringify(ping));
    this.pingsSent++;
    const delay = this.pingsSent < PING_BURST ? PING_BURST_INTERVAL_S : PING_INTERVAL_S;
    this.pingTimer = setTimeout(() => this.ping(), delay * 1000);
  }

  private checkStale(): void {
    if (this.ws !== null && this.now() - this.lastRx > STALE_AFTER_S) this.dropped();
  }

  /** The link is gone (closed, errored or silent): clean up and retry. */
  private dropped(): void {
    this.teardown();
    this.store.setConnection(this.store.everConnected ? 'disconnected' : 'connecting');
    this.scheduleReconnect();
  }

  private teardown(): void {
    if (this.pingTimer !== null) clearTimeout(this.pingTimer);
    if (this.staleTimer !== null) clearInterval(this.staleTimer);
    this.pingTimer = null;
    this.staleTimer = null;
    const ws = this.ws;
    this.ws = null;
    if (ws !== null) {
      ws.onopen = null;
      ws.onmessage = null;
      ws.onclose = null;
      ws.onerror = null;
      try {
        ws.close();
      } catch {
        // already closed
      }
    }
  }

  private scheduleReconnect(): void {
    if (this.stopped || this.reconnectTimer !== null) return;
    const delay = this.backoff;
    this.backoff = Math.min(this.backoff * 2, RECONNECT_MAX_S);
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      this.open();
    }, delay * 1000);
  }
}
