// Face2Face DJ mapping for Mixxx 2.4 and later.
//
// Mixxx scripts cannot open network connections, but they can send MIDI. This
// one sends the state of each deck, 20 times a second, as a MIDI
// system-exclusive message to the "Face2Face DJ" MIDI port, where the
// Face2Face DJ app reads it. It only reads Mixxx controls; it sets none.
//
// Message, 25 bytes (decoded in crates/face2face-core/src/sources/mixxx.rs,
// both sides tested against test-assets/mixxx/deck_state.hex):
//
//   F0 7D 46 32 01   start, the MIDI id for non-commercial use, "F2", version 1
//   deck             1 to 4
//   decks            how many decks Mixxx has
//   load             counts up (0 to 127, then 0) each time the deck's track changes
//   flags            bit 0 loaded, 1 playing, 2 sync leader, 3 audible in the mix
//   position         milliseconds, 4 bytes
//   duration         milliseconds, 4 bytes
//   tempo            BPM as played x 100, 3 bytes
//   track tempo      BPM of the file x 100, 3 bytes
//   key              Mixxx's number for the file's key: 1 to 12 C to B major,
//                    13 to 24 C to B minor, 0 unknown
//   F7               end
//
// MIDI data bytes hold 7 bits, so numbers are split into 7-bit groups, most
// significant first.

var Face2Face = {};

Face2Face.INTERVAL_MS = 50;
Face2Face.DECKS = 4;

Face2Face.init = function() {
    Face2Face.loads = [];
    Face2Face.connections = [];
    for (let deck = 1; deck <= Face2Face.DECKS; deck++) {
        Face2Face.loads.push(0);
        const changed = function() {
            Face2Face.loads[deck - 1] = (Face2Face.loads[deck - 1] + 1) & 0x7f;
        };
        // "duration" also changes when a track replaces another without the
        // deck being empty in between.
        for (const control of ["track_loaded", "duration"]) {
            const connection = engine.makeConnection("[Channel" + deck + "]", control, changed);
            if (connection) {
                Face2Face.connections.push(connection);
            }
        }
    }
    Face2Face.timer = engine.beginTimer(Face2Face.INTERVAL_MS, Face2Face.send);
};

Face2Face.shutdown = function() {
    engine.stopTimer(Face2Face.timer);
    for (const connection of Face2Face.connections) {
        connection.disconnect();
    }
};

// `value` as `bytes` 7-bit groups, most significant first. Out-of-range and
// non-numeric values become the nearest limit.
Face2Face.split = function(value, bytes) {
    const max = Math.pow(2, 7 * bytes) - 1;
    let rest = Math.round(Math.min(Math.max(Number(value) || 0, 0), max));
    const groups = [];
    for (let i = 0; i < bytes; i++) {
        groups.unshift(rest % 128);
        rest = Math.floor(rest / 128);
    }
    return groups;
};

Face2Face.message = function(deck) {
    const group = "[Channel" + deck + "]";
    const durationMs = engine.getValue(group, "duration") * 1000;
    const loaded = engine.getValue(group, "track_loaded") > 0;

    // Audible: the channel fader is up and the crossfader is not fully on the other side.
    const crossfader = engine.getValue("[Master]", "crossfader"); // -1 left to 1 right
    const side = engine.getValue(group, "orientation"); // 0 left, 1 centre, 2 right
    const cutByCrossfader = (side === 0 && crossfader >= 1) || (side === 2 && crossfader <= -1);
    const onAir = engine.getValue(group, "volume") > 0 && !cutByCrossfader;

    const flags = (loaded ? 1 : 0)
        | (engine.getValue(group, "play") > 0 ? 2 : 0)
        | (engine.getValue(group, "sync_leader") > 0 ? 4 : 0)
        | (onAir ? 8 : 0);

    return [0xF0, 0x7D, 0x46, 0x32, 0x01,
        deck,
        Face2Face.split(engine.getValue("[App]", "num_decks"), 1)[0],
        Face2Face.loads[deck - 1],
        flags]
        .concat(Face2Face.split(engine.getValue(group, "playposition") * durationMs, 4))
        .concat(Face2Face.split(durationMs, 4))
        .concat(Face2Face.split(engine.getValue(group, "bpm") * 100, 3))
        .concat(Face2Face.split(engine.getValue(group, "file_bpm") * 100, 3))
        .concat(Face2Face.split(engine.getValue(group, "file_key"), 1))
        .concat([0xF7]);
};

Face2Face.send = function() {
    for (let deck = 1; deck <= Face2Face.DECKS; deck++) {
        const message = Face2Face.message(deck);
        midi.sendSysexMsg(message, message.length);
    }
};
