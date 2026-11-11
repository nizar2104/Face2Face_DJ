import assert from 'node:assert/strict';
import { test } from 'node:test';
import { formatCode, isLocalUrl, loadPartner, myCode, parseAddress, parseCode, partnerUrl, savePartner } from './pairing.ts';

test('a code is 6 digits, however it is spaced', () => {
  assert.equal(parseCode('483920'), '483920');
  assert.equal(parseCode(' 483 920 '), '483920');
  assert.equal(parseCode('483-920'), '483920');
  assert.equal(parseCode('000001'), '000001');
  for (const bad of ['', '48392', '4839201', '48392a', '483.920']) assert.equal(parseCode(bad), null, bad);
  assert.equal(formatCode('483920'), '483 920');
});

test('an address is an IPv4 address or a host name, with an optional port', () => {
  assert.equal(parseAddress('192.168.1.80'), '192.168.1.80');
  assert.equal(parseAddress('  169.254.12.3 '), '169.254.12.3');
  assert.equal(parseAddress('192.168.1.80:47811'), '192.168.1.80:47811');
  assert.equal(parseAddress('192.168.1.80:47801'), '192.168.1.80', 'the default port is dropped');
  assert.equal(parseAddress('ws://192.168.1.80:47811/'), '192.168.1.80:47811');
  assert.equal(parseAddress('DJ-B.local'), 'DJ-B.local');
  assert.equal(parseAddress('macbook'), 'macbook');
  for (const bad of ['', '192.168.1', '300.1.1.1', '1.2.3.4.5', '192.168.1.80:0', '192.168.1.80:70000', '192.168.1.80:', 'a b', '-x.local', 'x/y', 'http://x', '[::1]', '192.168..1']) {
    assert.equal(parseAddress(bad), null, bad);
  }
});

test('a partner becomes a ws:// address', () => {
  assert.equal(partnerUrl({ kind: 'local' }, 47802), 'ws://127.0.0.1:47802');
  assert.equal(partnerUrl({ kind: 'network', address: '192.168.1.80', code: '483920' }, 47802), 'ws://192.168.1.80:47801');
  assert.equal(partnerUrl({ kind: 'network', address: 'dj-b.local:47811', code: '483920' }, 47802), 'ws://dj-b.local:47811');
  assert.ok(isLocalUrl('ws://127.0.0.1:47801') && isLocalUrl('ws://localhost:1'));
  assert.ok(!isLocalUrl('ws://192.168.1.80:47801') && !isLocalUrl('ws://127.0.0.10.example:1'));
});

test('the code and the partner are remembered, and bad stored values are dropped', () => {
  const stored = new Map<string, string>();
  (globalThis as { localStorage?: unknown }).localStorage = {
    getItem: (key: string) => stored.get(key) ?? null,
    setItem: (key: string, value: string) => void stored.set(key, value),
    removeItem: (key: string) => void stored.delete(key),
  };
  try {
    const code = myCode();
    assert.match(code, /^\d{6}$/);
    assert.equal(myCode(), code, 'the same code every time');

    assert.equal(loadPartner(), null);
    savePartner({ kind: 'network', address: '192.168.1.80', code: '483920' });
    assert.deepEqual(loadPartner(), { kind: 'network', address: '192.168.1.80', code: '483920' });
    savePartner({ kind: 'local' });
    assert.deepEqual(loadPartner(), { kind: 'local' });
    savePartner(null);
    assert.equal(loadPartner(), null);

    for (const bad of ['{', '"x"', '{"kind":"network","address":"not an address","code":"483920"}', '{"kind":"network","address":"1.2.3.4","code":"12"}']) {
      stored.set('face2face-dj.pairing.partner', bad);
      assert.equal(loadPartner(), null, bad);
    }
  } finally {
    delete (globalThis as { localStorage?: unknown }).localStorage;
  }
});

test('without storage there is still a code, and nothing throws', () => {
  assert.match(myCode(), /^\d{6}$/);
  assert.equal(loadPartner(), null);
  savePartner({ kind: 'local' });
});
