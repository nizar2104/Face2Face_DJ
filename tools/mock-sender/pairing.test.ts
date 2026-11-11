// The Viewer's side of pairing (app/src/net/connection.ts) against a stand-in
// for a paired Sender. The real Sender's side is tested in Rust
// (crates/face2face-core/tests/localhost.rs).

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { setTimeout as sleep } from 'node:timers/promises';
import { WebSocketServer } from 'ws';
import { RECONNECT_MIN_S, ViewerConnection } from '../../app/src/net/connection.ts';
import { ViewerStore } from '../../app/src/state/store.ts';

const CODE = '483920';

/** Accepts `CODE` with a hello, refuses anything else the way the Sender does. */
async function pairedSender(): Promise<{ port: number; hellos: Record<string, unknown>[]; close: () => Promise<void> }> {
  const hellos: Record<string, unknown>[] = [];
  const server = new WebSocketServer({ host: '127.0.0.1', port: 0 });
  await new Promise((resolve) => server.once('listening', resolve));
  server.on('connection', (socket) => {
    socket.on('message', (data) => {
      const message = JSON.parse(String(data)) as Record<string, unknown>;
      if (message['type'] !== 'hello') return;
      hellos.push(message);
      if (message['pair_code'] === CODE) {
        socket.send(JSON.stringify({ type: 'hello', protocol: 1, device_id: 's', name: 'Laptop B', role: 'sender', app_version: '0.1.0', os: 'test' }));
      } else {
        socket.send(JSON.stringify({ type: 'error', code: 'pairing_failed', message: 'Wrong pairing code.' }));
        socket.close();
      }
    });
  });
  const { port } = server.address() as { port: number };
  return { port, hellos, close: () => new Promise((resolve) => server.close(() => resolve())) };
}

function viewer(port: number, pairCode?: string): { store: ViewerStore; conn: ViewerConnection } {
  const store = new ViewerStore();
  const conn = new ViewerConnection(store, { url: `ws://127.0.0.1:${port}`, deviceId: 'v', name: 'Viewer', appVersion: '0.1.0', os: 'test', pairCode });
  conn.start();
  return { store, conn };
}

async function until(what: string, check: () => boolean): Promise<void> {
  const start = performance.now();
  while (!check()) {
    if (performance.now() - start > 4000) assert.fail(`timed out waiting for: ${what}`);
    await sleep(5);
  }
}

test('the right code links', async () => {
  const sender = await pairedSender();
  const { store, conn } = viewer(sender.port, CODE);
  await until('connected', () => store.connection === 'connected');
  assert.equal(sender.hellos[0]!['pair_code'], CODE);
  assert.equal(store.error, null);
  conn.stop();
  await sender.close();
});

test('a wrong code is reported once and not retried', async () => {
  const sender = await pairedSender();
  const { store, conn } = viewer(sender.port, '000000');
  await until('refused', () => store.error !== null);
  assert.equal(store.error!.code, 'pairing_failed');
  assert.equal(store.connection, 'connecting');
  // Long enough for several reconnects, had it kept trying.
  await sleep(RECONNECT_MIN_S * 1000 * 3);
  assert.equal(sender.hellos.length, 1);
  assert.equal(store.error!.code, 'pairing_failed', 'the reason stays on screen');
  conn.stop();
  await sender.close();
});

test('without a code the hello carries none', async () => {
  const sender = await pairedSender();
  const { store, conn } = viewer(sender.port);
  await until('refused', () => store.error !== null);
  assert.ok(!('pair_code' in sender.hellos[0]!));
  conn.stop();
  await sender.close();
});
