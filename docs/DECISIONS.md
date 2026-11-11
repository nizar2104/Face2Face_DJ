# Decisions

One short entry per decision that someone would otherwise have to rediscover.
Newest at the bottom. "Revisit when" says what would make the decision wrong.

## Milestone 0 (11 November 2026)

**D1. Stack: Tauri 2, Rust core, TypeScript + React UI, Canvas 2D.**
As the brief recommends. React rather than Svelte because React could be built
and run where Milestone 0 was written, and Svelte could not. The cost of
switching is low: the frame loop, canvas drawing, networking and state do not
use React at all. Revisit when: someone has a concrete reason to prefer Svelte.

**D2. The core is its own crate, `crates/tandem-core`, not modules under `app/src-tauri/src`.**
The brief proposed the modules inside the Tauri crate. Kept separate so that
`cargo test` needs no webview libraries, tests build fast, and command-line
tools can link the same code. `app/src-tauri` is a thin shell of Tauri commands.
Module names inside the core follow the brief (`protocol`, `net`, `sources`, `waveform`).

**D3. The Viewer's networking runs in the webview (TypeScript `WebSocket`), not in Rust.**
One hop fewer between socket and canvas, and the Viewer UI runs unchanged in a
plain browser, which is how it is developed and tested. Clock sync uses
`performance.now()`, which is monotonic.
Revisit when: (a) WKWebView refuses `ws://` to a LAN address from the `tauri://`
origin (to test on a Mac in M1); (b) WebKit's 1 ms timer resolution proves too
coarse for clock sync; (c) discovery and pairing in M1 make a Rust client simpler.
`app/src/net/connection.ts` is the only file that would change.

**D4. The protocol version is sent once, in `hello`, not in every message.**
The brief says every message carries it, but its own examples only show it in
`hello`, and `deck_state` goes out 60 times a second per deck. See PROTOCOL.md section 2.

**D5. `waveform_chunk` header: fixed 20 bytes + ASCII id, with a `total` field.**
The brief asked for id, kind, offset, length. `total` was added so the Viewer
can allocate and detect completion without knowing sizes from elsewhere. Ids are
opaque strings (not raw SHA-1 bytes) so the mock, and later the real hash, can
use the same field. See PROTOCOL.md section 4.

**D6. `heartbeat` is its own message; 3 s of silence means disconnected.**
The brief lists a 1 Hz heartbeat but no message for it.

**D7. Until pairing exists, the Sender listens on 127.0.0.1 only.**
The brief says only paired devices may connect; pairing is Milestone 1. Opening
the port to the network before that would let anything on it read deck data.
Known gap: any local web page can connect to a localhost WebSocket. The pairing
token in M1 closes it.

**D8. The role is asked at every launch and not remembered.**
Two instances on one machine (the Milestone 0 setup) share the same app storage,
so a remembered role would make both start as the same role. `?role=viewer` or
`?role=sender` in the URL skips the question in a browser. Revisit in M5 (wizard).

**D9. The mock exists twice: `tools/mock-sender` (TypeScript) and `MockSource` (Rust).**
The TypeScript one drives the Viewer in a browser without any Rust build; the
Rust one is the `DeckSource` the desktop app uses. To stop them drifting they
share one command vocabulary, one session file (`test-assets/sessions/demo.session`)
and the protocol fixtures. Their synthetic waveforms use the same algorithm but
are not guaranteed to be byte-identical.

**D10. TypeScript tests use Node's built-in test runner and type stripping.**
No test framework to install or configure. Needs Node 22.18 or newer. The code
therefore uses only erasable TypeScript syntax (`erasableSyntaxOnly`) and
imports with explicit `.ts` extensions.

**D11. Waveform drawing: redraw every frame, one column per device pixel, on a global pixel grid.**
Each screen column shows the maximum of the detail frames it covers. Columns
are numbered from track time 0, not from the window edge, so a column keeps its
height as it scrolls and the picture does not shimmer. Scrolling moves in whole
device pixels. Revisit when: profiling on a real laptop shows the redraw costs
too much (then pre-render strips and blit them).

**D12. Interpolation: snap above 50 ms of error, otherwise blend linearly over 100 ms.**
Values from the brief. The blend restarts on each new state, so at 60 Hz it
behaves like an exponential decay with a time constant of about 100 ms.
Extrapolation stops after 1 s without a new state.
Each state's timestamp is converted to the viewer clock once, on arrival. So
when the clock-sync estimate improves, the change reaches the picture through
this same blend rather than as a step. (Converting at draw time made the
position step by the size of the correction; an integration test caught it.)

**D13. Remaining time is track time (duration minus position), not divided by the pitch rate.**
The brief does not say. Check against what Rekordbox and Traktor display in M3/M4.

**D14. The webview content security policy is off (`csp: null`) for now.**
The app loads only its own bundled files. A strict policy must be tested on
real WebView2 and WKWebView, which was not possible in M0. Do it in M5 at the latest.

**D15. No licence chosen yet.**
The brief asks for it at M0; it is the owner's decision. Everything used so far
is MIT or Apache-2.0 (Tauri, tokio, tokio-tungstenite, futures, serde, anyhow,
tracing, React, Vite, ws, TypeScript), so no option is closed. Crates are marked
`publish = false` and packages `private`.

**D16. WebSocket server on `tokio-tungstenite` directly; no `axum` yet.**
Milestone 0 needs one WebSocket endpoint. `axum` comes in with the Traktor HTTP
receiver (M3).

**D17. The UI reaches Rust through `window.__TAURI__` (`withGlobalTauri`), not the `@tauri-apps/api` package.**
Keeps the web build free of Tauri imports, so the same bundle runs in a browser.
All calls are in `app/src/tauri.ts`.

**D18. `SourceEvent::DeckState` has plain `bool` fields for `playing`, `is_master`, `on_air`.**
The brief marks them optional because Rekordbox via rkbx_link cannot report all
of them. They become `Option` together with the first source that needs it (M4),
so the inference rules are written against real data.

**D19. The mock deck has no mixer: `on_air` equals `playing`.**

## After Milestone 0

**D20. The project is named Face2Face DJ; "Tandem" was the working name.**
Renamed everywhere except this log and the file name of the original brief:
crate `face2face-core`, app crate `face2face-dj`, npm packages `@face2face-dj/*`,
Tauri identifier `app.face2facedj.desktop`, local-storage keys `face2face-dj.*`.
The wire protocol is unchanged (no name appears in it).

**D21. Two-way: both laptops share and both view ("Link up"). It is two one-way links, not a new protocol.**
The brief has one Sender and one Viewer. In a back-to-back set both DJs need to
see each other, so the default mode is now two-way: each laptop runs a Sender
and a Viewer. Each direction is its own WebSocket connection with the unchanged
protocol, so one direction can fail or reconnect without the other, and the
one-way roles stay available as "Share only" and "View only".
Milestone 0 (one computer, loopback only, see D7): a linking instance takes the
first free port of 47801 and 47802 and views the other one, so the first to link
is "Laptop A" and the second "Laptop B", with no setting to choose. The two
sides start at different tracks of the mock library.
For Milestone 1: discovery must find the partner's Sender from both sides, and
pairing must be mutual (one confirmation should authorise both directions).
The window shows only the partner's decks, not one's own: those are in the DJ's
own software. Revisit when: someone wants both on one screen.

**D22. The NO-MAD logo is on the start screen and in the README.**
`docs/brand/nomad-logo.jpg` is the file as supplied (white on dark).
`app/public/nomad-logo.png` is made from it: cropped, with the dark background
turned into transparency. The app icons (`app/src-tauri/icons`) are still the
Milestone 0 placeholders; the logo is a wide wordmark and does not fit a square.
