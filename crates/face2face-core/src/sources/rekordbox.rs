//! Rekordbox in Performance mode, through rkbx_link.
//!
//! Rekordbox offers no way to read its decks, so this reader relies on
//! [rkbx_link](https://github.com/grufkork/rkbx_link), a separate program that
//! reads them from Rekordbox's memory and sends them as OSC over UDP. We
//! listen where rkbx_link sends by default. Its configuration needs
//! `osc.enabled true`, `osc.msg.n/time true` and `osc.msg.master/time true`
//! (docs/SOURCES.md).
//!
//! Messages used, `n` being the deck counted from 0:
//!
//! | Address | Argument |
//! |---|---|
//! | `/n/time` | float, track position in seconds |
//! | `/n/bpm/current`, `/n/bpm/original` | float |
//! | `/n/track/title`, `/artist`, `/album` | string; sent in this order when the track changes |
//! | `/master/time` | float, the master deck's position |
//!
//! What rkbx_link does not send, and this reader therefore makes up:
//! - **Track length and key**: unknown (length 0, no key).
//! - **Playing**: a deck plays while its position advances.
//! - **Master**: the deck whose position matches `/master/time`.
//! - **On air**: same as playing.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use super::board::{DeckBoard, DeckUpdate, TICK};
use super::osc::{decode, OscArg, OscMessage};
use super::{DeckSource, SourceEvent, SourceStatus, TrackMeta};
use crate::protocol::MAX_DECKS;

/// rkbx_link's default `osc.destination`.
pub const DEFAULT_OSC_PORT: u16 = 4460;

/// rkbx_link sends the position about 60 times a second while Rekordbox runs.
/// This long without one means the deck stopped or rkbx_link went away.
const STALE: Duration = Duration::from_millis(300);
const DISCONNECTED: Duration = Duration::from_secs(3);
/// How close a deck's position must be to `/master/time` to be the master.
const MASTER_TOLERANCE_S: f32 = 0.05;

pub struct RekordboxSource {
    pub listen: SocketAddr,
}

impl Default for RekordboxSource {
    fn default() -> Self {
        Self { listen: (Ipv4Addr::LOCALHOST, DEFAULT_OSC_PORT).into() }
    }
}

#[derive(Default)]
struct DeckInput {
    title: String,
    artist: String,
    bpm_original: f64,
    bpm_current: f64,
    /// The last position and when it arrived.
    time: Option<(f32, Instant)>,
    playing: bool,
}

/// The reader's logic, separate from the socket so tests can feed it messages.
struct Reader {
    board: DeckBoard,
    inputs: [DeckInput; MAX_DECKS as usize],
    last_message: Option<Instant>,
    connected: bool,
    reported_decks: u8,
}

impl Reader {
    fn new(events: mpsc::UnboundedSender<SourceEvent>) -> Self {
        let mut reader = Self {
            board: DeckBoard::new(events),
            inputs: Default::default(),
            last_message: None,
            connected: false,
            reported_decks: 0,
        };
        reader.report(false);
        reader
    }

    fn report(&mut self, connected: bool) {
        let decks = self.board.deck_span();
        if connected == self.connected && decks == self.reported_decks {
            return;
        }
        self.connected = connected;
        self.reported_decks = decks;
        self.board.status(SourceStatus {
            software: "rekordbox".to_string(),
            software_version: String::new(),
            connected,
            decks,
            reader: "rkbx_link".to_string(),
            reader_status: if connected { "running" } else { "waiting for rkbx_link" }.to_string(),
        });
    }

    fn handle(&mut self, message: &OscMessage, now: Instant) {
        self.last_message = Some(now);
        let mut parts = message.address.split('/').skip(1);
        let (Some(target), Some(first), second) = (parts.next(), parts.next(), parts.next()) else { return };

        if target == "master" {
            if let (("time", None), Some(OscArg::Float(time))) = ((first, second), message.args.first()) {
                // rkbx_link reads the master's position in the same pass as the
                // decks', so the deck at (nearly) that position is the master.
                let master = self
                    .inputs
                    .iter()
                    .enumerate()
                    .filter_map(|(index, input)| Some((index, (input.time?.0 - time).abs())))
                    .filter(|&(_, distance)| distance < MASTER_TOLERANCE_S)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(index, _)| index);
                if let Some(index) = master {
                    self.board.set_master(Some(index as u8 + 1));
                }
            }
            return;
        }
        let Some(index) = target.parse::<usize>().ok().filter(|&index| index < self.inputs.len()) else { return };
        let deck = index as u8 + 1;
        let input = &mut self.inputs[index];

        match ((first, second), message.args.first()) {
            (("time", None), Some(OscArg::Float(time))) => {
                let previous = input.time.replace((*time, now));
                // The first reading says nothing about movement.
                input.playing = previous.is_some_and(|(before, _)| *time != before);
                let update = DeckUpdate {
                    pos_s: Some(f64::from(*time)),
                    playing: Some(input.playing),
                    on_air: Some(input.playing),
                    ..DeckUpdate::default()
                };
                self.board.update_at(deck, now, update);
            }
            (("bpm", Some("current")), Some(OscArg::Float(bpm))) => {
                input.bpm_current = f64::from(*bpm);
                self.board.update_at(deck, now, DeckUpdate { bpm_current: Some(input.bpm_current), ..DeckUpdate::default() });
            }
            (("bpm", Some("original")), Some(OscArg::Float(bpm))) => {
                input.bpm_original = f64::from(*bpm);
                let bpm_original = input.bpm_original;
                self.board.revise(deck, |meta| meta.bpm_original = bpm_original);
            }
            (("track", Some("title")), Some(OscArg::Text(title))) => input.title = title.clone(),
            (("track", Some("artist")), Some(OscArg::Text(artist))) => input.artist = artist.clone(),
            // The album is the last of the three, so the track is complete now.
            (("track", Some("album")), Some(OscArg::Text(album))) => {
                if input.title.is_empty() && input.artist.is_empty() {
                    self.board.unload(deck);
                } else {
                    let meta = TrackMeta {
                        title: input.title.clone(),
                        artist: input.artist.clone(),
                        album: album.clone(),
                        duration_s: 0.0,
                        bpm_original: input.bpm_original,
                        key: None,
                        file_name: String::new(),
                    };
                    let bpm_current = (input.bpm_current > 0.0).then_some(input.bpm_current);
                    input.time = None;
                    input.playing = false;
                    self.board.load(deck, meta, None);
                    self.board.update_at(deck, now, DeckUpdate { pos_s: Some(0.0), bpm_current, ..DeckUpdate::default() });
                }
            }
            _ => {}
        }
        self.report(true);
    }

    /// Pauses decks whose position stopped arriving, and notices rkbx_link going away.
    fn tick(&mut self, now: Instant) {
        for (index, input) in self.inputs.iter_mut().enumerate() {
            let stale = input.time.is_some_and(|(_, at)| now.saturating_duration_since(at) > STALE);
            if input.playing && stale {
                input.playing = false;
                let pos_s = input.time.map(|(time, _)| f64::from(time));
                let update = DeckUpdate { pos_s, playing: Some(false), on_air: Some(false), ..DeckUpdate::default() };
                self.board.update_at(index as u8 + 1, now, update);
            }
        }
        if self.connected && self.last_message.is_some_and(|at| now.saturating_duration_since(at) > DISCONNECTED) {
            self.report(false);
        }
        self.board.tick();
    }
}

impl DeckSource for RekordboxSource {
    fn name(&self) -> &'static str {
        "rekordbox"
    }

    fn spawn(self: Box<Self>, events: mpsc::UnboundedSender<SourceEvent>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut reader = Reader::new(events);
            let socket = match UdpSocket::bind(self.listen).await {
                Ok(socket) => socket,
                Err(error) => {
                    reader.board.status(SourceStatus {
                        software: "rekordbox".to_string(),
                        software_version: String::new(),
                        connected: false,
                        decks: 2,
                        reader: "rkbx_link".to_string(),
                        reader_status: format!("cannot listen on {}: {error}", self.listen),
                    });
                    return;
                }
            };
            let mut ticks = tokio::time::interval(TICK);
            ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
            let mut buffer = [0u8; 4096];
            loop {
                tokio::select! {
                    received = socket.recv(&mut buffer) => {
                        let Ok(length) = received else { continue };
                        let now = Instant::now();
                        for message in decode(&buffer[..length]) {
                            reader.handle(&message, now);
                        }
                    }
                    _ = ticks.tick() => reader.tick(Instant::now()),
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::board::testing::{drain, last_state};
    use super::super::osc::testing::{float, text};
    use super::*;

    fn feed(reader: &mut Reader, packet: Vec<u8>, now: Instant) {
        for message in decode(&packet) {
            reader.handle(&message, now);
        }
    }

    fn load(reader: &mut Reader, deck: &str, title: &str, now: Instant) {
        feed(reader, float(&format!("/{deck}/bpm/original"), 124.0), now);
        feed(reader, float(&format!("/{deck}/bpm/current"), 126.0), now);
        feed(reader, text(&format!("/{deck}/track/title"), title), now);
        feed(reader, text(&format!("/{deck}/track/artist"), "Mara Voss"), now);
        feed(reader, text(&format!("/{deck}/track/album"), "Sessions"), now);
    }

    #[test]
    fn a_track_and_its_moving_position_become_a_playing_deck() {
        let (tx, mut events) = mpsc::unbounded_channel();
        let mut reader = Reader::new(tx);
        let start = Instant::now();
        load(&mut reader, "0", "Neon Tide", start);
        feed(&mut reader, float("/0/time", 10.0), start);
        feed(&mut reader, float("/0/time", 10.016), start + Duration::from_millis(16));

        let sent = drain(&mut events);
        assert!(matches!(&sent[0], SourceEvent::Status(status) if !status.connected && status.reader == "rkbx_link"));
        assert!(sent.iter().any(|event| matches!(event, SourceEvent::Status(status) if status.connected)));
        let loaded = sent.iter().find_map(|event| match event {
            SourceEvent::TrackLoaded { deck: 1, meta, .. } => Some(meta.clone()),
            _ => None,
        });
        let loaded = loaded.expect("deck 1 (rkbx_link's deck 0) is loaded");
        assert_eq!((loaded.title.as_str(), loaded.artist.as_str(), loaded.album.as_str()), ("Neon Tide", "Mara Voss", "Sessions"));
        assert_eq!((loaded.bpm_original, loaded.duration_s, &loaded.key), (124.0, 0.0, &None));

        let state = last_state(&sent, 1).expect("a state");
        assert!(state.playing && state.on_air);
        assert_eq!(state.bpm_current, 126.0);
        assert!((state.pos_s - 10.016).abs() < 0.05, "pos {}", state.pos_s);
    }

    #[test]
    fn a_position_that_stops_arriving_pauses_the_deck() {
        let (tx, mut events) = mpsc::unbounded_channel();
        let mut reader = Reader::new(tx);
        let start = Instant::now();
        load(&mut reader, "1", "Glass Garden", start);
        feed(&mut reader, float("/1/time", 30.0), start);
        feed(&mut reader, float("/1/time", 30.1), start + Duration::from_millis(100));
        // The same reading again: paused, and reported at once.
        feed(&mut reader, float("/1/time", 30.1), start + Duration::from_millis(116));
        assert_eq!(last_state(&drain(&mut events), 2).map(|state| state.playing), Some(false));

        feed(&mut reader, float("/1/time", 30.2), start + Duration::from_millis(200));
        assert_eq!(last_state(&drain(&mut events), 2).map(|state| state.playing), Some(true));
        reader.tick(start + Duration::from_millis(200) + STALE + Duration::from_millis(1));
        let state = last_state(&drain(&mut events), 2).expect("a state");
        assert!(!state.playing);
        assert!((state.pos_s - 30.2).abs() < 0.001);
    }

    #[test]
    fn the_master_is_the_deck_with_the_master_position() {
        let (tx, mut events) = mpsc::unbounded_channel();
        let mut reader = Reader::new(tx);
        let now = Instant::now();
        load(&mut reader, "0", "One", now);
        load(&mut reader, "1", "Two", now);
        feed(&mut reader, float("/0/time", 5.0), now);
        feed(&mut reader, float("/1/time", 77.25), now);
        feed(&mut reader, float("/master/time", 77.26), now);
        let sent = drain(&mut events);
        assert_eq!(last_state(&sent, 2).map(|state| state.is_master), Some(true));
        assert_eq!(last_state(&sent, 1).map(|state| state.is_master), Some(false));
    }

    #[test]
    fn an_original_tempo_that_arrives_late_corrects_the_track() {
        let (tx, mut events) = mpsc::unbounded_channel();
        let mut reader = Reader::new(tx);
        let now = Instant::now();
        feed(&mut reader, text("/2/track/title", "Late"), now);
        feed(&mut reader, text("/2/track/artist", ""), now);
        feed(&mut reader, text("/2/track/album", ""), now);
        feed(&mut reader, float("/2/bpm/original", 140.0), now);
        let tempos: Vec<f64> = drain(&mut events)
            .iter()
            .filter_map(|event| match event {
                SourceEvent::TrackLoaded { deck: 3, meta, .. } => Some(meta.bpm_original),
                _ => None,
            })
            .collect();
        assert_eq!(tempos, vec![0.0, 140.0]);
    }

    #[test]
    fn an_empty_track_unloads_and_silence_disconnects() {
        let (tx, mut events) = mpsc::unbounded_channel();
        let mut reader = Reader::new(tx);
        let now = Instant::now();
        load(&mut reader, "0", "One", now);
        for field in ["title", "artist", "album"] {
            feed(&mut reader, text(&format!("/0/track/{field}"), ""), now);
        }
        // Decks rkbx_link cannot have, and addresses we do not use.
        feed(&mut reader, float("/4/time", 1.0), now);
        feed(&mut reader, float("/0/beat/subdiv/1", 0.5), now);
        reader.tick(now + DISCONNECTED + Duration::from_millis(1));

        let sent = drain(&mut events);
        assert!(sent.contains(&SourceEvent::TrackUnloaded { deck: 1 }));
        assert!(matches!(sent.last(), Some(SourceEvent::Status(status)) if !status.connected));
    }

    #[tokio::test]
    async fn datagrams_reach_the_reader_over_udp() {
        let probe = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.expect("a free port");
        let listen = probe.local_addr().expect("an address");
        drop(probe);
        let (tx, mut events) = mpsc::unbounded_channel();
        let task = Box::new(RekordboxSource { listen }).spawn(tx);

        let sender = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.expect("a socket");
        let found = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                // UDP: repeat until the reader has bound and answered.
                for packet in [text("/0/track/title", "Over UDP"), text("/0/track/artist", "A"), text("/0/track/album", "B")] {
                    sender.send_to(&packet, listen).await.expect("sends");
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
                while let Ok(event) = events.try_recv() {
                    if let SourceEvent::TrackLoaded { meta, .. } = event {
                        return meta.title;
                    }
                }
            }
        })
        .await;
        task.abort();
        assert_eq!(found.as_deref(), Ok("Over UDP"));
    }
}
