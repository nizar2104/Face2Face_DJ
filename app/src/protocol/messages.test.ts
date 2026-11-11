import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import {
  WAVEFORM_CHUNK_PAYLOAD,
  chunkWaveform,
  decodeWaveformChunk,
  encodeWaveformChunk,
  parseMessage,
} from './messages.ts';

const fixture = (name: string): string =>
  readFileSync(new URL(`../../../test-assets/protocol/${name}`, import.meta.url), 'utf8');

test('every fixture message parses, and all message types are covered', () => {
  const lines = fixture('messages.jsonl').split('\n').filter((l) => l.trim() !== '');
  const types = new Set<string>();
  for (const line of lines) {
    const msg = parseMessage(line);
    assert.notEqual(msg, null, `should parse: ${line}`);
    types.add(msg!.type);
  }
  assert.deepEqual(
    [...types].sort(),
    ['deck_loaded', 'deck_state', 'deck_unloaded', 'error', 'heartbeat', 'hello', 'ping', 'pong', 'source_status', 'waveform_status'],
  );
});

test('malformed and unknown messages are rejected without throwing', () => {
  const bad = [
    '',
    'not json',
    'null',
    '[]',
    '42',
    '{}',
    '{"type":"nope"}',
    '{"type":"pair_request","device_id":"x","nonce":"y"}', // reserved for Milestone 1
    '{"type":"deck_state","deck":1}',
    '{"type":"deck_state","deck":0,"ts":1,"pos_s":1,"bpm_current":120,"playing":true,"is_master":false,"on_air":false}',
    '{"type":"deck_state","deck":5,"ts":1,"pos_s":1,"bpm_current":120,"playing":true,"is_master":false,"on_air":false}',
    '{"type":"deck_state","deck":1.5,"ts":1,"pos_s":1,"bpm_current":120,"playing":true,"is_master":false,"on_air":false}',
    '{"type":"deck_state","deck":1,"ts":"1","pos_s":1,"bpm_current":120,"playing":true,"is_master":false,"on_air":false}',
    '{"type":"deck_state","deck":1,"ts":1,"pos_s":1,"bpm_current":120,"playing":"yes","is_master":false,"on_air":false}',
    '{"type":"ping"}',
    '{"type":"pong","t_viewer":1}',
    '{"type":"hello","protocol":1,"device_id":"a","name":"b","role":"dj","app_version":"1","os":"x"}',
    '{"type":"source_status","software":"x","software_version":"1","connected":true,"decks":9,"reader":"r","reader_status":"s"}',
    '{"type":"deck_loaded","deck":1,"track":null,"waveform":null}',
    '{"type":"deck_unloaded"}',
  ];
  for (const text of bad) assert.equal(parseMessage(text), null, text);
});

test('deck_loaded rejects a bad track or waveform block', () => {
  const lines = fixture('messages.jsonl').split('\n');
  const good = JSON.parse(lines.find((l) => l.includes('"deck_loaded"'))!);
  assert.notEqual(parseMessage(JSON.stringify(good)), null);
  for (const mutate of [
    (m: any) => (m.track.resolve_status = 'maybe'),
    (m: any) => (m.track.duration_s = -1),
    (m: any) => delete m.track.title,
    (m: any) => (m.track.key = { camelot: '8A' }),
    (m: any) => (m.waveform.status = 'done'),
    (m: any) => (m.waveform.frames = 1.5),
    (m: any) => (m.waveform.waveform_id = 'has space'),
    (m: any) => (m.waveform.waveform_id = 'x'.repeat(65)),
  ]) {
    const copy = structuredClone(good);
    mutate(copy);
    assert.equal(parseMessage(JSON.stringify(copy)), null);
  }
});

test('the binary fixture decodes and re-encodes byte for byte', () => {
  const bytes = Uint8Array.from(Buffer.from(fixture('waveform_chunk.hex').replace(/\s+/g, ''), 'hex'));
  const chunk = decodeWaveformChunk(bytes);
  assert.ok(chunk);
  assert.equal(chunk.waveformId, 'wf1');
  assert.equal(chunk.kind, 'detail');
  assert.equal(chunk.offset, 6);
  assert.equal(chunk.total, 12);
  assert.deepEqual([...chunk.payload], [1, 2, 3, 4, 5, 6]);
  assert.deepEqual([...encodeWaveformChunk(chunk)], [...bytes]);
});

test('decodeWaveformChunk accepts an ArrayBuffer and a view with an offset', () => {
  const frame = encodeWaveformChunk({ waveformId: 'abc', kind: 'overview', offset: 0, total: 3, payload: Uint8Array.of(9, 8, 7) });
  assert.deepEqual([...decodeWaveformChunk(frame.buffer as ArrayBuffer)!.payload], [9, 8, 7]);
  const padded = new Uint8Array(frame.length + 5);
  padded.set(frame, 5);
  const viewed = decodeWaveformChunk(padded.subarray(5));
  assert.equal(viewed?.kind, 'overview');
  assert.deepEqual([...viewed!.payload], [9, 8, 7]);
});

test('corrupt binary frames are rejected', () => {
  const good = encodeWaveformChunk({ waveformId: 'abc', kind: 'detail', offset: 0, total: 6, payload: Uint8Array.of(1, 2, 3) });
  assert.ok(decodeWaveformChunk(good));
  const corrupt = (edit: (b: Uint8Array) => Uint8Array): Uint8Array => edit(Uint8Array.from(good));
  const cases: Uint8Array[] = [
    new Uint8Array(0),
    good.subarray(0, 10),
    good.subarray(0, good.length - 1), // payload shorter than the header says
    corrupt((b) => ((b[0] = 0), b)), // magic
    corrupt((b) => ((b[4] = 2), b)), // version
    corrupt((b) => ((b[5] = 7), b)), // kind
    corrupt((b) => ((b[6] = 0), b)), // id length
    corrupt((b) => ((b[8] = 5), b)), // offset + length > total
    corrupt((b) => ((b[19] = 0xff), b)), // total over the limit
    corrupt((b) => ((b[20] = 0x20), b)), // id is not printable ASCII
  ];
  for (const c of cases) assert.equal(decodeWaveformChunk(c), null);
});

test('chunkWaveform splits in order and covers the data exactly', () => {
  const data = new Uint8Array(WAVEFORM_CHUNK_PAYLOAD * 2 + 100).map((_, i) => i % 251);
  const frames = chunkWaveform('id', 'detail', data);
  assert.equal(frames.length, 3);
  const rebuilt = new Uint8Array(data.length);
  let expectedOffset = 0;
  for (const f of frames) {
    const c = decodeWaveformChunk(f)!;
    assert.equal(c.offset, expectedOffset);
    assert.equal(c.total, data.length);
    rebuilt.set(c.payload, c.offset);
    expectedOffset += c.payload.length;
  }
  assert.deepEqual(rebuilt, data);
});
