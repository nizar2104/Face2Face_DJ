import assert from 'node:assert/strict';
import { test } from 'node:test';
import { encodeWaveformChunk } from '../protocol/messages.ts';
import type { DeckLoaded, DeckState, Hello, SourceStatus } from '../protocol/messages.ts';
import { ViewerStore, WaveformCache, deckNotice } from './store.ts';

const hello: Hello = { type: 'hello', protocol: 1, device_id: 'd', name: 'DJ A', role: 'sender', app_version: '0.1.0', os: 'macos' };
const status = (decks: number): SourceStatus => ({
  type: 'source_status', software: 'mock', software_version: '0.1.0', connected: true, decks, reader: 'mock', reader_status: 'running',
});
const loaded = (deck: number, over: Partial<DeckLoaded['track']> = {}): DeckLoaded => ({
  type: 'deck_loaded',
  deck,
  track: {
    track_id: 't', title: 'Title', artist: 'Artist', album: '', duration_s: 300, bpm_original: 125,
    key: { camelot: '8A', musical: 'Am' }, file_name: 'a.flac', local: true, resolve_status: 'ok', ...over,
  },
  waveform: { waveform_id: 'wf', detail_fps: 150, frames: 4, status: 'ready' },
});
const state = (deck: number, over: Partial<DeckState> = {}): DeckState => ({
  type: 'deck_state', deck, ts: 0, pos_s: 0, bpm_current: 125, playing: false, is_master: false, on_air: false, ...over,
});

function makeStore(): { store: ViewerStore; clock: { t: number }; notified: () => number } {
  const clock = { t: 0 };
  const store = new ViewerStore(() => clock.t);
  let n = 0;
  store.subscribe(() => n++);
  return { store, clock, notified: () => n };
}

test('shows 2 decks until the source says otherwise', () => {
  const { store } = makeStore();
  assert.deepEqual(store.deckNumbers(), [1, 2]);
  store.handleMessage(status(4));
  assert.deepEqual(store.deckNumbers(), [1, 2, 3, 4]);
});

test('load, play, pitch, pause, eject', () => {
  const { store, clock } = makeStore();
  store.handleMessage(loaded(1));
  assert.equal(store.position(1), null); // loaded, but no state yet
  store.handleMessage(state(1, { ts: 0, pos_s: 10, playing: true, bpm_current: 125 }));
  clock.t = 2;
  assert.equal(store.position(1), 11); // extrapolation stops after MAX_EXTRAPOLATION_S of silence
  store.handleMessage(state(1, { ts: 2, pos_s: 12, playing: true, bpm_current: 137.5 })); // +10 %
  clock.t = 2.5;
  assert.ok(Math.abs(store.position(1)! - 12.55) < 1e-9);
  store.handleMessage(state(1, { ts: 2.5, pos_s: 12.55, playing: false, bpm_current: 137.5 }));
  clock.t = 30;
  assert.ok(Math.abs(store.position(1)! - 12.55) < 1e-9);
  store.handleMessage({ type: 'deck_unloaded', deck: 1 });
  assert.equal(store.position(1), null);
  assert.equal(store.deck(1).track, null);
});

test('deck_state for an empty deck is ignored', () => {
  const { store } = makeStore();
  store.handleMessage(state(2, { playing: true }));
  assert.equal(store.position(2), null);
});

test('React is only woken when something it renders changed', () => {
  const { store, notified } = makeStore();
  store.handleMessage(loaded(1));
  store.handleMessage(state(1, { playing: true, pos_s: 1 }));
  const before = notified();
  for (let i = 0; i < 60; i++) store.handleMessage(state(1, { playing: true, ts: i / 60, pos_s: 1 + i / 60 }));
  assert.equal(notified(), before);
  store.handleMessage(state(1, { playing: true, bpm_current: 125.004 })); // same at 2 decimals
  assert.equal(notified(), before);
  store.handleMessage(state(1, { playing: true, bpm_current: 125.01 }));
  store.handleMessage(state(1, { playing: true, bpm_current: 125.01, is_master: true }));
  store.handleMessage(state(1, { playing: false, bpm_current: 125.01, is_master: true }));
  assert.equal(notified(), before + 3);
});

test('master deck lookup', () => {
  const { store } = makeStore();
  assert.equal(store.masterDeck(), null);
  store.handleMessage(loaded(1));
  store.handleMessage(loaded(2));
  store.handleMessage(state(1));
  store.handleMessage(state(2, { is_master: true }));
  assert.equal(store.masterDeck(), 2);
});

test('disconnect freezes the picture; a new hello clears the decks', () => {
  const { store, clock } = makeStore();
  store.handleMessage(hello);
  store.setConnection('connected');
  store.handleMessage(loaded(1));
  store.handleMessage(state(1, { ts: 0, pos_s: 5, playing: true }));
  clock.t = 0.5;
  store.setConnection('disconnected');
  clock.t = 60;
  assert.equal(store.position(1), 5.5); // showing last known
  assert.equal(store.everConnected, true);
  store.handleMessage(hello);
  store.setConnection('connected');
  assert.equal(store.decks.size, 0);
});

test('with a synced clock, the sender timestamp places the state in viewer time', () => {
  const { store, clock } = makeStore();
  store.clock.addSample(0, 1000.001, 0.002); // sender clock = viewer clock + 1000, 1 ms each way
  store.handleMessage(loaded(1));
  clock.t = 0.105;
  store.handleMessage(state(1, { ts: 1000.1, pos_s: 20, playing: true })); // sent at viewer time 0.100
  assert.ok(Math.abs(store.position(1)! - 20.005) < 1e-9);
  clock.t = 0.2;
  assert.ok(Math.abs(store.position(1)! - 20.1) < 1e-9);
});

test('a better clock estimate is blended in, the picture does not step', () => {
  const { store, clock } = makeStore();
  store.clock.addSample(0, 1000.03, 0.04); // first pong: 40 ms round trip, offset estimate 1000.01
  store.handleMessage(loaded(1));
  const truth = (viewerT: number): number => 10 + viewerT; // real offset is exactly 1000
  const send = (viewerT: number): void => {
    clock.t = viewerT;
    store.handleMessage(state(1, { ts: 1000 + viewerT, pos_s: truth(viewerT), playing: true }));
  };
  send(1.0);
  const before = store.position(1)!;
  assert.ok(Math.abs(before - truth(1.0) - 0.01) < 1e-9, 'starts 10 ms off, as the poor estimate implies');

  store.clock.addSample(1.0, 1001.0005, 1.001); // a 1 ms round trip arrives: offset 1000.0000
  assert.ok(Math.abs(store.position(1)! - before) < 1e-9, 'no step when the estimate changes');
  let last = before;
  for (let t = 1.0 + 1 / 60; t < 1.5; t += 1 / 60) {
    send(t);
    const p = store.position(1)!;
    assert.ok(p > last, 'keeps moving forward');
    assert.ok(Math.abs(p - last - 1 / 60) < 0.004, 'no visible step between frames');
    last = p;
  }
  assert.ok(Math.abs(last - truth(clock.t)) < 0.0005, 'converged on the truth');
});

test('before the first connection the status stays "connecting"', () => {
  const { store } = makeStore();
  assert.equal(store.connection, 'connecting');
  assert.equal(store.everConnected, false);
});

test('waveform chunks fill the cache in order, and gaps are dropped', () => {
  const { store } = makeStore();
  const chunk = (kind: 'overview' | 'detail', offset: number, total: number, bytes: number[]): Uint8Array =>
    encodeWaveformChunk({ waveformId: 'wf', kind, offset, total, payload: Uint8Array.from(bytes) });
  store.handleBinary(chunk('overview', 0, 3, [1, 2, 3]));
  store.handleBinary(chunk('detail', 0, 6, [1, 2, 3]));
  store.handleBinary(chunk('detail', 5, 6, [9])); // gap: ignored
  let wf = store.waveforms.peek('wf')!;
  assert.equal(wf.overviewFilled, 3);
  assert.equal(wf.detailFilled, 3);
  store.handleBinary(chunk('detail', 3, 6, [4, 5, 6]));
  wf = store.waveforms.peek('wf')!;
  assert.deepEqual([...wf.detail!], [1, 2, 3, 4, 5, 6]);
  assert.equal(wf.detailFilled, 6);
  store.handleBinary(Uint8Array.of(1, 2, 3)); // garbage: ignored
  assert.equal(store.waveforms.size, 1);
});

test('the waveform cache evicts the least recently used entry', () => {
  const cache = new WaveformCache(2);
  const add = (id: string): void => {
    cache.addChunk({ waveformId: id, kind: 'overview', offset: 0, total: 1, payload: Uint8Array.of(1) });
  };
  add('a');
  add('b');
  cache.get('a'); // touch
  add('c');
  assert.ok(cache.peek('a'));
  assert.equal(cache.peek('b'), null);
  assert.ok(cache.peek('c'));
});

test('waveform_status updates the matching deck', () => {
  const { store } = makeStore();
  const msg = loaded(1);
  msg.waveform!.status = 'analysing';
  store.handleMessage(msg);
  store.handleMessage({ type: 'waveform_status', waveform_id: 'wf', status: 'analysing', progress: 0.4 });
  assert.equal(deckNotice(store.deck(1), null), 'Analysing waveform… 40%');
  store.handleMessage({ type: 'waveform_status', waveform_id: 'wf', status: 'ready', progress: 1 });
  assert.equal(deckNotice(store.deck(1), null), 'Loading waveform…');
});

test('deck notices cover every state in the brief', () => {
  const { store } = makeStore();
  assert.equal(deckNotice(store.deck(1), null), 'Deck empty');
  for (const [resolve, text] of [
    ['not_local', 'Local files only'],
    ['file_not_found', 'File not found'],
    ['ambiguous', 'Ambiguous track'],
  ] as const) {
    store.handleMessage(loaded(1, { resolve_status: resolve }));
    assert.equal(deckNotice(store.deck(1), null), text);
  }
  const noWf = loaded(1);
  noWf.waveform = null;
  store.handleMessage(noWf);
  assert.equal(deckNotice(store.deck(1), null), 'No waveform');
  store.handleMessage(loaded(1));
  store.handleBinary(encodeWaveformChunk({ waveformId: 'wf', kind: 'detail', offset: 0, total: 12, payload: new Uint8Array(12) }));
  assert.equal(deckNotice(store.deck(1), store.waveforms.peek('wf')), null);
});
