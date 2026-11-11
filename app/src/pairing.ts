// Linking two laptops: this laptop's pairing code, and who the partner is.
// Both are kept in localStorage. See docs/SETUP.md for the user's side of it,
// and crates/face2face-core/src/net/pairing.rs for what the Sender checks.

import { DEFAULT_PORT } from './protocol/messages.ts';

/** Where the Sender to view is. */
export type Partner =
  /** On this computer: the mock sender in a browser, or a second window when trying two-way out. */
  | { kind: 'local' }
  /** Another laptop: its address (with a port only if not the default) and its pairing code. */
  | { kind: 'network'; address: string; code: string };

export const CODE_DIGITS = 6;
const CODE_KEY = 'face2face-dj.pairing.code';
const PARTNER_KEY = 'face2face-dj.pairing.partner';

/** The code as typed, without the spaces or dashes people add: 6 digits, or null. */
export function parseCode(text: string): string | null {
  const digits = text.replace(/[\s-]/g, '');
  return /^\d{6}$/.test(digits) ? digits : null;
}

/** "483920" as "483 920", the way it is shown and read out. */
export function formatCode(code: string): string {
  return `${code.slice(0, 3)} ${code.slice(3)}`;
}

/**
 * A partner's address as typed: an IPv4 address or a host name, optionally
 * with `:port`. Returns it tidied ("192.168.1.80", "dj-b.local:47811"), or null.
 */
export function parseAddress(text: string): string | null {
  const match = /^(?:ws:\/\/)?([A-Za-z0-9](?:[A-Za-z0-9.-]*[A-Za-z0-9])?)(?::(\d{1,5}))?\/?$/.exec(text.trim());
  if (match === null) return null;
  const host = match[1]!;
  // All digits and dots must be a real IPv4 address, not "192.168.1" or "300.1.1.1".
  if (/^[\d.]+$/.test(host)) {
    const parts = host.split('.');
    if (parts.length !== 4 || parts.some((part) => part === '' || part.length > 3 || Number(part) > 255)) return null;
  }
  if (match[2] === undefined) return host;
  const port = Number(match[2]);
  if (port < 1 || port > 65535) return null;
  return port === DEFAULT_PORT ? host : `${host}:${port}`;
}

/** The ws:// address of a partner's Sender. `localPort` is where one on this computer listens. */
export function partnerUrl(partner: Partner, localPort: number): string {
  if (partner.kind === 'local') return `ws://127.0.0.1:${localPort}`;
  return `ws://${partner.address.includes(':') ? partner.address : `${partner.address}:${DEFAULT_PORT}`}`;
}

/** Whether a ws:// address is on this computer. */
export function isLocalUrl(url: string): boolean {
  return /^ws:\/\/(127\.0\.0\.1|localhost|\[::1\])(:|\/|$)/.test(url);
}

function parsePartner(raw: unknown): Partner | null {
  if (typeof raw !== 'object' || raw === null) return null;
  const r = raw as Record<string, unknown>;
  if (r['kind'] === 'local') return { kind: 'local' };
  if (r['kind'] !== 'network' || typeof r['address'] !== 'string' || typeof r['code'] !== 'string') return null;
  const address = parseAddress(r['address']);
  const code = parseCode(r['code']);
  return address !== null && code !== null ? { kind: 'network', address, code } : null;
}

/** A uniformly random 6-digit code. */
function newCode(): string {
  const limit = 10 ** CODE_DIGITS;
  const values = new Uint32Array(1);
  // Reject the top of the range, which would make low codes slightly likelier.
  do crypto.getRandomValues(values);
  while (values[0]! >= Math.floor(2 ** 32 / limit) * limit);
  return String(values[0]! % limit).padStart(CODE_DIGITS, '0');
}

/**
 * This laptop's pairing code: made on first use, then always the same, so a
 * partner who has typed it once stays linked across restarts.
 */
export function myCode(): string {
  try {
    const saved = parseCode(localStorage.getItem(CODE_KEY) ?? '');
    if (saved !== null) return saved;
    const code = newCode();
    localStorage.setItem(CODE_KEY, code);
    return code;
  } catch {
    return newCode(); // no storage: a code for this run only
  }
}

export function loadPartner(): Partner | null {
  try {
    return parsePartner(JSON.parse(localStorage.getItem(PARTNER_KEY) ?? 'null'));
  } catch {
    return null;
  }
}

export function savePartner(partner: Partner | null): void {
  try {
    if (partner === null) localStorage.removeItem(PARTNER_KEY);
    else localStorage.setItem(PARTNER_KEY, JSON.stringify(partner));
  } catch {
    // no storage: the partner is asked for again next time
  }
}
