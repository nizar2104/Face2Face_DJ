// Face2Face DJ wire protocol v1. Source of truth: docs/PROTOCOL.md.
// The Rust mirror of this file is crates/face2face-core/src/protocol/.
// This module has no DOM or Node dependencies: the Viewer UI, the mock sender
// and the tests all import it.

export const PROTOCOL_VERSION = 1;
export const DEFAULT_PORT = 47801;
export const MAX_DECKS = 4;

export type Role = 'sender' | 'viewer';
export type ResolveStatus = 'ok' | 'ambiguous' | 'file_not_found' | 'not_local';
export type WaveformStatus = 'ready' | 'analysing' | 'unavailable';

export interface Hello {
  type: 'hello';
  protocol: number;
  device_id: string;
  name: string;
  role: Role;
  app_version: string;
  os: string;
  /** Viewer to Sender: the Sender's pairing code, when it has one. */
  pair_code?: string;
}

export interface SourceStatus {
  type: 'source_status';
  software: string;
  software_version: string;
  connected: boolean;
  decks: number;
  reader: string;
  reader_status: string;
}

export interface KeyInfo {
  camelot: string;
  musical: string;
}

export interface TrackInfo {
  track_id: string;
  title: string;
  artist: string;
  album: string;
  duration_s: number;
  bpm_original: number;
  key: KeyInfo | null;
  file_name: string;
  local: boolean;
  resolve_status: ResolveStatus;
}

export interface WaveformInfo {
  waveform_id: string;
  detail_fps: number;
  frames: number;
  status: WaveformStatus;
}

export interface DeckLoaded {
  type: 'deck_loaded';
  deck: number;
  track: TrackInfo;
  waveform: WaveformInfo | null;
}

export interface DeckState {
  type: 'deck_state';
  deck: number;
  /** Sender monotonic clock, seconds. */
  ts: number;
  pos_s: number;
  bpm_current: number;
  playing: boolean;
  is_master: boolean;
  on_air: boolean;
}

export interface DeckUnloaded {
  type: 'deck_unloaded';
  deck: number;
}

export interface WaveformStatusMsg {
  type: 'waveform_status';
  waveform_id: string;
  status: WaveformStatus;
  progress: number;
}

export interface Ping {
  type: 'ping';
  t_viewer: number;
}

export interface Pong {
  type: 'pong';
  t_viewer: number;
  t_sender: number;
}

export interface Heartbeat {
  type: 'heartbeat';
  ts: number;
}

export interface ErrorMsg {
  type: 'error';
  code: string;
  message: string;
}

export type Message =
  | Hello
  | SourceStatus
  | DeckLoaded
  | DeckState
  | DeckUnloaded
  | WaveformStatusMsg
  | Ping
  | Pong
  | Heartbeat
  | ErrorMsg;

// ---------------------------------------------------------------------------
// JSON parsing. Input comes from the network, so nothing here may throw:
// anything malformed or unknown yields null and is ignored by the caller.
// ---------------------------------------------------------------------------

type Obj = Record<string, unknown>;

const RESOLVE: readonly string[] = ['ok', 'ambiguous', 'file_not_found', 'not_local'];
const WF_STATUS: readonly string[] = ['ready', 'analysing', 'unavailable'];

function isObj(v: unknown): v is Obj {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}
function str(o: Obj, k: string): boolean {
  return typeof o[k] === 'string';
}
function num(o: Obj, k: string): boolean {
  const v = o[k];
  return typeof v === 'number' && Number.isFinite(v);
}
function bool(o: Obj, k: string): boolean {
  return typeof o[k] === 'boolean';
}
function deckNo(o: Obj): boolean {
  const v = o['deck'];
  return typeof v === 'number' && Number.isInteger(v) && v >= 1 && v <= MAX_DECKS;
}
function all(o: Obj, check: (o: Obj, k: string) => boolean, keys: string[]): boolean {
  return keys.every((k) => check(o, k));
}

function validKey(v: unknown): boolean {
  return v === null || (isObj(v) && all(v, str, ['camelot', 'musical']));
}

function validTrack(v: unknown): boolean {
  return (
    isObj(v) &&
    all(v, str, ['track_id', 'title', 'artist', 'album', 'file_name']) &&
    all(v, num, ['duration_s', 'bpm_original']) &&
    (v['duration_s'] as number) >= 0 &&
    (v['bpm_original'] as number) >= 0 &&
    bool(v, 'local') &&
    validKey(v['key']) &&
    RESOLVE.includes(v['resolve_status'] as string)
  );
}

function validWaveformId(v: unknown): boolean {
  return typeof v === 'string' && v.length >= 1 && v.length <= MAX_WAVEFORM_ID_LEN && /^[\x21-\x7e]+$/.test(v);
}

function validWaveformInfo(v: unknown): boolean {
  return (
    v === null ||
    (isObj(v) &&
      validWaveformId(v['waveform_id']) &&
      all(v, num, ['detail_fps', 'frames']) &&
      (v['detail_fps'] as number) > 0 &&
      Number.isInteger(v['frames']) &&
      (v['frames'] as number) >= 0 &&
      WF_STATUS.includes(v['status'] as string))
  );
}

function valid(o: Obj): boolean {
  switch (o['type']) {
    case 'hello':
      return (
        num(o, 'protocol') &&
        all(o, str, ['device_id', 'name', 'app_version', 'os']) &&
        (o['role'] === 'sender' || o['role'] === 'viewer')
      );
    case 'source_status':
      return (
        all(o, str, ['software', 'software_version', 'reader', 'reader_status']) &&
        bool(o, 'connected') &&
        Number.isInteger(o['decks']) &&
        (o['decks'] as number) >= 1 &&
        (o['decks'] as number) <= MAX_DECKS
      );
    case 'deck_loaded':
      return deckNo(o) && validTrack(o['track']) && validWaveformInfo(o['waveform']);
    case 'deck_state':
      return (
        deckNo(o) &&
        all(o, num, ['ts', 'pos_s', 'bpm_current']) &&
        all(o, bool, ['playing', 'is_master', 'on_air'])
      );
    case 'deck_unloaded':
      return deckNo(o);
    case 'waveform_status':
      return validWaveformId(o['waveform_id']) && WF_STATUS.includes(o['status'] as string) && num(o, 'progress');
    case 'ping':
      return num(o, 't_viewer');
    case 'pong':
      return all(o, num, ['t_viewer', 't_sender']);
    case 'heartbeat':
      return num(o, 'ts');
    case 'error':
      return all(o, str, ['code', 'message']);
    default:
      return false;
  }
}

/** Parses one JSON text frame. Returns null for malformed or unknown messages. */
export function parseMessage(text: string): Message | null {
  let v: unknown;
  try {
    v = JSON.parse(text);
  } catch {
    return null;
  }
  if (!isObj(v) || !valid(v)) return null;
  return v as unknown as Message;
}

// ---------------------------------------------------------------------------
// Binary frame: waveform_chunk (layout in docs/PROTOCOL.md, section 4).
// ---------------------------------------------------------------------------

export const WAVEFORM_MAGIC = 0x46574454; // "TDWF" read as a little-endian u32
export const WAVEFORM_CHUNK_VERSION = 1;
export const WAVEFORM_HEADER_LEN = 20;
export const MAX_WAVEFORM_ID_LEN = 64;
/** 16 MiB of detail data is about 10 hours of audio at 150 fps x 3 bytes. */
export const MAX_WAVEFORM_BYTES = 16 * 1024 * 1024;
export const WAVEFORM_CHUNK_PAYLOAD = 16 * 1024;
export const BYTES_PER_FRAME = 3;
export const DETAIL_FPS = 150;
export const OVERVIEW_POINTS = 1200;

export type WaveformKind = 'overview' | 'detail';

export interface WaveformChunk {
  waveformId: string;
  kind: WaveformKind;
  /** Byte offset of this payload within the kind's data. */
  offset: number;
  /** Total byte length of the kind's data. */
  total: number;
  payload: Uint8Array;
}

export function encodeWaveformChunk(c: WaveformChunk): Uint8Array {
  const id = c.waveformId;
  if (!validWaveformId(id)) throw new Error('invalid waveform id');
  const out = new Uint8Array(WAVEFORM_HEADER_LEN + id.length + c.payload.length);
  const dv = new DataView(out.buffer);
  dv.setUint32(0, WAVEFORM_MAGIC, true);
  dv.setUint8(4, WAVEFORM_CHUNK_VERSION);
  dv.setUint8(5, c.kind === 'overview' ? 0 : 1);
  dv.setUint8(6, id.length);
  dv.setUint8(7, 0);
  dv.setUint32(8, c.offset, true);
  dv.setUint32(12, c.payload.length, true);
  dv.setUint32(16, c.total, true);
  for (let i = 0; i < id.length; i++) out[WAVEFORM_HEADER_LEN + i] = id.charCodeAt(i);
  out.set(c.payload, WAVEFORM_HEADER_LEN + id.length);
  return out;
}

/** Decodes one binary frame. Returns null if it is not a valid waveform chunk. */
export function decodeWaveformChunk(data: ArrayBuffer | Uint8Array): WaveformChunk | null {
  const bytes = data instanceof Uint8Array ? data : new Uint8Array(data);
  if (bytes.length < WAVEFORM_HEADER_LEN) return null;
  const dv = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (dv.getUint32(0, true) !== WAVEFORM_MAGIC) return null;
  if (dv.getUint8(4) !== WAVEFORM_CHUNK_VERSION) return null;
  const kindByte = dv.getUint8(5);
  if (kindByte > 1) return null;
  const idLen = dv.getUint8(6);
  if (idLen < 1 || idLen > MAX_WAVEFORM_ID_LEN) return null;
  const offset = dv.getUint32(8, true);
  const length = dv.getUint32(12, true);
  const total = dv.getUint32(16, true);
  if (total > MAX_WAVEFORM_BYTES || offset + length > total) return null;
  if (bytes.length !== WAVEFORM_HEADER_LEN + idLen + length) return null;
  let id = '';
  for (let i = 0; i < idLen; i++) {
    const ch = bytes[WAVEFORM_HEADER_LEN + i]!;
    if (ch < 0x21 || ch > 0x7e) return null;
    id += String.fromCharCode(ch);
  }
  const start = WAVEFORM_HEADER_LEN + idLen;
  return {
    waveformId: id,
    kind: kindByte === 0 ? 'overview' : 'detail',
    offset,
    total,
    payload: bytes.subarray(start, start + length),
  };
}

/** Splits one kind's data into wire chunks, in order. */
export function chunkWaveform(waveformId: string, kind: WaveformKind, data: Uint8Array): Uint8Array[] {
  const out: Uint8Array[] = [];
  for (let offset = 0; offset < data.length; offset += WAVEFORM_CHUNK_PAYLOAD) {
    const payload = data.subarray(offset, Math.min(data.length, offset + WAVEFORM_CHUNK_PAYLOAD));
    out.push(encodeWaveformChunk({ waveformId, kind, offset, total: data.length, payload }));
  }
  return out;
}
