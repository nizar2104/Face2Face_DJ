// Bridge to the Rust side. The UI also runs in a plain browser (npm run dev),
// where there is no Tauri: callers check isTauri() first.
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

function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const tauri = window.__TAURI__;
  if (tauri === undefined) return Promise.reject(new Error('Not running in the Face2Face DJ desktop app.'));
  return tauri.core.invoke<T>(cmd, args);
}

/** Where the Sender reads decks from. Mirrors `SourceKind` in crates/face2face-core/src/sources/mod.rs. */
export const SOURCES = [
  { kind: 'mock', label: 'Mock decks' },
  { kind: 'traktor', label: 'Traktor' },
  { kind: 'rekordbox', label: 'rekordbox' },
  { kind: 'pro_dj_link', label: 'Pro DJ Link' },
  { kind: 'mixxx', label: 'Mixxx' },
] as const;
export type SourceKind = (typeof SOURCES)[number]['kind'];

/** Mirrors `SenderInfo` in crates/face2face-core/src/sender.rs. */
export interface SenderInfo {
  port: number;
  source: SourceKind;
  /** From the source: its deck count, whether the DJ software is there, and the reader's state in words. */
  decks: number;
  connected: boolean;
  reader_status: string;
  viewers: number;
  /** Mock only. */
  demo: boolean;
  /** Linked (two-way) only: the port of a partner's Sender on this computer. */
  partner_port: number | null;
}

/** Mirrors `LocalAddress` in crates/face2face-core/src/net/addresses.rs. */
export interface LocalAddress {
  /** IPv4, e.g. "192.168.1.70". */
  address: string;
  /** The network it is on, e.g. "Wi-Fi" or "en0". */
  interface: string;
}

/**
 * Starts the Sender reading `source`; one already running with another source is replaced.
 * `link`: two-way. Takes the free one of the two link ports and reports the other,
 * where a partner on this computer would be.
 * `code`: this laptop's pairing code. Viewers must present it.
 */
export const startSender = (link: boolean, source: SourceKind, code: string): Promise<SenderInfo> =>
  invoke<SenderInfo>('start_sender', { link, source, code });
/** This laptop's addresses on its networks, the main one first. */
export const networkAddresses = (): Promise<LocalAddress[]> => invoke<LocalAddress[]>('network_addresses');
export const stopSender = (): Promise<void> => invoke<void>('stop_sender');
export const senderStatus = (): Promise<SenderInfo | null> => invoke<SenderInfo | null>('sender_status');
/** Runs one mock-deck command, e.g. "play 1". Same vocabulary as tools/mock-sender. */
export const mockCommand = (command: string): Promise<void> => invoke<void>('mock_command', { command });
