// CLI: a mock Face2Face DJ Sender for developing and testing the Viewer without
// Rekordbox, Traktor or the desktop app.
//
//   npm run mock-sender                  scripted demo on ws://127.0.0.1:47801
//   npm run mock-sender -- --no-demo     decks controlled from stdin only
//   npm run mock-sender -- --decks 4 --port 47811 --session path/to/file.session

import { readFileSync } from 'node:fs';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { DEFAULT_PORT } from '../../app/src/protocol/messages.ts';
import { COMMAND_HELP, parseSession } from './engine.ts';
import { MockSender } from './server.ts';

const DEFAULT_SESSION = fileURLToPath(new URL('../../test-assets/sessions/demo.session', import.meta.url));

const { values } = parseArgs({
  options: {
    port: { type: 'string', default: String(DEFAULT_PORT) },
    host: { type: 'string', default: '127.0.0.1' },
    decks: { type: 'string', default: '2' },
    session: { type: 'string', default: DEFAULT_SESSION },
    'no-demo': { type: 'boolean', default: false },
    help: { type: 'boolean', short: 'h', default: false },
  },
});

if (values.help) {
  console.log(`Usage: node tools/mock-sender/main.ts [--port N] [--host ADDR] [--decks 2|4] [--session FILE] [--no-demo]\n\n${COMMAND_HELP}`);
  process.exit(0);
}

const port = Number(values.port);
const decks = Number(values.decks);
if (!Number.isInteger(port) || port < 0 || port > 65535) throw new Error('--port must be 0-65535');
if (decks !== 2 && decks !== 4) throw new Error('--decks must be 2 or 4');

const sender = new MockSender({
  port,
  host: values.host,
  decks,
  session: parseSession(readFileSync(values.session, 'utf8')),
  demo: !values['no-demo'],
  log: (line) => console.log(line),
});

const bound = await sender.start();
console.log(`Face2Face DJ mock sender on ws://${values.host}:${bound} (${decks} decks, demo ${values['no-demo'] ? 'off' : 'on'})`);
if (values.host !== '127.0.0.1' && values.host !== 'localhost') {
  console.log('Warning: there is no pairing yet, so anyone who can reach this address can connect.');
}
console.log('Type a command and press Enter ("help" lists them). Ctrl+C quits.');

const rl = createInterface({ input: process.stdin });
rl.on('line', (line) => {
  const text = line.trim();
  if (text === '') return;
  if (text === 'help') {
    console.log(COMMAND_HELP);
    return;
  }
  const error = sender.command(text);
  console.log(error === null ? `ok: ${text}` : `error: ${error}`);
});

process.on('SIGINT', () => {
  void sender.stop().then(() => process.exit(0));
});
