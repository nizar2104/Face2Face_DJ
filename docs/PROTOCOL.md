# Face2Face DJ protocol, version 1

Status: Milestone 0. Implemented by `crates/face2face-core/src/protocol/` (Rust),
`app/src/protocol/messages.ts` (TypeScript) and `tools/mock-sender/`. Both
implementations are tested against the fixtures in `test-assets/protocol/`.
Change this document, the fixtures and both implementations together.

## 1. Transport

- WebSocket over TCP. The Sender listens (default port **47801**), the Viewer connects.
- `TCP_NODELAY` is set on the Sender's sockets.
- **Text frames** carry one JSON object each. **Binary frames** carry waveform chunks (section 4).
- Milestone 0: the Sender listens on `127.0.0.1` only, because pairing does not exist yet.

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

Codes so far: `protocol_mismatch`. `message` is text for the user.

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
The mock source reports `software: "mock"`, `reader: "mock"`.

```json
{"type":"deck_loaded","deck":1,
 "track":{"track_id":"mock-1a2b3c4d","title":"Neon Tide","artist":"Mara Voss","album":"Mock Sessions",
          "duration_s":372.4,"bpm_original":124.0,"key":{"camelot":"8A","musical":"Am"},
          "file_name":"Mara Voss - Neon Tide.flac","local":true,"resolve_status":"ok"},
 "waveform":{"waveform_id":"mockwf-1a2b3c4d","detail_fps":150,"frames":55860,"status":"ready"}}
```

- `key` is `null` when the key is unknown.
- `resolve_status`: `ok`, `ambiguous`, `file_not_found`, `not_local`.
- `waveform` is `null` when there will be none. `status`: `ready`, `analysing`, `unavailable`.
- `waveform_id`: 1 to 64 printable ASCII characters, no spaces. It identifies waveform data in binary frames. Milestone 0 uses `mockwf-` plus 8 hex digits; from Milestone 2 it is a hash of the audio file.
- `frames` is the number of detail frames; the detail data is `frames * 3` bytes.
- `deck_loaded` replaces whatever the deck held, and resets its state: the Viewer shows no position until the next `deck_state`.

```json
{"type":"deck_state","deck":1,"ts":123456.789,"pos_s":64.512,"bpm_current":126.0,"playing":true,"is_master":true,"on_air":true}
```

- Sent about 60 times a second for each playing deck, and once on every change for a paused deck.
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

### 3.5 Reserved for Milestone 1

`pair_request` and `pair_confirm` (brief, section 8). Not implemented; a
Milestone 0 peer ignores them like any unknown type.

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
