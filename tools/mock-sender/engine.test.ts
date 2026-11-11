import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { MOCK_LIBRARY, MockEngine, SessionPlayer, parseCommand, parseSession } from './engine.ts';
import type { MockCommand, MockEvent } from './engine.ts';
import { buildOverview, fnv1a32, hex8, synthWaveform } from './synth.ts';

type StateEvent = Extract<MockEvent, { kind: 'state' }>;

function run(engine: MockEngine, line: string): MockEvent[] {
  const cmd = parseCommand(line);
  assert.equal(typeof cmd, 'object', `${line}: ${String(cmd)}`);
  const result = engine.apply(cmd as MockCommand);
  assert.equal(result.error, null, line);
  return result.events;
}
function stateOf(events: MockEvent[], deck: number): StateEvent {
  const ev = events.find((e) => e.kind === 'state' && e.deck === deck);
  assert.ok(ev, `no state for deck ${deck}`);
  return ev as StateEvent;
}

test('parseCommand accepts the documented commands', () => {
  assert.deepEqual(parseCommand('load 1'), { kind: 'load', deck: 1, track: null });
  assert.deepEqual(parseCommand('load 2 3'), { kind: 'load', deck: 2, track: 2 });
  assert.deepEqual(parseCommand('  PLAY   4 '), { kind: 'play', deck: 4 });
  assert.deepEqual(parseCommand('pause 1'), { kind: 'pause', deck: 1 });
  assert.deepEqual(parseCommand('pitch 1 1.6129'), { kind: 'pitch', deck: 1, percent: 1.6129 });
  assert.deepEqual(parseCommand('pitch 1 -2'), { kind: 'pitch', deck: 1, percent: -2 });
  assert.deepEqual(parseCommand('jump 1 -16'), { kind: 'jump', deck: 1, beats: -16 });
  assert.deepEqual(parseCommand('eject 2'), { kind: 'eject', deck: 2 });
  assert.deepEqual(parseCommand('master 2'), { kind: 'master', deck: 2 });
  assert.deepEqual(parseCommand('demo off'), { kind: 'demo', on: false });
});

test('parseCommand rejects bad input with a message', () => {
  for (const line of ['', 'play', 'play 0', 'play 5', 'play x', 'play 1 2', 'load 1 9', 'pitch 1', 'pitch 1 fast', 'jump 1', 'demo', 'demo maybe', 'scratch 1']) {
    assert.equal(typeof parseCommand(line), 'string', `"${line}" should be rejected`);
  }
});

test('load cycles through the library and starts paused at zero', () => {
  const e = new MockEngine(2);
  const first = run(e, 'load 1');
  assert.deepEqual(first[0], { kind: 'loaded', deck: 1, track: MOCK_LIBRARY[0], trackIndex: 0 });
  const s = stateOf(first, 1);
  assert.equal(s.pos_s, 0);
  assert.equal(s.playing, false);
  assert.equal(s.bpm_current, MOCK_LIBRARY[0]!.bpm);
  const second = run(e, 'load 2');
  assert.equal((second[0] as Extract<MockEvent, { kind: 'loaded' }>).trackIndex, 1);
});

test('commands on an empty or missing deck report an error', () => {
  const e = new MockEngine(2);
  assert.equal(e.apply({ kind: 'play', deck: 1 }).error, 'deck 1 is empty');
  assert.equal(e.apply({ kind: 'load', deck: 3, track: null }).error, 'this session has 2 decks');
});

test('playback advances with pitch and stops at the end of the track', () => {
  const e = new MockEngine(2);
  run(e, 'load 1 1');
  run(e, 'play 1');
  assert.ok(Math.abs(stateOf(e.tick(1), 1).pos_s - 1) < 1e-9);
  const pitched = stateOf(run(e, 'pitch 1 10'), 1);
  assert.ok(Math.abs(pitched.bpm_current - 136.4) < 1e-9);
  assert.ok(Math.abs(stateOf(e.tick(1), 1).pos_s - 2.1) < 1e-9);
  const end = stateOf(e.tick(10_000), 1);
  assert.equal(end.pos_s, MOCK_LIBRARY[0]!.duration_s);
  assert.equal(end.playing, false);
  assert.deepEqual(e.tick(1), []); // paused decks send nothing on tick
  assert.equal(stateOf(run(e, 'play 1'), 1).pos_s, 0); // play at the end restarts
});

test('pitch is clamped', () => {
  const e = new MockEngine(2);
  run(e, 'load 1 1');
  assert.ok(Math.abs(stateOf(run(e, 'pitch 1 400'), 1).bpm_current - 124 * 1.5) < 1e-9);
});

test('jump moves by beats in track time and clamps to the track', () => {
  const e = new MockEngine(2);
  run(e, 'load 1 1'); // 124 BPM
  run(e, 'play 1');
  e.tick(20);
  const back = stateOf(run(e, 'jump 1 -16'), 1);
  assert.ok(Math.abs(back.pos_s - (20 - (16 * 60) / 124)) < 1e-9);
  assert.equal(stateOf(run(e, 'jump 1 -4000'), 1).pos_s, 0);
  assert.equal(stateOf(run(e, 'jump 1 40000'), 1).pos_s, MOCK_LIBRARY[0]!.duration_s);
});

test('master rules: first deck to play, explicit change, handover on eject', () => {
  const e = new MockEngine(2);
  run(e, 'load 1');
  run(e, 'load 2');
  assert.equal(stateOf(run(e, 'play 1'), 1).is_master, true);
  const both = run(e, 'play 2');
  assert.equal(stateOf(both, 1).is_master, true);
  assert.equal(stateOf(both, 2).is_master, false);
  const swapped = run(e, 'master 2');
  assert.equal(stateOf(swapped, 1).is_master, false);
  assert.equal(stateOf(swapped, 2).is_master, true);
  const ejected = run(e, 'eject 2');
  assert.deepEqual(ejected[0], { kind: 'unloaded', deck: 2 });
  assert.equal(stateOf(ejected, 1).is_master, true); // handed to the deck still playing
  assert.equal(ejected.some((ev) => ev.kind === 'state' && ev.deck === 2), false);
});

test('the bundled demo session parses and exercises every Milestone 0 action', () => {
  const text = readFileSync(new URL('../../test-assets/sessions/demo.session', import.meta.url), 'utf8');
  const steps = parseSession(text);
  const kinds = new Set<string>(steps.map((s) => s.command?.kind ?? 'restart'));
  for (const k of ['load', 'play', 'pause', 'pitch', 'jump', 'eject', 'master', 'restart']) assert.ok(kinds.has(k), k);

  // Play it for two full loops against an engine: no command may fail.
  const engine = new MockEngine(2);
  const player = new SessionPlayer(steps);
  for (let i = 0; i < 60 * 100; i++) {
    for (const cmd of player.advance(1 / 60)) assert.equal(engine.apply(cmd).error, null, JSON.stringify(cmd));
    engine.tick(1 / 60);
  }
});

test('parseSession reports bad lines', () => {
  assert.throws(() => parseSession('play 1'), /line 1/);
  assert.throws(() => parseSession('1 play 1\n0 pause 1'), /backwards/);
  assert.throws(() => parseSession('# ok\n2 fly 1'), /line 2/);
  assert.deepEqual(parseSession('# only comments\n\n'), []);
});

test('SessionPlayer fires due commands once and loops on restart', () => {
  const player = new SessionPlayer(parseSession('0 load 1\n1 play 1\n2 restart'));
  assert.deepEqual(player.advance(0.5).map((c) => c.kind), ['load']);
  assert.deepEqual(player.advance(0.6).map((c) => c.kind), ['play']);
  assert.deepEqual(player.advance(1.0), []); // restart
  assert.deepEqual(player.advance(0.1).map((c) => c.kind), ['load']);
});

test('synthetic waveform has the wire format, peaks at 255 and is repeatable', () => {
  const wf = synthWaveform(372.4, 124, 42);
  assert.equal(wf.frames, 55860);
  assert.equal(wf.detail.length, 55860 * 3);
  assert.equal(wf.overview.length, 3600);
  assert.equal(Math.max(...wf.detail.subarray(0, 60000)) <= 255, true);
  assert.equal(wf.detail.reduce((m, v) => Math.max(m, v), 0), 255);
  assert.deepEqual(synthWaveform(372.4, 124, 42).detail, wf.detail);
  assert.notDeepEqual(synthWaveform(372.4, 124, 43).detail, wf.detail);
});

test('synthetic waveform has a kick on every beat outside the breakdown', () => {
  const bpm = 120; // 75 frames per beat at 150 fps
  const wf = synthWaveform(60, bpm, 1);
  for (let beat = 0; beat < 100; beat++) {
    const onBeat = wf.detail[beat * 75 * 3]!;
    const offBeat = wf.detail[(beat * 75 + 60) * 3]!;
    assert.ok(onBeat > 150 && offBeat < onBeat / 2, `beat ${beat}: ${onBeat} vs ${offBeat}`);
  }
});

test('overview holds the per-band maximum of the frames it covers', () => {
  const frames = 2400; // two frames per overview point
  const detail = new Uint8Array(frames * 3);
  detail[0] = 10; // frame 0, low
  detail[3] = 200; // frame 1, low  -> point 0 low = 200
  detail[2399 * 3 + 2] = 77; // last frame, high -> last point high = 77
  const ov = buildOverview(detail, frames);
  assert.equal(ov[0], 200);
  assert.equal(ov[1199 * 3 + 2], 77);
  // Shorter than the overview: every point still maps to a real frame.
  const tiny = buildOverview(Uint8Array.of(5, 6, 7), 1);
  assert.deepEqual([...tiny.subarray(0, 3), ...tiny.subarray(3597)], [5, 6, 7, 5, 6, 7]);
});

test('mock ids are stable', () => {
  assert.equal(hex8(fnv1a32('')), '811c9dc5');
  assert.equal(hex8(fnv1a32('a')), 'e40c292c');
  assert.equal(hex8(fnv1a32('foobar')), 'bf9cf968');
});
