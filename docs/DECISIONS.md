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

## Deck readers (11 November 2026)

Readers for real DJ software, ahead of the brief's order (they were Milestones
3 and 4). How each one works and what it needs is in `docs/SOURCES.md`.

**D23. Four readers behind the existing `DeckSource` trait; the source is chosen in the Sender window.**
Traktor (HTTP receiver for the traktor-api-client mod), rekordbox (OSC from
rkbx_link), Pro DJ Link (virtual player, for CDJ/XDJ hardware) and Mixxx (MIDI
from a controller mapping we ship). `SourceKind` names them; `Sender` takes one.
The choice is not remembered between launches, for the reason in D8.
None has been run against the real program or hardware yet; they are built from
the published formats and tested with hand-built messages.

**D24. A track from a reader has no waveform (`waveform: null`), rather than a synthetic one.**
A made-up waveform under a real track title would mislead the DJ reading it.
The synthetic waveform moved from the session into the mock, which is the only
thing it was ever true for. Revisit: Milestone 2 (audio analysis) fills it in.
Pro DJ Link players could also be asked for rekordbox's own waveform.

**D25. Readers share one deck model, `sources/board.rs`, which sends 20 states a second.**
DJ software reports the playhead between once a second (Traktor) and 60 times
(rkbx_link). The board keeps the last report as an anchor and sends the
extrapolated position every 50 ms, so the Viewer never reaches its 1 s
extrapolation limit, and each reader only translates its input into
"load / update / unload".

**D26. `playing`, `is_master`, `on_air` stay plain booleans; a reader infers what its software does not say.**
D18 expected them to become optional with the first source that lacks one.
rkbx_link lacks all three, and the inference is simple enough to do at the
source (playing = position advances, on air = playing, master = the deck at the
master's position), so the protocol and the Viewer did not have to change.
Revisit when: an inferred value proves wrong often enough that the Viewer should
show "unknown" instead.

**D27. `duration_s: 0` and `bpm_original: 0` mean unknown.**
rkbx_link sends no track length. The Viewer shows `--:--` for the remaining
time and stops clamping the position. No new field.

**D28. rekordbox on a laptop is read through rkbx_link, not through Pro DJ Link.**
PRO DJ LINK Bridge reads players on the network, not rekordbox's own decks.
rkbx_link is the only known reader for Performance mode. Its cost is real: it
reads rekordbox's memory, so it needs a build for each rekordbox version, a
paid licence on Windows for recent versions, and on macOS an Apple Silicon Mac
and a re-signed rekordbox. Revisit when: AlphaTheta gives rekordbox an output
for deck state, or someone confirms that Performance mode announces its decks
on Pro DJ Link.

**D29. The Pro DJ Link reader listens on all interfaces; D7 still holds for our own port.**
The players broadcast, so the reader cannot listen on loopback. It parses
fixed-layout packets and bounds everything it allocates from them. The Sender's
WebSocket port stays on 127.0.0.1 until pairing exists.

**D30. HTTP through `axum` (as D16 planned), MIDI through `midir`, interfaces through `netdev`, socket options through `socket2`.**
`midir` needs the ALSA development package to build on Linux
(`libasound2-dev`). On Intel Macs its CoreMIDI layer (`coremidi` 0.9.2) reads
packets through unaligned pointers, which a debug build aborts on; the workspace
`Cargo.toml` turns that check off for that one crate (found by sending real MIDI
to the reader, not by the unit tests). Mixxx was not in the brief; it is here because its mapping
route needed no change to Mixxx itself.

**D31. For Mixxx we ship the mapping (`tools/mixxx-mapping`); for Traktor we point to traktor-api-client.**
Mixxx had no existing way out, so the script and its message format are ours,
tested from both ends against `test-assets/mixxx/deck_state.hex`. Traktor's mod
exists, is MIT-licensed and installs unchanged on Traktor Pro 4.5, so we speak
its format instead of shipping a second mod. Revisit when: its one position
report per second proves too coarse (then ship a variant with a shorter interval).

## Linking two laptops (12 November 2026)

Part of what the brief calls Milestone 1, to a simpler design than the brief's
(which is not in this repository): chosen by the owner over waiting for it.
The user's side is `docs/SETUP.md`.

**D32. A Sender is reachable from the network only with a pairing code. This replaces D7's "loopback only".**
`Access::Paired { code }` is the one way to listen on all interfaces; without a
code a Sender still listens on 127.0.0.1 alone. The Viewer sends the code in
`hello.pair_code`; a wrong one gets `pairing_failed`. The app makes a 6-digit
code once, keeps it, and shows it for the partner to type.
Six digits can be guessed, so each address gets one attempt a second
(`net/pairing.rs`): eleven days to try them all. A correct code clears the
address, so a reconnecting partner never waits.
What it is not: encryption. Code and deck data cross the network in the clear.
Judged proportionate: the data is what is on the decks, no audio and no library.
The same code also closes D7's known gap (any local web page could connect),
for Senders started by the app.
Revisit when: the brief's `pair_request` / `pair_confirm` exchange is
implemented, or anything more sensitive than deck state is sent.

**D33. No discovery yet: each DJ types the other's address and code.**
The window shows this laptop's addresses (`net/addresses.rs`), the default
route's first. That is two entries, not the single mutual confirmation D21
hoped for. It needs no extra dependency and works on any network where the
laptops can reach each other, including a direct cable (self-assigned
169.254 addresses). Its weakness: addresses change between sessions on Wi-Fi,
and the partner has to be entered again.
Revisit when: that proves annoying in use. Then find the partner by mDNS, and
let the first connection carry the return address so one entry links both ways.

**D34. D3's first doubt is answered for development builds on macOS: the webview connects `ws://` to a LAN address.**
Seen with `tauri dev` on macOS 15: the window linked to a Sender at the
laptop's own Wi-Fi address. For the packaged app, `app/src-tauri/Info.plist`
adds `NSAllowsLocalNetworking` and the local-network usage text macOS asks for.
The packaged app, and Windows, are still untested.

**D35. The pairing code and the partner live in the webview's local storage, like the device id.**
So the core stays free of files, and two windows on one computer share them:
"My partner is on this computer" therefore needs no address, only the other
link port (D21). The role and the source are still asked at every launch (D8);
the partner is remembered because typing it is the expensive part.
"View only" views this computer until another laptop is entered, which keeps
`npm run mock-sender` plus a browser working without set-up.
