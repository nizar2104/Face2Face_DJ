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

/** Mirrors `SenderInfo` in crates/face2face-core/src/sender.rs. */
export interface SenderInfo {
  port: number;
  decks: number;
  viewers: number;
  demo: boolean;
}

export const startSender = (): Promise<SenderInfo> => invoke<SenderInfo>('start_sender');
export const stopSender = (): Promise<void> => invoke<void>('stop_sender');
export const senderStatus = (): Promise<SenderInfo | null> => invoke<SenderInfo | null>('sender_status');
/** Runs one mock-deck command, e.g. "play 1". Same vocabulary as tools/mock-sender. */
export const mockCommand = (command: string): Promise<void> => invoke<void>('mock_command', { command });
