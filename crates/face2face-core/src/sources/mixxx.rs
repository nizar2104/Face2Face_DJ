//! Mixxx, through a controller mapping that sends MIDI.
//!
//! Mixxx has no network interface, and its controller scripts cannot open
//! connections or files. They can send MIDI. `tools/mixxx-mapping` is a
//! mapping whose script sends each deck's state 20 times a second as a
//! system-exclusive message, and this reader is the MIDI port it sends to.
//! The message layout is documented in the script, `Face2Face-DJ.js`.
//!
//! The port: an existing MIDI input whose name contains "Face2Face" is used
//! if there is one (a loopMIDI port on Windows, an IAC bus on macOS).
//! Otherwise, on macOS and Linux, we create a virtual port named
//! "Face2Face DJ". Mixxx lists MIDI devices when it starts, so the port must
//! exist before Mixxx is opened (docs/SOURCES.md).
//!
//! What Mixxx scripts cannot read: which track is on a deck. That, with the
//! title, artist and Mixxx's own waveform, is looked up in Mixxx's library by
//! the track's length ([`super::mixxx_library`]). Until the look-up succeeds
//! (or if the library cannot be read) a deck is shown as "Mixxx deck 1" and so
//! on, with tempo, key, position and length.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutputConnection};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use super::board::{DeckBoard, DeckUpdate, TICK};
use super::key::key_from_pitch_class;
use super::mixxx_library::{self, LibraryTrack};
use super::{DeckSource, SourceEvent, SourceStatus, TrackMeta, RETRY_OPEN};
use crate::protocol::MAX_DECKS;

/// The name Mixxx shows for our MIDI port, and what we look for in existing ports.
pub const PORT_NAME: &str = "Face2Face DJ";
const PORT_NAME_MATCH: &str = "face2face";

/// Start of every message: SysEx, the id for non-commercial use, "F2", version 1.
const HEADER: [u8; 5] = [0xf0, 0x7d, 0x46, 0x32, 0x01];
const MESSAGE_LENGTH: usize = 25;
const DISCONNECTED: Duration = Duration::from_secs(3);
/// Mixxx analyses a track when it is first loaded, and only then has its
/// waveform. So a look-up that finds none is repeated for about a minute.
const LOOKUP_EVERY: Duration = Duration::from_secs(3);
const LOOKUP_ATTEMPTS: u8 = 20;

pub struct MixxxSource {
    /// The folder with Mixxx's library, for titles and waveforms. `None`: there is none to read.
    pub library_dir: Option<PathBuf>,
}

impl Default for MixxxSource {
    fn default() -> Self {
        Self { library_dir: mixxx_library::default_dir() }
    }
}

/// A question for Mixxx's library, to be asked off the main loop.
#[derive(Debug, Clone, PartialEq)]
struct Lookup {
    deck: u8,
    /// The load the answer is for; a later load makes it stale.
    load: u8,
    duration_s: f64,
    bpm: f64,
}

/// How far the look-up for the track on a deck has got.
#[derive(Debug, Clone, Copy, Default)]
struct LookupProgress {
    attempts: u8,
    next: Option<Instant>,
    /// The waveform has arrived; nothing more to ask.
    done: bool,
}

/// One deck as the mapping script reports it.
#[derive(Debug, Clone, PartialEq)]
struct DeckReport {
    deck: u8,
    decks: u8,
    /// Changes whenever the deck's track does.
    load: u8,
    loaded: bool,
    playing: bool,
    sync_leader: bool,
    on_air: bool,
    pos_s: f64,
    duration_s: f64,
    bpm_current: f64,
    bpm_track: f64,
    /// 1 to 12: C to B major. 13 to 24: C to B minor. 0: unknown.
    key: u8,
}

/// A number sent as 7-bit groups, most significant first.
fn joined(groups: &[u8]) -> f64 {
    groups.iter().fold(0u32, |value, &group| (value << 7) | u32::from(group & 0x7f)).into()
}

fn decode(message: &[u8]) -> Option<DeckReport> {
    if message.len() != MESSAGE_LENGTH || !message.starts_with(&HEADER) || message[24] != 0xf7 {
        return None;
    }
    let (deck, flags) = (message[5], message[8]);
    if !(1..=MAX_DECKS).contains(&deck) || message[5..24].iter().any(|&byte| byte > 0x7f) {
        return None;
    }
    Some(DeckReport {
        deck,
        decks: message[6].clamp(1, MAX_DECKS),
        load: message[7],
        loaded: flags & 1 != 0,
        playing: flags & 2 != 0,
        sync_leader: flags & 4 != 0,
        on_air: flags & 8 != 0,
        pos_s: joined(&message[9..13]) / 1000.0,
        duration_s: joined(&message[13..17]) / 1000.0,
        bpm_current: joined(&message[17..20]) / 100.0,
        bpm_track: joined(&message[20..23]) / 100.0,
        key: message[23],
    })
}

/// The reports in a run of MIDI bytes, which may hold several messages or
/// other MIDI traffic.
fn reports(bytes: &[u8]) -> impl Iterator<Item = DeckReport> + '_ {
    bytes
        .iter()
        .enumerate()
        .filter(|(_, &byte)| byte == 0xf0)
        .filter_map(move |(start, _)| decode(bytes.get(start..start + MESSAGE_LENGTH)?))
}

fn status(connected: bool, decks: u8, reader_status: String) -> SourceStatus {
    SourceStatus {
        software: "mixxx".to_string(),
        software_version: String::new(),
        connected,
        decks,
        reader: "midi mapping".to_string(),
        reader_status,
    }
}

/// The reader's logic, separate from the MIDI port so tests can feed it bytes.
struct Reader {
    board: DeckBoard,
    loads: [Option<u8>; MAX_DECKS as usize],
    lookups: [LookupProgress; MAX_DECKS as usize],
    last_message: Option<Instant>,
    /// Whether Mixxx is sending, and how many decks it has.
    reported: (bool, u8),
}

impl Reader {
    fn new(events: mpsc::UnboundedSender<SourceEvent>) -> Self {
        let reader = Self {
            board: DeckBoard::new(events),
            loads: [None; MAX_DECKS as usize],
            lookups: [LookupProgress::default(); MAX_DECKS as usize],
            last_message: None,
            reported: (false, 2),
        };
        reader.board.status(status(false, 2, "waiting for Mixxx".to_string()));
        reader
    }

    fn report(&mut self, connected: bool, decks: u8) {
        if self.reported != (connected, decks) {
            self.reported = (connected, decks);
            let text = if connected { "running" } else { "waiting for Mixxx" };
            self.board.status(status(connected, decks, text.to_string()));
        }
    }

    /// One report from the script. Returns a library look-up that is now due.
    fn handle(&mut self, report: &DeckReport, now: Instant) -> Option<Lookup> {
        self.last_message = Some(now);
        self.report(true, report.decks);
        let deck = report.deck;
        let index = usize::from(deck - 1);
        if !report.loaded {
            self.loads[index] = None;
            self.board.unload(deck);
            return None;
        }
        // Mixxx fills in the tempo and key when it has analysed a new track.
        let bpm_original = if report.bpm_track > 0.0 { report.bpm_track } else { report.bpm_current };
        let key = match report.key {
            1..=12 => Some(key_from_pitch_class(report.key - 1, false)),
            13..=24 => Some(key_from_pitch_class(report.key - 13, true)),
            _ => None,
        };
        if self.loads[index] != Some(report.load) {
            self.loads[index] = Some(report.load);
            self.lookups[index] = LookupProgress::default();
            let meta = TrackMeta {
                title: format!("Mixxx deck {deck}"),
                artist: String::new(),
                album: String::new(),
                duration_s: report.duration_s,
                bpm_original,
                key,
                file_name: String::new(),
            };
            self.board.load(deck, meta, None);
        } else {
            self.board.revise(deck, |meta| {
                meta.duration_s = report.duration_s;
                meta.bpm_original = bpm_original;
                meta.key = key;
            });
        }
        let update = DeckUpdate {
            pos_s: Some(report.pos_s),
            bpm_current: Some(report.bpm_current),
            playing: Some(report.playing),
            is_master: Some(report.sync_leader),
            on_air: Some(report.on_air),
        };
        self.board.update_at(deck, now, update);

        let progress = &mut self.lookups[index];
        let due = !progress.done && progress.attempts < LOOKUP_ATTEMPTS && progress.next.is_none_or(|next| now >= next);
        if !due || report.duration_s <= 0.0 {
            return None;
        }
        progress.attempts += 1;
        progress.next = Some(now + LOOKUP_EVERY);
        Some(Lookup { deck, load: report.load, duration_s: report.duration_s, bpm: report.bpm_track })
    }

    /// What the library knows about the track a [`Lookup`] asked for.
    fn found(&mut self, lookup: &Lookup, track: LibraryTrack) {
        let index = usize::from(lookup.deck - 1);
        if self.loads[index] != Some(lookup.load) {
            return; // another track has been loaded since
        }
        let Some(current) = self.board.meta(lookup.deck) else { return };
        let file_name = track.file_path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let meta = TrackMeta { title: track.title, artist: track.artist, album: track.album, file_name, ..current.clone() };
        self.lookups[index].done = track.waveform.is_some();
        // Asked again while waiting for the analysis: say it once only.
        if meta == *current && track.waveform.is_none() {
            return;
        }
        // The next report from the script, 50 ms away, restores the deck's state.
        self.board.load_with(lookup.deck, meta, Some(track.file_path), track.waveform);
    }

    fn tick(&mut self, now: Instant) {
        if self.reported.0 && self.last_message.is_some_and(|at| now.saturating_duration_since(at) > DISCONNECTED) {
            // Mixxx was closed or the mapping switched off: its decks are gone.
            for deck in 1..=MAX_DECKS {
                self.loads[usize::from(deck - 1)] = None;
                self.board.unload(deck);
            }
            self.report(false, self.reported.1);
        }
        self.board.tick();
    }
}

/// The open MIDI port. Dropping it closes the port.
struct Port {
    name: String,
    _receiving: MidiInputConnection<()>,
    /// The other half of a virtual port, see [`open_port`]. Nothing is sent on it.
    _sending: Option<MidiOutputConnection>,
}

/// Opens the MIDI port and forwards everything received on it.
fn open_port(received: mpsc::UnboundedSender<Vec<u8>>) -> Result<Port, String> {
    let mut input = MidiInput::new(PORT_NAME).map_err(|error| format!("MIDI is not available: {error}"))?;
    input.ignore(Ignore::None); // system-exclusive messages are ignored by default
    let forward = move |_time: u64, bytes: &[u8], _: &mut ()| {
        let _ = received.send(bytes.to_vec());
    };
    let existing = input.ports().into_iter().find_map(|port| {
        let name = input.port_name(&port).ok()?;
        name.to_lowercase().contains(PORT_NAME_MATCH).then_some((port, name))
    });
    if let Some((port, name)) = existing {
        let connection = input.connect(&port, "from Mixxx", forward, ()).map_err(|error| format!("cannot open MIDI port {name}: {error}"))?;
        return Ok(Port { name, _receiving: connection, _sending: None });
    }
    #[cfg(unix)]
    {
        use midir::os::unix::{VirtualInput, VirtualOutput};
        let connection = input.create_virtual(PORT_NAME, forward, ()).map_err(|error| format!("cannot create a MIDI port: {error}"))?;
        // Mixxx lists a MIDI device by its input and does not offer one that
        // can only be sent to. So the port gets a second half of the same
        // name for Mixxx to pair it with, although we never send on it.
        let sending = midir::MidiOutput::new(PORT_NAME)
            .ok()
            .and_then(|output| output.create_virtual(PORT_NAME).ok());
        if sending.is_none() {
            tracing::warn!("Mixxx: could not create the sending half of the MIDI port; Mixxx may not list it");
        }
        Ok(Port { name: PORT_NAME.to_string(), _receiving: connection, _sending: sending })
    }
    #[cfg(not(unix))]
    {
        let _ = forward;
        Err(format!("no MIDI port named \"{PORT_NAME}\": create one with loopMIDI, then start again"))
    }
}

impl DeckSource for MixxxSource {
    fn name(&self) -> &'static str {
        "mixxx"
    }

    fn spawn(self: Box<Self>, events: mpsc::UnboundedSender<SourceEvent>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut reader = Reader::new(events);
            // The port lives on its own thread: MIDI connections are tied to
            // the thread that made them on some systems. It closes when
            // `close` is dropped, which happens when this task ends.
            // A port that cannot be opened (on Windows it has to be made with
            // loopMIDI first) is reported once and tried again.
            let mut reported = false;
            let (mut received, _close) = loop {
                let (received_tx, received) = mpsc::unbounded_channel::<Vec<u8>>();
                let (close, closed) = std::sync::mpsc::channel::<()>();
                let (opened_tx, opened) = tokio::sync::oneshot::channel();
                std::thread::spawn(move || {
                    let port = open_port(received_tx);
                    let _ = opened_tx.send(port.as_ref().map(|port| port.name.clone()).map_err(String::clone));
                    if let Ok(_connection) = port {
                        let _ = closed.recv();
                    }
                });
                let error = match opened.await {
                    Ok(Ok(name)) => {
                        tracing::info!("Mixxx: listening on MIDI port {name}");
                        break (received, close);
                    }
                    Ok(Err(error)) => error,
                    Err(_) => "the MIDI thread stopped".to_string(),
                };
                if !reported {
                    reader.board.status(status(false, 2, error));
                    reported = true;
                }
                tokio::time::sleep(RETRY_OPEN).await;
            };
            if reported {
                reader.board.status(status(false, 2, "waiting for Mixxx".to_string()));
            }
            let (found_tx, mut found_rx) = mpsc::unbounded_channel::<(Lookup, LibraryTrack)>();
            let mut ticks = tokio::time::interval(TICK);
            ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    Some(bytes) = received.recv() => {
                        let now = Instant::now();
                        for report in reports(&bytes) {
                            let (Some(lookup), Some(dir)) = (reader.handle(&report, now), self.library_dir.clone()) else { continue };
                            let found = found_tx.clone();
                            // Reading the database and a waveform of a few megabytes blocks.
                            tokio::task::spawn_blocking(move || {
                                match mixxx_library::find_track(&dir, lookup.duration_s, lookup.bpm) {
                                    Ok(Some(track)) => {
                                        let _ = found.send((lookup, track));
                                    }
                                    Ok(None) => {}
                                    Err(error) => tracing::warn!("Mixxx: cannot read the library: {error:#}"),
                                }
                            });
                        }
                    }
                    Some((lookup, track)) = found_rx.recv() => reader.found(&lookup, track),
                    _ = ticks.tick() => reader.tick(Instant::now()),
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::board::testing::{drain, last_state};
    use super::*;

    /// The messages in the fixture shared with the mapping script's own test.
    fn fixture() -> Vec<Vec<u8>> {
        include_str!("../../../../test-assets/mixxx/deck_state.hex")
            .lines()
            .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
            .map(|line| line.split_whitespace().map(|byte| u8::from_str_radix(byte, 16).expect("hex")).collect())
            .collect()
    }

    #[test]
    fn the_scripts_message_decodes_to_the_deck_it_describes() {
        let messages = fixture();
        let deck_2 = decode(&messages[0]).expect("valid");
        let expected = DeckReport {
            deck: 2,
            decks: 4,
            load: 3,
            loaded: true,
            playing: true,
            sync_leader: true,
            on_air: true,
            pos_s: 93.1,
            duration_s: 372.4,
            bpm_current: 126.5,
            bpm_track: 124.0,
            key: 22,
        };
        assert_eq!(deck_2, expected);
        let deck_1 = decode(&messages[1]).expect("valid");
        assert_eq!((deck_1.deck, deck_1.loaded, deck_1.decks), (1, false, 4));
    }

    #[test]
    fn a_report_becomes_a_loaded_playing_deck() {
        let (tx, mut events) = mpsc::unbounded_channel();
        let mut reader = Reader::new(tx);
        let now = Instant::now();
        // Both messages in one run of bytes, with a stray MIDI note between them.
        let mut bytes = fixture()[1].clone();
        bytes.extend([0x90, 0x3c, 0x7f]);
        bytes.extend(&fixture()[0]);
        for report in reports(&bytes) {
            reader.handle(&report, now);
        }

        let sent = drain(&mut events);
        assert!(matches!(&sent[0], SourceEvent::Status(status) if !status.connected && status.software == "mixxx"));
        assert!(matches!(&sent[1], SourceEvent::Status(status) if status.connected && status.decks == 4));
        let meta = sent.iter().find_map(|event| match event {
            SourceEvent::TrackLoaded { deck: 2, meta, .. } => Some(meta.clone()),
            _ => None,
        });
        let meta = meta.expect("deck 2 is loaded");
        assert_eq!((meta.title.as_str(), meta.duration_s, meta.bpm_original), ("Mixxx deck 2", 372.4, 124.0));
        assert_eq!(meta.key.map(|key| key.camelot).as_deref(), Some("8A"));
        let state = last_state(&sent, 2).expect("a state");
        assert!(state.playing && state.is_master && state.on_air);
        assert_eq!(state.bpm_current, 126.5);
        assert!((state.pos_s - 93.1).abs() < 0.05);
        assert!(last_state(&sent, 1).is_none(), "deck 1 is empty");
    }

    #[test]
    fn the_load_counter_tells_a_new_track_from_a_late_analysis() {
        let (tx, mut events) = mpsc::unbounded_channel();
        let mut reader = Reader::new(tx);
        let now = Instant::now();
        let mut report = decode(&fixture()[0]).expect("valid");
        report.bpm_track = 0.0;
        report.key = 0;
        reader.handle(&report, now);
        let loads = |events: &[SourceEvent]| -> Vec<(f64, bool)> {
            events
                .iter()
                .filter_map(|event| match event {
                    SourceEvent::TrackLoaded { meta, .. } => Some((meta.bpm_original, meta.key.is_some())),
                    _ => None,
                })
                .collect()
        };
        // Not analysed yet: the tempo being played stands in for the track's.
        assert_eq!(loads(&drain(&mut events)), vec![(126.5, false)]);

        reader.handle(&report, now);
        assert!(loads(&drain(&mut events)).is_empty(), "the same report changes nothing");

        report.bpm_track = 124.0;
        report.key = 22;
        reader.handle(&report, now);
        assert_eq!(loads(&drain(&mut events)), vec![(124.0, true)], "the analysis corrects the track");

        report.load = 4;
        reader.handle(&report, now);
        assert_eq!(loads(&drain(&mut events)).len(), 1, "a new track");

        report.loaded = false;
        reader.handle(&report, now);
        assert!(drain(&mut events).contains(&SourceEvent::TrackUnloaded { deck: 2 }));
    }

    #[test]
    fn the_library_names_the_track_and_brings_its_waveform() {
        use super::super::mixxx_library::testing::{add_track, analysis_file, library};
        let (dir, database) = library();
        add_track(&database, 1, Some("Neon Tide"), "/Music/Mara Voss - Neon Tide.flac", 372.4, 124.0);
        database.execute("INSERT INTO track_analysis VALUES (8, 101, '1')", []).expect("inserts");

        let (tx, mut events) = mpsc::unbounded_channel();
        let mut reader = Reader::new(tx);
        let now = Instant::now();
        let report = decode(&fixture()[0]).expect("valid");
        let lookup = reader.handle(&report, now).expect("a new track is looked up");
        assert_eq!(lookup, Lookup { deck: 2, load: 3, duration_s: 372.4, bpm: 124.0 });
        assert_eq!(reader.handle(&report, now + Duration::from_millis(50)), None, "not asked again at once");

        // Mixxx is still analysing: the title comes first, the waveform later.
        let track = mixxx_library::find_track(&dir, lookup.duration_s, lookup.bpm).expect("reads").expect("found");
        reader.found(&lookup, track.clone());
        reader.found(&lookup, track);
        let loads = |events: &[SourceEvent]| -> Vec<(String, String, bool)> {
            events
                .iter()
                .filter_map(|event| match event {
                    SourceEvent::TrackLoaded { deck: 2, meta, waveform, .. } => Some((meta.title.clone(), meta.file_name.clone(), waveform.is_some())),
                    _ => None,
                })
                .collect()
        };
        let named = ("Neon Tide".to_string(), "Mara Voss - Neon Tide.flac".to_string());
        assert_eq!(
            loads(&drain(&mut events)),
            vec![("Mixxx deck 2".to_string(), String::new(), false), (named.0.clone(), named.1.clone(), false)],
            "the same answer twice is announced once"
        );

        let again = reader.handle(&report, now + LOOKUP_EVERY).expect("asked again while there is no waveform");
        let band = vec![9u8; 882];
        std::fs::write(dir.join("analysis/8"), analysis_file(441.0, &band, &band, &band)).expect("writes");
        let track = mixxx_library::find_track(&dir, again.duration_s, again.bpm).expect("reads").expect("found");
        reader.found(&again, track);
        let sent = drain(&mut events);
        assert_eq!(loads(&sent), vec![(named.0, named.1, true)]);
        // Tempo and key from the script survive the renaming.
        assert!(sent.iter().any(|event| matches!(event, SourceEvent::TrackLoaded { meta, .. } if meta.bpm_original == 124.0 && meta.key.is_some())));
        assert_eq!(reader.handle(&report, now + LOOKUP_EVERY * 5), None, "nothing left to ask");

        // An answer for a track that has since been replaced is dropped.
        let mut next = report.clone();
        next.load = 4;
        reader.handle(&next, now + LOOKUP_EVERY * 6);
        drain(&mut events);
        reader.found(&again, mixxx_library::find_track(&dir, 372.4, 124.0).expect("reads").expect("found"));
        assert!(loads(&drain(&mut events)).is_empty());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_library_is_not_asked_for_ever() {
        let (tx, _events) = mpsc::unbounded_channel();
        let mut reader = Reader::new(tx);
        let start = Instant::now();
        let report = decode(&fixture()[0]).expect("valid");
        let asked = (0..200u32).filter(|&second| reader.handle(&report, start + Duration::from_secs(second.into())).is_some()).count();
        assert_eq!(asked, usize::from(LOOKUP_ATTEMPTS));
    }

    #[test]
    fn silence_from_mixxx_empties_the_decks() {
        let (tx, mut events) = mpsc::unbounded_channel();
        let mut reader = Reader::new(tx);
        let now = Instant::now();
        reader.handle(&decode(&fixture()[0]).expect("valid"), now);
        reader.tick(now + DISCONNECTED + Duration::from_millis(1));
        let sent = drain(&mut events);
        assert!(sent.contains(&SourceEvent::TrackUnloaded { deck: 2 }));
        assert!(matches!(sent.last(), Some(SourceEvent::Status(status)) if !status.connected && status.decks == 4));
    }

    #[test]
    fn other_midi_traffic_is_ignored() {
        let good = fixture()[0].clone();
        assert!(decode(&good[..24]).is_none(), "cut short");
        for (index, value) in [(1, 0x7e), (4, 0x02), (5, 0), (5, 5), (10, 0x80), (24, 0x00)] {
            let mut bad = good.clone();
            bad[index] = value;
            assert!(decode(&bad).is_none(), "byte {index} = {value:#x}");
        }
        assert_eq!(reports(&[0xf0, 0x7e, 0x7f, 0x06, 0x01, 0xf7, 0xf8, 0xf0]).count(), 0);
    }
}
