# Tandem

Links two DJ laptops in the same booth: the **Viewer** (computer 2) shows the
waveform, BPM, key, time and status of the decks on the **Sender** (computer 1).
Scope and design: `docs/PROJECT_BRIEF.md`.

## Status: Milestone 0 (foundations, one computer)

Mock decks and synthetic waveforms only. No Rekordbox, Traktor, audio files,
discovery or pairing yet; the Sender listens on localhost only.

| Part | State |
|---|---|
| Viewer UI, clock sync, interpolation, reconnect (TypeScript) | Built and tested |
| Mock sender tool (TypeScript) | Built and tested |
| Rust core (`crates/tandem-core`) and Tauri shell (`app/src-tauri`) | **Written, never compiled.** The environment Milestone 0 was written in had no Rust toolchain. Run `cargo test` first and expect to fix compile errors. |

![Viewer with two mock decks](docs/screenshots/viewer-two-decks.png)

(480x220 window at 2x, driven by `npm run mock-sender`. More in `docs/screenshots/`.)

## What you need

- Node.js 22.18 or newer, with npm.
- For the desktop app: Rust (stable, 1.77.2 or newer) and the Tauri 2
  prerequisites for your OS: https://tauri.app/start/prerequisites/

## Run it

```sh
npm install
```

**In a browser (no Rust needed).** Two terminals:

```sh
npm run mock-sender     # stand-in Sender with a scripted demo on ws://127.0.0.1:47801
npm run dev             # then open http://localhost:1420 and choose "View partner"
```

The mock sender takes commands on its terminal while it runs: `help` lists them
(`load 1`, `play 1`, `pause 1`, `pitch 1 2.5`, `jump 1 -16`, `eject 1`, ...).
`npm run mock-sender -- --no-demo` starts it without the script, `--decks 4` with four decks.

**As the desktop app.**

```sh
cargo test              # Rust core tests; do this first (see Status)
npm run tauri dev       # opens the Tandem window
```

Choose **Share my decks**: the window starts the Rust Sender with mock decks,
shows buttons for them, and previews what the partner sees through the real
WebSocket path. For a second window on the same machine, open
http://localhost:1420 in a browser while `tauri dev` runs and choose **View partner**.

## Check it

```sh
npm test                # TypeScript unit + integration tests (Node's test runner)
npm run typecheck
npm run build           # production build of the UI
cargo test              # Rust unit + integration tests
```

## Layout

```
docs/                 PROJECT_BRIEF, DECISIONS, PROTOCOL
app/                  the desktop app
  src/                UI (TypeScript, React)
    protocol/         wire messages and binary waveform chunks
    net/              clock sync, interpolation, WebSocket connection
    state/            Viewer state and waveform cache
    waveform/         canvas drawing
    views/            RoleSelect, Viewer, Sender
    components/       DeckCard, WaveformView, StatusBar, KeyBadge
  src-tauri/          Tauri shell: commands that call tandem-core
crates/tandem-core/   Rust core: protocol, sources (mock), waveform (synthetic),
                      session, net (transport), sender
tools/mock-sender/    stand-in Sender for developing the Viewer
test-assets/          fixtures shared by the TypeScript and Rust tests
```

## Safety rules that already apply

Tandem never writes to a DJ library or to music files (brief, section 12).
Milestone 0 reads and writes nothing outside this folder and the browser's or
webview's own local storage (three display settings and a device id).
