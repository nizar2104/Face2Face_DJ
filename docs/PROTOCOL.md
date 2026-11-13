# Face2Face DJ protocol, version 1

Status: Milestone 0, plus the deck readers (docs/SOURCES.md). Implemented by `crates/face2face-core/src/protocol/` (Rust),
`app/src/protocol/messages.ts` (TypeScript) and `tools/mock-sender/`. Both
implementations are tested against the fixtures in `test-assets/protocol/`.
Change this document, the fixtures and both implementations together.

## 1. Transport

- WebSocket over TCP. The Sender listens (default port **47801**), the Viewer connects.
- `TCP_NODELAY` is set on the Sender's sockets.
- **Text frames** carry one JSON object each. **Binary frames** carry waveform chunks (section 4).
- The Sender listens on all of the computer's network interfaces when it has a
  pairing code (section 3.1), and on `127.0.0.1` only when it has none (the mock
  sender, tests, the `sender` example).
- **Two-way** ("Link up") is two such connections, one in each direction: each
  laptop runs a Sender and connects a Viewer to the partner's Sender. Nothing in
  the messages changes, and either direction can drop and reconnect without the other.
- Two laptops both listen on 47801. Two linked windows on one computer: the first
  listens on 47801, the second on 47802, and each views the other.

## 2. Rules for every JSON message

- Every message has a string field `type`.
- The protocol version travels once per connection, in `hello.protocol`. It is `1`.
- Numbers are JSON numbers. Times and positions are seconds. An integer where a float is expected is valid (`124` = `124.0`).
- Deck numbers are integers 1 to 4.
- A receiver ignores messages that are malformed or have a `type` it does not know. It never closes the connection or fails because of them.
- Unknown extra fields are ignored.

## 3. Messages

### 3.1 Connection

`hello` (both directions). The Viewer sends it first; the Sender answers with its own.

```json
{"type":"hello","protocol":1,"device_id":"0b5c6f0e-...","name":"DJ A MacBook","role":"sender","app_version":"0.1.0","os":"macos"}
```

`role` is `sender` or `viewer`. `os` is `macos`, `windows` or another lower-case name.

**Pairing.** A Sender reachable from the network has a pairing code (6 digits in
the app). The Viewer sends it as a string field `pair_code` in its `hello`; the
field is absent otherwise, and a Sender without a code ignores it. A missing or
wrong code is answered with `error` code `pairing_failed`, and the Sender closes.
The Viewer then shows the message and does not reconnect until the user changes
the code. To keep guessing slow, the Sender looks at one `hello` a second from
each address (later ones wait their turn, and an address with more than 10 s of
waiting queued is refused at once); a correct code ends the address's waiting.
The code is sent in the clear and is not encryption.
If the Viewer's `protocol` differs from the Sender's, the Sender replies with
`error` code `protocol_mismatch` and closes. If the Sender's differs, the Viewer
shows the mismatch and retries later.

`heartbeat` (Sender to Viewer, once a second, also when nothing plays):

```json
{"type":"heartbeat","ts":5001.5}
```

A Viewer that receives nothing at all for 3 seconds treats the link as lost,
shows "Disconnected - showing last known" and reconnects with backoff from 0.5 s to 5 s.

`error` (Sender to Viewer):

```json
{"type":"error","code":"rkbx_link_not_running","message":"The Rekordbox deck reader is not running."}
```

Codes so far: `protocol_mismatch`, `pairing_failed`. `message` is text for the user.

### 3.2 Clock sync

```json
{"type":"ping","t_viewer":1000.123}
{"type":"pong","t_viewer":1000.123,"t_sender":5000.456}
```

- `ping` is Viewer to Sender; `t_viewer` is the Viewer's monotonic clock. The Sender answers at once, echoing `t_viewer` and adding its own monotonic clock. A ping is answered even before `hello`.
- The Viewer computes `rtt = t_receive - t_viewer` and `offset = t_sender - (t_viewer + rtt / 2)`, keeps the last 20 samples and uses the one with the smallest `rtt`. Then `sender_time = viewer_time + offset`.
- The Viewer pings 5 times, 150 ms apart, after connecting, then every 2 s.
- `deck_state.ts`, `pong.t_sender` and `heartbeat.ts` are readings of the same Sender clock. Its zero point is arbitrary and changes when the Sender restarts.

### 3.3 Source and decks (Sender to Viewer)

```json
{"type":"source_status","software":"rekordbox","software_version":"7.2.18","connected":true,"decks":4,"reader":"rkbx_link","reader_status":"running"}
```

`decks` (1 to 4) is how many decks the DJ software has; the Viewer lays itself out from it.
`software` / `reader` pairs so far: `mock` / `mock`, `traktor` / `traktor-api-client`,
`rekordbox` / `rkbx_link`, `pro dj link` / `virtual cdj`, `mixxx` / `midi mapping`.
`connected` is false while the reader waits for its DJ software; `reader_status`
is text for the user ("waiting for Traktor", "2 players", or why the reader failed).

```json
{"type":"deck_loaded","deck":1,
 "track":{"track_id":"mock-1a2b3c4d","title":"Neon Tide","artist":"Mara Voss","album":"Mock Sessions",
          "duration_s":372.4,"bpm_original":124.0,"key":{"camelot":"8A","musical":"Am"},
          "file_name":"Mara Voss - Neon Tide.flac","local":true,"resolve_status":"ok"},
 "waveform":{"waveform_id":"mockwf-1a2b3c4d","detail_fps":150,"frames":55860,"status":"ready"}}
```

- `key` is `null` when the key is unknown.
- `duration_s` is `0` when the source does not know the track's length (rekordbox through rkbx_link). The Viewer then shows no remaining time.
- `bpm_original` is `0` when the track has no known tempo; the Viewer then advances the position at normal speed.
- `track_id` is opaque. `mock-` plus 8 hex digits for mock tracks, `trk-` plus 8 hex digits for tracks from a reader.
- `resolve_status`: `ok`, `ambiguous`, `file_not_found`, `not_local`.
- `waveform` is `null` when there will be none: every track from a real reader until Milestone 2. `status`: `ready`, `analysing`, `unavailable`.
- `waveform_id`: 1 to 64 printable ASCII characters, no spaces. It identifies waveform data in binary frames. Milestone 0 uses `mockwf-` plus 8 hex digits; from Milestone 2 it is a hash of the audio file.
- `frames` is the number of detail frames; the detail data is `frames * 3` bytes.
- `deck_loaded` replaces whatever the deck held, and resets its state: the Viewer shows no position until the next `deck_state`.

```json
{"type":"deck_state","deck":1,"ts":123456.789,"pos_s":64.512,"bpm_current":126.0,"playing":true,"is_master":true,"on_air":true}
```

- Sent 20 to 60 times a second for each playing deck (the mock 60, the readers 20 plus one per report from the DJ software), and once on every change for a paused deck.
- `pos_s` is the track position at Sender time `ts`. It can jump in either direction (loops, hot cues, scratching).
- The Viewer ignores `deck_state` for a deck that has nothing loaded.
- Between messages the Viewer draws `pos_s + (now - ts) * bpm_current / bpm_original` while `playing`, and `pos_s` otherwise, with `ts` moved onto the Viewer's clock (section 3.2) when the state arrives. A new state more than 50 ms away from what is drawn snaps; a smaller difference is blended away over 100 ms.

```json
{"type":"deck_unloaded","deck":2}
{"type":"waveform_status","waveform_id":"mockwf-1a2b3c4d","status":"analysing","progress":0.4}
```

`progress` is 0 to 1. `waveform_status` updates the `status` of every deck showing that waveform.

### 3.4 Order on connect

1. Viewer: `hello`.
2. Sender: `hello`.
3. Sender: `source_status`, if a source has reported one.
4. For each loaded deck: `deck_loaded`, its waveform chunks (overview, then detail), then its latest `deck_state`.
5. Live messages from then on.

On receiving the Sender's `hello` the Viewer discards all deck state from any
earlier connection; step 4 rebuilds it. Its waveform cache is kept.

### 3.5 Reserved

`pair_request` and `pair_confirm` (brief, section 8). Not implemented; a peer
ignores them like any unknown type. Pairing is done with `hello.pair_code`
(section 3.1) for now.

## 4. Binary frame: `waveform_chunk`

All integers little-endian.

| Offset | Size | Field | Meaning |
|---|---|---|---|
| 0 | 4 | magic | ASCII `TDWF` |
| 4 | 1 | version | `1` |
| 5 | 1 | kind | `0` = overview, `1` = detail |
| 6 | 1 | id_len | `N`, 1 to 64 |
| 7 | 1 | reserved | `0` |
| 8 | 4 | offset | byte offset of this payload within the kind's data |
| 12 | 4 | length | payload bytes in this frame |
| 16 | 4 | total | total bytes of the kind's data (at most 16 MiB) |
| 20 | N | waveform_id | printable ASCII |
| 20+N | length | payload | |

- A frame is invalid, and ignored, if the magic, version or kind is wrong, `offset + length > total`, or the frame length is not exactly `20 + N + length`.
- Chunks of one kind are sent in order, starting at offset 0, at most 16 KiB of payload each. The overview is sent before the detail so the Viewer can draw the whole track at once.
- The Viewer may draw a partly received waveform.

### 4.1 Waveform data

- **Detail**: 150 frames per second of audio, 3 bytes per frame: low, mid, high band level, 0 to 255. Frame 0 is track position 0.
- **Overview**: 1,200 points for the whole track, 3 bytes per point, same bands. Point `j` is the per-band maximum of the detail frames it covers.
- Levels are normalised so the loudest band value in the track is 255.

The on-disk cache file format (brief 7.5) will be added here in Milestone 2.

## 5. Not part of the protocol: mock sessions

`test-assets/sessions/*.session` files drive the mock source. One step per line,
`<seconds> <command>`, `#` starts a comment, `restart` loops. Commands:
`load <deck> [track]`, `play <deck>`, `pause <deck>`, `pitch <deck> <percent>`,
`jump <deck> <beats>`, `eject <deck>`, `master <deck>`. The same commands can be
typed into `npm run mock-sender` and are what the Sender window's buttons send.
