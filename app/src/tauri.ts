// Bridge to the Rust side. The UI also runs in a plain browser (npm run dev),
// where there is no Tauri. There the same four commands go to the dev server's
// development bridge (tools/mock-sender/dev-bridge.ts), which answers them with
// mock senders, so the whole app can be tried without the desktop build.
// Uses the global injected by `app.withGlobalTauri` (src-tauri/tauri.conf.json)
// instead of the @tauri-apps/api package, so the web build has no Tauri dependency.

interface TauriCore {
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
}

declare global {
  interface Window {
    __TAURI__?: { core: TauriCore };
  }
}

export function isTauri(): boolean {
  return typeof window !== 'undefined' && window.__TAURI__ !== undefined;
}

/** Thrown in a browser whose page does not come from the dev server: nothing there can share decks. */
export class NoBridgeError extends Error {
  constructor() {
    super('Sharing decks needs the desktop app, or the dev server (npm run dev).');
  }
}

const DEV_BRIDGE = '/dev-bridge';

/**
 * `session` names the "laptop" a command is for. The desktop app is one laptop
 * and ignores it; the development bridge keeps one Sender per session.
 */
async function invoke<T>(cmd: string, session: string, args: Record<string, unknown> = {}): Promise<T> {
  const tauri = window.__TAURI__;
  if (tauri !== undefined) return tauri.core.invoke<T>(cmd, args);
  let response: Response;
  try {
    response = await fetch(DEV_BRIDGE, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ cmd, session, args }),
    });
  } catch {
    throw new NoBridgeError();
  }
  // Anything but the bridge answers with a page or an error page, not JSON.
  if (!(response.headers.get('content-type') ?? '').includes('application/json')) throw new NoBridgeError();
  const body = (await response.json()) as { result?: T; error?: string };
  if (!response.ok) throw new Error(body.error ?? `development bridge: HTTP ${response.status}`);
  return body.result as T;
}

/** Mirrors `SenderInfo` in crates/face2face-core/src/sender.rs. */
export interface SenderInfo {
  port: number;
  decks: number;
  viewers: number;
  demo: boolean;
  /** Linked (two-way) only: the port of the partner's Sender. */
  partner_port: number | null;
}

/** `link`: two-way. Takes the free one of the two link ports and reports the partner's. */
export const startSender = (session: string, link: boolean): Promise<SenderInfo> => invoke<SenderInfo>('start_sender', session, { link });
export const stopSender = (session: string): Promise<void> => invoke<void>('stop_sender', session);
export const senderStatus = (session: string): Promise<SenderInfo | null> => invoke<SenderInfo | null>('sender_status', session);
/** Runs one mock-deck command, e.g. "play 1". Same vocabulary as tools/mock-sender. */
export const mockCommand = (session: string, command: string): Promise<void> => invoke<void>('mock_command', session, { command });

/** Stops a browser laptop's Sender as its page goes away, when a normal request would be dropped. */
export function stopSenderOnExit(session: string): void {
  if (isTauri()) return;
  const body = new Blob([JSON.stringify({ cmd: 'stop_sender', session, args: {} })], { type: 'application/json' });
  navigator.sendBeacon(DEV_BRIDGE, body);
}

/**
 * WebSocket address of a Sender on this computer. The desktop app connects
 * straight to it. A browser goes through the dev server's relay, because the
 * dev server's own address is the only one a forwarded or previewed port exposes.
 */
export function localSenderUrl(port: number): string {
  if (isTauri()) return `ws://127.0.0.1:${port}`;
  return `${window.location.protocol === 'https:' ? 'wss' : 'ws'}://${window.location.host}/dev-sender/${port}`;
}
