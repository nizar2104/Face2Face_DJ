<p align="center"><img src="docs/brand/nomad-logo.jpg" alt="NO-MAD" width="360"></p>

# Face2Face DJ

**See your back-to-back partner's decks on your own screen, and they see yours.**

When two DJs play back-to-back (B2B) from two laptops in the same booth, each one
can only see their own software. Face2Face DJ links the two laptops over the local
network. Each laptop does two things at once:

- as a **Sender** it reads the state of its own DJ software's decks and streams it;
- as a **Viewer** it shows the partner's decks in a small always-on-top window:
  scrolling waveform, current and original BPM, musical key (Camelot and standard
  notation), remaining time, play/pause, master and on-air status, for up to four decks.

So each DJ can see what the other is playing, where the phrase ends and what key
and tempo to mix into, without leaning over the other laptop. Choose **Link up**
on both laptops for this two-way mode; **Share only** and **View only** are the
two halves on their own.

![Two linked windows, each showing the other laptop's decks](docs/screenshots/linked-pair.png)

How it works, briefly:

- **One WebSocket per direction.** A linked pair is two independent one-way links,
  one from each laptop's Sender to the other's Viewer. JSON messages for track loads
  and deck state (60 Hz per deck), binary frames for waveform data, a 1 Hz heartbeat.
  Spec: [`docs/PROTOCOL.md`](docs/PROTOCOL.md).
- **Clock sync + interpolation.** The Viewer estimates the Sender's clock from
  ping/pong round trips and extrapolates each deck's position between updates, so
  the waveform scrolls smoothly and stays in time with the real deck.
- **Read-only by design.** It never writes to a DJ library or to music files.
- **Planned sources:** Rekordbox (via rkbx_link) and Traktor. Milestone 0 uses mock
  decks with synthetic waveforms.

Built with Tauri 2: a Rust core (protocol, deck sources, waveforms, networking)
and a TypeScript + React UI drawing on Canvas 2D. Design decisions are logged in
[`docs/DECISIONS.md`](docs/DECISIONS.md). The project's working name was *Tandem*.

## Status: Milestone 0 (foundations, one computer)

Mock decks and synthetic waveforms only. No Rekordbox, Traktor, audio files,
discovery or pairing yet; the Sender listens on localhost only, so a linked pair
is two windows on the same computer for now.

| Part | State |
|---|---|
| Viewer UI, clock sync, interpolation, reconnect (TypeScript) | Built and tested |
| Mock sender tool (TypeScript) | Built and tested |
| Rust core (`crates/face2face-core`) | Built and tested (`cargo test`) |
| Desktop app (`app/src-tauri`), including two linked windows | Built and run on Linux. Not yet tried on macOS or Windows |

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
npm run dev             # then open http://localhost:1420 and choose "View only"
```

The mock sender takes commands on its terminal while it runs: `help` lists them
(`load 1`, `play 1`, `pause 1`, `pitch 1 2.5`, `jump 1 -16`, `eject 1`, ...).
`npm run mock-sender -- --no-demo` starts it without the script, `--decks 4` with four decks.
A browser can only view; sharing decks needs the desktop app.

**As the desktop app.**

```sh
cargo test              # Rust core tests
npm run tauri dev       # opens the Face2Face DJ window
```

**Two-way, both DJs see each other.** Choose **Link up** in the window, then start a
second copy of the app and choose **Link up** there too. While `tauri dev` runs,
the second copy is the program it built:

```sh
./target/debug/face2face-dj          # Windows: target\debug\face2face-dj.exe
```

Each window starts its own Sender with mock decks and shows the other window's
decks. The first to link is "Laptop A" (port 47801), the second "Laptop B" (port
47802); they load different mock tracks so you can tell them apart. The top row
of buttons controls your own mock decks, which the *other* window displays.

**One-way.** **Share only** starts the Sender and previews what a partner would
see. For a viewer of it, open http://localhost:1420 in a browser while `tauri dev`
runs and choose **View only**.

## Check it

```sh
npm test                # TypeScript unit + integration tests (Node's test runner)
npm run typecheck
npm run build           # production build of the UI
cargo test              # Rust unit + integration tests
```

## Layout

```
docs/                 PROJECT_BRIEF, DECISIONS, PROTOCOL, screenshots, brand (logo)
app/                  the desktop app
  src/                UI (TypeScript, React)
    protocol/         wire messages and binary waveform chunks
    net/              clock sync, interpolation, WebSocket connection
    state/            Viewer state and waveform cache
    waveform/         canvas drawing
    views/            RoleSelect, Viewer, Sender
    components/       DeckCard, WaveformView, StatusBar, KeyBadge
  src-tauri/          Tauri shell: commands that call face2face-core
crates/face2face-core/  Rust core: protocol, sources (mock), waveform (synthetic),
                        session, net (transport), sender
tools/mock-sender/    stand-in Sender for developing the Viewer
test-assets/          fixtures shared by the TypeScript and Rust tests
```

## Safety rules that already apply

Face2Face DJ never writes to a DJ library or to music files (brief, section 12).
Milestone 0 reads and writes nothing outside this folder and the browser's or
webview's own local storage (three display settings and a device id).
