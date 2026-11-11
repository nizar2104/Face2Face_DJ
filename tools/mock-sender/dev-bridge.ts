// Development bridge: lets the UI run completely in a plain browser (npm run dev),
// where there is no Tauri and so no Rust Sender.
//
// It is a Vite dev-server plugin with two parts:
//   POST /dev-bridge          the four Tauri commands (app/src/tauri.ts), answered
//                             by mock senders from this folder, one per "laptop"
//   WebSocket /dev-sender/N   a relay to the Sender on 127.0.0.1:N, so the page
//                             reaches it through the dev server's own address
//                             (which is all a forwarded or previewed port exposes)
//
// A "laptop" is a session id the page makes up: a browser tab, or one half of
// the two-laptops-in-one-tab test page. Not part of the product; `vite build`
// never loads this file.

import { readFileSync } from 'node:fs';
import type { IncomingMessage, ServerResponse } from 'node:http';
import { fileURLToPath } from 'node:url';
import type { Plugin } from 'vite';
import { WebSocket, WebSocketServer } from 'ws';
import { DEFAULT_PORT } from '../../app/src/protocol/messages.ts';
import { parseSession } from './engine.ts';
import { MockSender } from './server.ts';

/** As `LINK_PORTS` in crates/face2face-core/src/protocol/mod.rs. */
const LINK_PORTS = [DEFAULT_PORT, DEFAULT_PORT + 1] as const;
const DECKS = 2;
/** A laptop whose page stopped asking for status is gone (tab closed without saying so). */
const FORGET_AFTER_MS = 90_000;
/** A page asks for status every second. One silent for this long gives up its port if another laptop needs it. */
const SILENT_AFTER_MS = 5000;
const SESSION_FILE = fileURLToPath(new URL('../../test-assets/sessions/demo.session', import.meta.url));

/** Mirrors `SenderInfo` in app/src/tauri.ts. */
interface SenderInfo {
  port: number;
  decks: number;
  viewers: number;
  demo: boolean;
  partner_port: number | null;
}

interface Laptop {
  sender: MockSender;
  port: number;
  partnerPort: number | null;
  lastSeen: number;
}

class Bridge {
  private readonly laptops = new Map<string, Laptop>();
  /** Commands run one at a time, so two laptops linking at once get one port each. */
  private queue: Promise<unknown> = Promise.resolve();

  run(cmd: string, session: string, args: Record<string, unknown>): Promise<unknown> {
    const result = this.queue.then(() => this.dispatch(cmd, session, args));
    this.queue = result.catch(() => undefined);
    return result;
  }

  private async dispatch(cmd: string, session: string, args: Record<string, unknown>): Promise<unknown> {
    const laptop = this.laptops.get(session);
    if (laptop !== undefined) laptop.lastSeen = Date.now();
    switch (cmd) {
      case 'start_sender':
        return info(laptop ?? (await this.start(session, args['link'] === true)));
      case 'stop_sender':
        if (laptop !== undefined) {
          this.laptops.delete(session);
          await laptop.sender.stop();
        }
        return null;
      case 'sender_status':
        return laptop !== undefined ? info(laptop) : null;
      case 'mock_command': {
        if (laptop === undefined) throw new Error('The Sender is not running.');
        const error = laptop.sender.command(String(args['command'] ?? ''));
        if (error !== null) throw new Error(error);
        return null;
      }
      default:
        throw new Error(`unknown command: ${cmd}`);
    }
  }

  /** As `MockSender::start` and `MockSender::start_link` in crates/face2face-core/src/sender.rs. */
  private async start(session: string, link: boolean): Promise<Laptop> {
    const ports: readonly number[] = link ? LINK_PORTS : [DEFAULT_PORT];
    const started = (await this.tryStart(session, link, ports)) ?? ((await this.evictSilent(ports)) ? await this.tryStart(session, link, ports) : null);
    if (started !== null) return started;
    throw new Error(
      link
        ? `ports ${LINK_PORTS[0]} and ${LINK_PORTS[1]} are both in use (are two laptops already linked?)`
        : `could not listen on 127.0.0.1:${DEFAULT_PORT} (is another Sender running?)`,
    );
  }

  /** Frees the port of the longest-silent laptop on one of `ports`, if any is silent. */
  private async evictSilent(ports: readonly number[]): Promise<boolean> {
    let silent: [string, Laptop] | null = null;
    for (const entry of this.laptops) {
      const laptop = entry[1];
      if (!ports.includes(laptop.port) || Date.now() - laptop.lastSeen < SILENT_AFTER_MS) continue;
      if (silent === null || laptop.lastSeen < silent[1].lastSeen) silent = entry;
    }
    if (silent === null) return false;
    this.laptops.delete(silent[0]);
    await silent[1].sender.stop();
    return true;
  }

  private async tryStart(session: string, link: boolean, ports: readonly number[]): Promise<Laptop | null> {
    for (const [side, port] of ports.entries()) {
      const sender = new MockSender({
        port,
        decks: DECKS,
        firstTrack: side * DECKS,
        name: link ? `Laptop ${side === 0 ? 'A' : 'B'} (mock decks)` : 'Face2Face DJ Sender (mock decks)',
        session: parseSession(readFileSync(SESSION_FILE, 'utf8')),
      });
      try {
        await sender.start();
      } catch (error) {
        await sender.stop();
        if ((error as NodeJS.ErrnoException).code === 'EADDRINUSE') continue;
        throw error;
      }
      const laptop: Laptop = { sender, port, partnerPort: link ? (ports[1 - side] ?? null) : null, lastSeen: Date.now() };
      this.laptops.set(session, laptop);
      return laptop;
    }
    return null;
  }

  forgetSilent(): void {
    for (const [session, laptop] of this.laptops) {
      if (Date.now() - laptop.lastSeen < FORGET_AFTER_MS) continue;
      this.laptops.delete(session);
      void laptop.sender.stop();
    }
  }

  stopAll(): void {
    for (const laptop of this.laptops.values()) void laptop.sender.stop();
    this.laptops.clear();
  }
}

function info(laptop: Laptop): SenderInfo {
  return {
    port: laptop.port,
    decks: laptop.sender.deckCount,
    viewers: laptop.sender.viewerCount,
    demo: laptop.sender.demoRunning,
    partner_port: laptop.partnerPort,
  };
}

function readBody(req: IncomingMessage): Promise<string> {
  return new Promise((resolve, reject) => {
    let text = '';
    req.setEncoding('utf8');
    req.on('data', (chunk: string) => {
      text += chunk;
      if (text.length > 4096) reject(new Error('request too large'));
    });
    req.on('end', () => resolve(text));
    req.on('error', reject);
  });
}

async function answer(bridge: Bridge, req: IncomingMessage, res: ServerResponse): Promise<void> {
  const reply = (status: number, body: unknown): void => {
    res.writeHead(status, { 'content-type': 'application/json', 'cache-control': 'no-store' });
    res.end(JSON.stringify(body));
  };
  try {
    const body: unknown = JSON.parse(await readBody(req));
    const b = typeof body === 'object' && body !== null ? (body as Record<string, unknown>) : {};
    const { cmd, session, args } = b;
    if (typeof cmd !== 'string' || typeof session !== 'string' || session === '') {
      reply(400, { error: 'expected {cmd, session, args}' });
      return;
    }
    const result = await bridge.run(cmd, session, typeof args === 'object' && args !== null ? (args as Record<string, unknown>) : {});
    reply(200, { result });
  } catch (error) {
    reply(400, { error: error instanceof Error ? error.message : String(error) });
  }
}

/** Joins a browser's WebSocket to the Sender on a loopback port, frame for frame. */
function relay(browser: WebSocket, port: number): void {
  const sender = new WebSocket(`ws://127.0.0.1:${port}`);
  const waiting: { data: WebSocket.RawData; binary: boolean }[] = [];
  browser.on('message', (data, binary) => {
    if (sender.readyState === WebSocket.OPEN) sender.send(data, { binary });
    else waiting.push({ data, binary });
  });
  sender.on('open', () => {
    for (const { data, binary } of waiting.splice(0)) sender.send(data, { binary });
  });
  sender.on('message', (data, binary) => {
    if (browser.readyState === WebSocket.OPEN) browser.send(data, { binary });
  });
  // No Sender on that port yet is normal (the partner has not linked): the
  // Viewer sees the close and tries again.
  sender.on('error', () => browser.close());
  sender.on('close', () => browser.close());
  browser.on('error', () => sender.terminate());
  browser.on('close', () => sender.terminate());
}

export function devBridge(): Plugin {
  return {
    name: 'face2face-dev-bridge',
    apply: 'serve',
    configureServer(server) {
      const bridge = new Bridge();
      const forget = setInterval(() => bridge.forgetSilent(), 5000);

      server.middlewares.use('/dev-bridge', (req, res, next) => {
        if (req.method !== 'POST') return next();
        void answer(bridge, req, res);
      });

      const sockets = new WebSocketServer({ noServer: true });
      server.httpServer?.on('upgrade', (req, socket, head) => {
        const match = /^\/dev-sender\/(\d+)$/.exec(req.url ?? '');
        if (match === null) return; // Vite's own hot-reload socket
        const port = Number(match[1]);
        // Only the Sender ports: this must not become a way to reach other local services.
        if (!LINK_PORTS.some((p) => p === port)) {
          socket.destroy();
          return;
        }
        sockets.handleUpgrade(req, socket, head, (browser) => relay(browser, port));
      });

      server.httpServer?.once('close', () => {
        clearInterval(forget);
        bridge.stopAll();
        sockets.close();
      });
    },
  };
}
