# Deck sources

Where the Sender gets deck data from. Choose the source in the Sender window
(the menu at the left of the control row); `?source=traktor` in the URL, or
`cargo run --example sender -- traktor` without a window, select it too.

None of the DJ programs has an official way to read its decks, so each reader
uses what that program happens to offer. This page says how each one works,
what to install, and what it cannot know. All readers only read: nothing is
written to a library, a music file or a player.

| Source | How the data gets out | Needs installing | Checked against the real software |
|---|---|---|---|
| Traktor Pro 3 / 4 | QML mod that POSTs JSON to `localhost:8080` | [traktor-api-client](https://github.com/ErikMinekus/traktor-api-client) into Traktor | No |
| rekordbox (Performance mode) | [rkbx_link](https://github.com/grufkork/rkbx_link) reads rekordbox's memory and sends OSC | rkbx_link, and a licence for recent rekordbox versions | No |
| Pro DJ Link (CDJ, XDJ) | We join the network as a virtual player, like PRO DJ LINK Bridge | Nothing | No |
| Mixxx 2.4+ | Controller mapping that sends MIDI, plus Mixxx's library for titles and waveforms | The mapping in `tools/mixxx-mapping` | Yes: Mixxx 2.5.6 on macOS, Mac to Windows PC |

"No" means: built from the published formats and tested with hand-built
messages (`cargo test`, `npm test`), and each reader was run for real with a
stand-in for the DJ software sending over the actual HTTP, UDP or MIDI
connection. None has yet run next to the real program or hardware, so expect to
find differences the first time. The Mixxx script was also type-checked against
the scripting API definitions inside Mixxx 2.5.6.

Common to all readers:

- **No waveform yet, except for Mixxx.** Waveforms come from analysing the
  audio file (Milestone 2). Until then the Viewer shows "No waveform" for real
  decks, with title, tempo, key, time, play, master and on-air working. Mixxx
  is the exception: its own waveform is read from its library.
- **Position between reports.** DJ software reports the playhead far less often
  than the Viewer draws. The Sender carries it forward at the deck's tempo
  between reports (`sources/board.rs`) and corrects on each new one.
- **Ports are fixed** to each tool's default for now. A reader that cannot
  open its port says so in the Sender window's status.

## Traktor

`crates/face2face-core/src/sources/traktor.rs`

Traktor runs QML scripts for its controllers, and those can read every deck
property and make HTTP requests. traktor-api-client (MIT) is such a script.

Set-up, once:

1. Quit Traktor. Open its `qml/CSI` folder:
   - macOS: right-click `Traktor Pro 4.app` (in `/Applications/Native Instruments/Traktor Pro 4`), Show Package Contents, then `Contents/Resources/qml/CSI`.
   - Windows: `C:\Program Files\Native Instruments\Traktor Pro 4\Resources64\qml\CSI`.
2. Back up the `D2` folder there, then replace it with the `D2` folder from traktor-api-client.
   (Its `D2.qml` is Traktor's own plus two lines; compared against Traktor Pro 4.5.0.)
3. Start Traktor. Without a Kontrol D2: Preferences > Controller Manager > Add… > Traktor > Kontrol D2.
4. In Face2Face DJ choose **Traktor**. The status changes from "waiting for Traktor" to "running" at the first change in Traktor (load a track).

A Traktor update restores the original `D2` folder; repeat step 2 after one.

Limits:

- The mod reports the position on play and pause and once a second while
  playing. A jump inside a playing track (hot cue, loop, seek) shows up to a
  second late. Lowering `interval: 1000` in the mod's `Api/ApiDeck.qml` shortens that.
- Traktor says nothing when it quits, so the status stays "running".
- Port 8080 is popular. If something else has it, change `API_BASE_URL` in the
  mod's `Api/ApiClient.js` and `DEFAULT_HTTP_PORT` here together.

## rekordbox (Performance mode)

`crates/face2face-core/src/sources/rekordbox.rs`, `sources/osc.rs`

rekordbox has no interface for this at all. rkbx_link (GPL-3.0, a separate
program; we only receive what it sends) reads the decks out of rekordbox's
memory, which ties it to exact rekordbox versions:

- Windows: needs a licence from its author for rekordbox versions newer than 7.2.2.
- macOS: Apple Silicon only, a few versions (7.2.8, 7.2.17, 7.2.18 at the time
  of writing), and rekordbox must be re-signed. See rkbx_link's `MACOS_SETUP.md`.
  **Not possible on an Intel Mac.**

In rkbx_link's `config` file:

```
keeper.decks 2                    # or 4; must not exceed the decks rekordbox shows
osc.enabled true
osc.destination 127.0.0.1:4460    # the default; where Face2Face DJ listens
osc.msg.n/time true               # off by default: each deck's position
osc.msg.master/time true          # off by default: used to tell which deck is master
```

Then start rekordbox, rkbx_link, and choose **rekordbox** in Face2Face DJ.

Limits, because rkbx_link does not send these:

- **Track length and key** are unknown: the Viewer shows no remaining time and no key.
  (Reading them from the rekordbox library is planned with the track resolver.)
- **Playing** is inferred: a deck plays while its position advances.
- **On air** equals playing. **Master** is the deck whose position matches the master's.

## Pro DJ Link (CDJ and XDJ players)

`crates/face2face-core/src/sources/prolink/`

For players connected by network cable, with or without a DJM mixer. This is
the method of AlphaTheta's PRO DJ LINK Bridge and of
[autoblink](https://github.com/strikeout/autoblink): announce ourselves as one
more player (number 5, named "Face2Face DJ"), and the real players send us
their status. Track names are asked from the players' own database server.
Packet formats are from Deep Symmetry's
[DJ Link Ecosystem Analysis](https://djl-analysis.deepsymmetry.org).

Set-up: connect the laptop to the same switch as the players (or straight to
one), choose **Pro DJ Link**. Players 1 to 4 become decks 1 to 4. The status
shows how many players were found.

It listens on UDP 50000 to 50002 on all interfaces (it has to: the players
broadcast) and shares those ports, so rekordbox or PRO DJ LINK Bridge can run on
the same laptop.

Limits:

- **Position** is exact on players that send it (CDJ-3000 and later). Older
  players only send a beat number, and the position is estimated as
  `(beat - 1) × 60 / track tempo`: right for a steady-tempo track whose first
  beat is at the very start, off by the intro's length otherwise.
- **On air** needs a DJM mixer on the network; without one players report "off air".
- **Track names** need the track to be analysed by rekordbox, and on players
  older than the CDJ-3000 a second player on the network (they only answer
  player numbers 1 to 4, so we borrow another player's number; with one old
  player alone the deck shows "Unknown track").
- We send keep-alives only, not the number-claim sequence real players perform
  at start-up. Players 5 and 6 of a six-player CDJ-3000 set-up are not shown,
  and number 5 clashes with a real player 5.
- **rekordbox on a laptop is not expected to appear here.** As far as is
  documented, rekordbox in Performance mode does not send its decks' status on
  Pro DJ Link. This was not confirmed either way here; use the rekordbox
  source above.

## Mixxx

`crates/face2face-core/src/sources/mixxx.rs`, `tools/mixxx-mapping/`

Mixxx has no network interface, and its controller scripts can neither open
connections nor write files. They can send MIDI: the mapping's script sends each
deck's state 20 times a second as a system-exclusive message, and Face2Face DJ
is the MIDI port it sends to. The script only reads Mixxx controls.

Set-up, once.

**Windows:** close Mixxx, then double-click `Setup Mixxx (Windows).cmd` in
`tools/mixxx-mapping` (it is also in the `Mixxx set-up` folder of the Windows
download). It copies the mapping, switches the controller on in Mixxx's
settings, and takes you through creating the MIDI port. Windows cannot create
a MIDI port by itself, so the port comes from
[loopMIDI](https://www.tobias-erichsen.de/software/loopmidi.html), which the
script offers to install. Adding the port named `Face2Face DJ` in loopMIDI is
the one step done by hand (loopMIDI has no command for it); tick loopMIDI's
autostart so the port survives a restart of the PC.

**macOS:**

1. Copy `Face2Face-DJ.midi.xml` and `Face2Face-DJ.js` from `tools/mixxx-mapping`
   into Mixxx's user controller folder (Preferences > Controllers > "Open User Mapping Folder").
2. Turn the IAC bus on, if it is not: open the app Audio MIDI Setup, Window >
   Show MIDI Studio, double-click **IAC Driver**, tick "Device is online".
3. In Mixxx: Preferences > Controllers > **IAC Driver Bus 1** > tick Enabled,
   Load Mapping "Face2Face DJ", Apply. Remembered from then on.

Face2Face DJ listens on every MIDI input of the Mac and acts only on its own
messages, so it finds the IAC bus by itself. The IAC bus is always there, so
it does not matter which of Mixxx and Face2Face DJ starts first, or whether
one of them is restarted.

Without the IAC bus it still works: Face2Face DJ also creates a port of its
own, "Face2Face DJ", which shows up in Mixxx's controller list the same way.
That port is new each time Face2Face DJ starts, and Mixxx only looks for ports
when it starts, so Mixxx then has to be restarted whenever Face2Face DJ was.

**Linux:** as macOS without the IAC bus: enable the mapping on the
"Face2Face DJ" port, and start Face2Face DJ before Mixxx.

**If titles or waveforms are missing**, the status at the top left of the
sending laptop's window says why: Mixxx's library was not found, could not be
read, or does not contain the track on that deck.

Limits:

- **Title, artist and waveform come from Mixxx's library**, not from the
  script (scripts cannot read text). The track is found in `mixxxdb.sqlite` by
  its length, and by tempo if two tracks are equally long; the waveform is the
  one Mixxx analysed. Read-only. A deck shows "Mixxx deck 1" until the track is
  found, and stays that way for a track that is not in the library or that
  shares length and tempo with another. The waveform appears once Mixxx has
  finished analysing a newly loaded track.
- **Master** is Mixxx's sync leader, so there is none unless sync is in use.
- **On air**: channel fader up and the crossfader not fully on the other side.
