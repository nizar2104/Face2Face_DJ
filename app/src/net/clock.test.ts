import assert from 'node:assert/strict';
import { test } from 'node:test';
import { CLOCK_WINDOW, ClockSync } from './clock.ts';

const close = (a: number, b: number, eps = 1e-9): void => assert.ok(Math.abs(a - b) <= eps, `${a} vs ${b}`);

test('unsynced clock is the identity and reports no latency', () => {
  const c = new ClockSync();
  assert.equal(c.synced, false);
  assert.equal(c.toSender(12.5), 12.5);
  assert.equal(c.toViewer(12.5), 12.5);
  assert.equal(c.latencyMs, null);
  assert.equal(c.best, null);
});

test('symmetric round trip recovers the exact offset', () => {
  const c = new ClockSync();
  // Sender clock = viewer clock + 4000. 2 ms each way.
  c.addSample(1000.0, 5000.002, 1000.004);
  close(c.best!.offset, 4000, 1e-6);
  close(c.best!.rtt, 0.004, 1e-9);
  close(c.toSender(1000.01), 5000.01, 1e-6);
  close(c.toViewer(5000.01), 1000.01, 1e-6);
  assert.equal(c.sampleCount, 1);
  close(c.latencyMs!, 2, 1e-6);
});

test('the minimum-RTT sample wins over later, slower samples', () => {
  const c = new ClockSync();
  c.addSample(10.0, 110.001, 10.002); // rtt 2 ms, offset 100
  c.addSample(12.0, 112.03, 12.04); // rtt 40 ms, asymmetric: offset estimate 100.01
  c.addSample(14.0, 114.004, 14.03); // rtt 30 ms, asymmetric the other way
  close(c.best!.rtt, 0.002, 1e-9);
  close(c.best!.offset, 100, 1e-6);
});

test('offset error is bounded by half the round trip', () => {
  const c = new ClockSync();
  const trueOffset = 250;
  // 1 ms out, 9 ms back: the worst kind of asymmetry.
  c.addSample(50.0, 50.001 + trueOffset, 50.01);
  assert.ok(Math.abs(c.best!.offset - trueOffset) <= c.best!.rtt / 2 + 1e-9);
});

test('only the last CLOCK_WINDOW samples are kept', () => {
  const c = new ClockSync();
  c.addSample(0, 100.0005, 0.001); // best ever, rtt 1 ms
  for (let i = 1; i <= CLOCK_WINDOW; i++) c.addSample(i, 100 + i + 0.005, i + 0.01);
  close(c.best!.rtt, 0.01, 1e-9); // the 1 ms sample has aged out
});

test('nonsense samples are ignored', () => {
  const c = new ClockSync();
  c.addSample(10, 5, 9); // negative round trip
  c.addSample(10, Number.NaN, 11);
  c.addSample(Number.NaN, 5, 11);
  assert.equal(c.synced, false);
});

test('reset forgets everything', () => {
  const c = new ClockSync();
  c.addSample(1, 2, 1.001);
  c.reset();
  assert.equal(c.synced, false);
});
