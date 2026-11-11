// Integration: the real Viewer networking and state (app/src) against the mock
// Sender, over a real WebSocket on localhost.

import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { setTimeout as sleep } from 'node:timers/promises';
import { PING_BURST, ViewerConnection } from '../../app/src/net/connection.ts';
import { ViewerStore } from '../../app/src/state/store.ts';
import { MOCK_LIBRARY } from './engine.ts';
import { MockSender } from './server.ts';

let sender: MockSender;
let port: number;
let store: ViewerStore;
let conn: ViewerConnection;

async function until(what: string, check: () => boolean, timeoutMs = 4000): Promise<void> {
  const start = performance.now();
  while (!check()) {
    if (performance.now() - start > timeoutMs) assert.fail(`timed out waiting for: ${what}`);
    await sleep(5);
  }
}
function ok(line: string): void {
  assert.equal(sender.command(line), null, line);
}
function connect(): void {
  store = new ViewerStore();
  conn = new ViewerConnection(store, { url: `ws://127.0.0.1:${port}`, deviceId: 'test-viewer', name: 'Test Viewer', appVersion: '0.1.0', os: 'test' });
  conn.start();
}

before(async () => {
  sender = new MockSender({ port: 0, decks: 2 });
  port = await sender.start();
});
after(async () => {
  conn?.stop();
  await sender.stop();
});

test('a Viewer that connects late gets the full snapshot', async () => {
  ok('load 1 1');
  ok('play 1');
  connect();
  await until('connected', () => store.connection === 'connected');
  await until('deck 1 loaded', () => store.deck(1).track !== null);
  const track = store.deck(1).track!;
  assert.equal(track.title, MOCK_LIBRARY[0]!.title);
  assert.equal(track.bpm_original, 124);
  assert.deepEqual(track.key, { camelot: '8A', musical: 'Am' });
  assert.equal(store.sender?.role, 'sender');
  assert.equal(store.source?.software, 'mock');
  assert.deepEqual(store.deckNumbers(), [1, 2]);
  assert.equal(sender.viewerCount, 1);

  const info = store.deck(1).waveform!;
  await until('waveform transferred', () => {
    const wf = store.waveforms.peek(info.waveform_id);
    return wf !== null && wf.detailFilled === info.frames * 3 && wf.overviewFilled === 3600;
  });
  assert.equal(info.frames, Math.round(372.4 * 150));
});

test('the clock syncs and latency on localhost is small', async () => {
  await until('first pong', () => store.clock.synced);
  assert.ok(store.clock.latencyMs! < 50, `latency ${store.clock.latencyMs} ms`);
});

test('playing: the estimate follows real time at the pitch rate', async () => {
  await until('playing state', () => store.deck(1).state?.playing === true);
  // Let the clock settle first: while the first pings come in, the estimate
  // still improves, and each improvement is blended into the position.
  await until('ping burst done', () => store.clock.sampleCount >= PING_BURST);
  await sleep(200);
  const measure = async (ms: number): Promise<number> => {
    const p0 = store.position(1)!;
    const t0 = performance.now();
    await sleep(ms);
    return (store.position(1)! - p0) / ((performance.now() - t0) / 1000);
  };
  assert.ok(Math.abs((await measure(400)) - 1) < 0.05, 'rate at 0 %');
  ok('pitch 1 10');
  await until('pitch seen', () => Math.abs((store.deck(1).state?.bpm_current ?? 0) - 136.4) < 1e-6);
  await sleep(150); // let the old-rate estimate settle
  assert.ok(Math.abs((await measure(400)) - 1.1) < 0.05, 'rate at +10 %');
  assert.equal(store.masterDeck(), 1);
});

test('pause holds, loop jump snaps, both directions', async () => {
  ok('pause 1');
  await until('paused', () => store.deck(1).state?.playing === false);
  await sleep(150);
  const held = store.position(1)!;
  await sleep(200);
  assert.ok(Math.abs(store.position(1)! - held) < 1e-6, 'position moved while paused');

  ok('jump 1 64');
  const beat = 60 / 124;
  await until('jump forward', () => Math.abs(store.position(1)! - (held + 64 * beat)) < 0.01);
  ok('jump 1 -16');
  await until('jump back', () => Math.abs(store.position(1)! - (held + 48 * beat)) < 0.01);
});

test('eject empties the deck; load brings a new track and waveform', async () => {
  ok('eject 1');
  await until('ejected', () => store.deck(1).track === null);
  assert.equal(store.position(1), null);
  ok('load 2 2');
  await until('deck 2 loaded', () => store.deck(2).track?.title === MOCK_LIBRARY[1]!.title);
  const info = store.deck(2).waveform!;
  await until('deck 2 waveform', () => store.waveforms.peek(info.waveform_id)?.detailFilled === info.frames * 3);
});

test('garbage from a client does not disturb the Sender or other Viewers', async () => {
  const rogue = new WebSocket(`ws://127.0.0.1:${port}`);
  rogue.binaryType = 'arraybuffer';
  await new Promise((resolve) => (rogue.onopen = resolve));
  rogue.send('not json');
  rogue.send('{"type":"deck_state"}');
  rogue.send(new Uint8Array([1, 2, 3]));
  rogue.send('{"type":"ping","t_viewer":1}'); // ping before hello is answered
  const reply = await new Promise<MessageEvent>((resolve) => (rogue.onmessage = resolve));
  assert.equal(JSON.parse(reply.data as string).type, 'pong');

  // A Viewer from the future is told why it is refused.
  rogue.send('{"type":"hello","protocol":99,"device_id":"x","name":"x","role":"viewer","app_version":"9","os":"x"}');
  const refusal = await new Promise<MessageEvent>((resolve) => (rogue.onmessage = resolve));
  assert.equal(JSON.parse(refusal.data as string).code, 'protocol_mismatch');
  await new Promise((resolve) => (rogue.onclose = resolve));

  ok('play 2');
  await until('still streaming', () => store.deck(2).state?.playing === true);
  assert.equal(sender.viewerCount, 1);
});

test('Sender restart: the Viewer shows last known, then reconnects by itself', async () => {
  await sender.stop();
  await until('disconnected', () => store.connection === 'disconnected');
  const frozen = store.position(2)!;
  await sleep(200);
  assert.equal(store.position(2), frozen, 'position must hold while disconnected');
  assert.equal(store.deck(2).track?.title, MOCK_LIBRARY[1]!.title);

  sender = new MockSender({ port, decks: 2 });
  await sender.start();
  ok('load 1 3');
  await until('reconnected', () => store.connection === 'connected', 8000);
  await until('fresh snapshot', () => store.deck(1).track?.title === MOCK_LIBRARY[2]!.title);
  assert.equal(store.deck(2).track, null); // the old deck 2 is gone with the old Sender
});
