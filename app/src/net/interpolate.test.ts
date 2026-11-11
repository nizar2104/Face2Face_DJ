import assert from 'node:assert/strict';
import { test } from 'node:test';
import { DeckInterpolator, MAX_EXTRAPOLATION_S, SMOOTH_TIME_S, SNAP_THRESHOLD_S } from './interpolate.ts';

const close = (a: number | null, b: number, eps = 1e-9): void => {
  assert.notEqual(a, null);
  assert.ok(Math.abs(a! - b) <= eps, `${a} vs ${b}`);
};

test('no state means no position', () => {
  assert.equal(new DeckInterpolator().position(1), null);
});

test('a playing deck advances at the pitch rate between messages', () => {
  const d = new DeckInterpolator();
  d.update({ ts: 100, pos_s: 60, playing: true, rate: 126 / 124 }, 100);
  close(d.position(100), 60);
  close(d.position(100.5), 60 + 0.5 * (126 / 124));
});

test('a paused deck holds its position', () => {
  const d = new DeckInterpolator();
  d.update({ ts: 100, pos_s: 42.5, playing: false, rate: 1.05 }, 100);
  close(d.position(100), 42.5);
  close(d.position(250), 42.5);
});

test('a message that arrives late is extrapolated from its timestamp', () => {
  const d = new DeckInterpolator();
  d.update({ ts: 100, pos_s: 10, playing: true, rate: 1 }, 100.02); // 20 ms on the wire
  close(d.position(100.02), 10.02);
});

test('a small disagreement is corrected smoothly, without a jump', () => {
  const d = new DeckInterpolator();
  d.update({ ts: 0, pos_s: 0, playing: true, rate: 1 }, 0);
  // At t=1 we show 1.000; the Sender says it is really 1.020 (inside the snap threshold).
  d.update({ ts: 1, pos_s: 1.02, playing: true, rate: 1 }, 1);
  close(d.position(1), 1.0, 1e-9); // nothing moved on screen at the moment of the update
  const half = d.position(1 + SMOOTH_TIME_S / 2)!;
  const truthHalf = 1.02 + SMOOTH_TIME_S / 2;
  close(half, truthHalf - 0.01, 1e-9); // half of the 20 ms error is left
  close(d.position(1 + SMOOTH_TIME_S), 1.02 + SMOOTH_TIME_S, 1e-9); // fully converged
  close(d.position(2), 2.02, 1e-9);
});

test('the picture never runs backwards while a small error is bled away', () => {
  const d = new DeckInterpolator();
  d.update({ ts: 0, pos_s: 0, playing: true, rate: 1 }, 0);
  d.update({ ts: 1, pos_s: 0.96, playing: true, rate: 1 }, 1); // we are 40 ms ahead
  let last = d.position(1)!;
  for (let t = 1.001; t < 1.2; t += 0.001) {
    const p = d.position(t)!;
    assert.ok(p >= last - 1e-12, `went backwards at t=${t}`);
    last = p;
  }
});

test('an error above the threshold snaps (loop, hot cue, seek)', () => {
  const d = new DeckInterpolator();
  d.update({ ts: 0, pos_s: 30, playing: true, rate: 1 }, 0);
  const jump = SNAP_THRESHOLD_S * 2;
  d.update({ ts: 1, pos_s: 31 + jump, playing: true, rate: 1 }, 1);
  close(d.position(1), 31 + jump);
});

test('positions may go backwards: a loop jump back is followed at once', () => {
  const d = new DeckInterpolator();
  d.update({ ts: 0, pos_s: 64, playing: true, rate: 1 }, 0);
  d.update({ ts: 2, pos_s: 58.26, playing: true, rate: 1 }, 2); // back 4 bars
  close(d.position(2), 58.26);
  close(d.position(2.5), 58.76);
});

test('pausing mid-play freezes without a visible step', () => {
  const d = new DeckInterpolator();
  d.update({ ts: 0.5, pos_s: 0.5, playing: true, rate: 1 }, 0.5);
  d.update({ ts: 1, pos_s: 1.0, playing: false, rate: 1 }, 1.01); // pause seen 10 ms late
  // We had drawn 1.010; the truth is 1.000. That 10 ms is smoothed away, then it holds.
  close(d.position(1.01), 1.01, 1e-9);
  close(d.position(1.01 + SMOOTH_TIME_S), 1.0, 1e-9);
  close(d.position(9), 1.0, 1e-9);
});

test('position is clamped to the track', () => {
  const d = new DeckInterpolator();
  d.setDuration(300);
  d.update({ ts: 0, pos_s: 299.9, playing: true, rate: 1 }, 0);
  close(d.position(0.5), 300);
  d.update({ ts: 1, pos_s: -3, playing: false, rate: 1 }, 1);
  close(d.position(1), 0);
});

test('a silent Sender is not extrapolated forever', () => {
  const d = new DeckInterpolator();
  d.update({ ts: 0, pos_s: 10, playing: true, rate: 1 }, 0);
  close(d.position(60), 10 + MAX_EXTRAPOLATION_S);
});

test('a bad rate falls back to 1', () => {
  for (const rate of [0, -1, Number.NaN, Number.POSITIVE_INFINITY]) {
    const d = new DeckInterpolator();
    d.update({ ts: 0, pos_s: 0, playing: true, rate }, 0);
    close(d.position(0.5), 0.5);
  }
});

test('reset drops the state', () => {
  const d = new DeckInterpolator();
  d.update({ ts: 0, pos_s: 5, playing: true, rate: 1 }, 0);
  d.reset();
  assert.equal(d.hasState, false);
  assert.equal(d.position(1), null);
});

test('60 Hz updates with timing jitter stay within a millisecond of the truth', () => {
  const d = new DeckInterpolator();
  const rate = 1.03;
  let worst = 0;
  let seed = 12345;
  const rand = (): number => ((seed = (seed * 1103515245 + 12345) & 0x7fffffff), seed / 0x7fffffff);
  for (let i = 0; i < 600; i++) {
    const ts = i / 60;
    const arrival = ts + 0.0005 + rand() * 0.002; // 0.5-2.5 ms network delay
    d.update({ ts, pos_s: ts * rate, playing: true, rate }, arrival);
    const mid = arrival + 0.008;
    if (i > 10) worst = Math.max(worst, Math.abs(d.position(mid)! - mid * rate));
  }
  assert.ok(worst < 0.001, `worst error ${worst}`);
});
