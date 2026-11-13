// Runs the Mixxx mapping script against a stand-in for Mixxx's `engine` and
// `midi` objects, and checks its output against the fixture that the Rust
// decoder is tested with (crates/face2face-core/src/sources/mixxx.rs).
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

type Controls = Record<string, number>;
interface Mapping {
  init(): void;
  shutdown(): void;
  send(): void;
  split(value: unknown, bytes: number): number[];
}

const dir = import.meta.dirname;
const script = readFileSync(`${dir}/Face2Face-DJ.js`, 'utf8');
const fixture = readFileSync(`${dir}/../../test-assets/mixxx/deck_state.hex`, 'utf8')
  .split('\n')
  .filter((line) => line.trim() !== '' && !line.startsWith('#'))
  .map((line) => line.trim().split(/\s+/).map((byte) => parseInt(byte, 16)));

/** Deck 2 as the fixture describes it; every other control reads 0, like an empty deck. */
const DECK_2: Controls = {
  '[App].num_decks': 4,
  '[Master].crossfader': 0.25,
  '[Channel2].track_loaded': 1,
  '[Channel2].duration': 372.4,
  '[Channel2].playposition': 0.25,
  '[Channel2].play': 1,
  '[Channel2].sync_leader': 2,
  '[Channel2].volume': 0.8,
  '[Channel2].orientation': 2,
  '[Channel2].bpm': 126.5,
  '[Channel2].file_bpm': 124,
  '[Channel2].file_key': 22,
};

function run(controls: Controls) {
  const sent: number[][] = [];
  const listeners = new Map<string, () => void>();
  let ticks: (() => void) | null = null;
  let disconnected = 0;
  const engine = {
    getValue: (group: string, name: string) => controls[`${group}.${name}`] ?? 0,
    makeConnection: (group: string, name: string, callback: () => void) => {
      listeners.set(`${group}.${name}`, callback);
      return { disconnect: () => { disconnected += 1; } };
    },
    beginTimer: (_ms: number, callback: () => void) => { ticks = callback; return 1; },
    stopTimer: () => { ticks = null; },
  };
  const midi = { sendSysexMsg: (bytes: number[]) => { sent.push([...bytes]); } };
  const mapping = new Function('engine', 'midi', `${script}\nreturn Face2Face;`)(engine, midi) as Mapping;
  mapping.init();
  return {
    mapping,
    sent,
    tick: () => ticks?.(),
    change: (control: string) => listeners.get(control)?.(),
    running: () => ticks !== null,
    disconnected: () => disconnected,
  };
}

test('each tick sends every deck, and deck 2 matches the shared fixture', () => {
  const mixxx = run(DECK_2);
  mixxx.change('[Channel2].track_loaded');
  mixxx.change('[Channel2].duration');
  mixxx.change('[Channel2].duration');
  mixxx.tick();
  assert.equal(mixxx.sent.length, 4);
  assert.deepEqual(mixxx.sent[1], fixture[0]);
  assert.deepEqual(mixxx.sent[0], fixture[1], 'deck 1 is empty');
});

test('every message is valid MIDI: 25 bytes, data bytes below 128', () => {
  const extreme: Controls = { ...DECK_2, '[Channel2].playposition': -0.2, '[Channel2].bpm': 1e9, '[Channel2].duration': 1e12, '[App].num_decks': 64 };
  for (const controls of [DECK_2, extreme, {}]) {
    const mixxx = run(controls);
    mixxx.tick();
    for (const message of mixxx.sent) {
      assert.equal(message.length, 25);
      assert.equal(message[0], 0xf0);
      assert.equal(message[24], 0xf7);
      assert.ok(message.slice(1, 24).every((byte) => Number.isInteger(byte) && byte >= 0 && byte < 128), message.join(' '));
    }
  }
});

test('the crossfader on the other side takes a deck off air', () => {
  const flags = (controls: Controls): number => {
    const mixxx = run(controls);
    mixxx.tick();
    return mixxx.sent[1]![8]!;
  };
  assert.equal(flags(DECK_2) & 8, 8);
  assert.equal(flags({ ...DECK_2, '[Master].crossfader': -1 }) & 8, 0, 'deck on the right, crossfader fully left');
  assert.equal(flags({ ...DECK_2, '[Channel2].volume': 0 }) & 8, 0);
  assert.equal(flags({ ...DECK_2, '[Channel2].orientation': 1, '[Master].crossfader': -1 }) & 8, 8, 'centre ignores the crossfader');
});

test('numbers are split into 7-bit groups and clamped', () => {
  const { mapping } = run({});
  assert.deepEqual(mapping.split(93100, 4), [0, 5, 0x57, 0x2c]);
  assert.deepEqual(mapping.split(-5, 2), [0, 0]);
  assert.deepEqual(mapping.split(1e9, 2), [127, 127]);
  assert.deepEqual(mapping.split(Number.NaN, 1), [0]);
  assert.deepEqual(mapping.split(undefined, 1), [0]);
});

test('the load counter wraps at 128, and shutdown stops everything', () => {
  const mixxx = run(DECK_2);
  for (let i = 0; i < 130; i++) mixxx.change('[Channel2].track_loaded');
  mixxx.tick();
  assert.equal(mixxx.sent[1]![7], 2);
  mixxx.mapping.shutdown();
  assert.equal(mixxx.running(), false);
  assert.equal(mixxx.disconnected(), 8, 'two connections for each of four decks');
});
