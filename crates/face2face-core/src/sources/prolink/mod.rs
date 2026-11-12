//! Pioneer / AlphaTheta players on a Pro DJ Link network (CDJ, XDJ).
//!
//! Works the way PRO DJ LINK Bridge does: we announce ourselves on the network
//! as one more player (a "virtual CDJ"), and the real players then send us
//! their status. Nothing is installed on the players and nothing is sent to
//! them except the announcements and the questions about track names.
//!
//! - Port 50000: keep-alives. We listen to find the players and send our own.
//! - Port 50002: each player's status, five times a second: track, play
//!   state, master, on air, tempo, beat number.
//! - Port 50001: beats, and on the CDJ-3000 and later the exact playhead
//!   every 30 ms.
//! - TCP, [`dbserver`]: title, artist, length, tempo and key of a track.
//!
//! Players 1 to 4 become decks 1 to 4.
//!
//! **Position.** Players that send the exact playhead are shown exactly. The
//! older ones only report a beat number, and the position is estimated as
//! `(beat - 1) * 60 / track tempo`: right for a track with a steady tempo
//! whose first beat is at the start, off by the length of the intro otherwise.

pub mod dbserver;
pub mod packet;

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::{Duration, Instant};

use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use self::dbserver::{track_metadata, TrackMetadata, PORT_FINDER};
use self::packet::{
    build_keep_alive, parse_beat, parse_keep_alive, parse_position, parse_status, Beat, KeepAlive, Position, Status,
    TrackRef, PORT_ANNOUNCE, PORT_BEAT, PORT_STATUS,
};
use super::board::{DeckBoard, DeckUpdate, TICK};
use super::key::parse_key;
use super::{DeckSource, SourceEvent, SourceStatus, TrackMeta, RECEIVE_ERROR_PAUSE, RETRY_OPEN};
use crate::protocol::MAX_DECKS;

const KEEP_ALIVE_EVERY: Duration = Duration::from_millis(1500);
/// Players announce themselves every 1.5 to 2 s and report status every 0.2 s.
const DEVICE_GONE_AFTER: Duration = Duration::from_secs(6);
const LOOKUP_TIME_LIMIT: Duration = Duration::from_secs(4);
/// A beat packet this recent says when the beat in a status packet happened.
const BEAT_PACKET_WINDOW: Duration = Duration::from_millis(400);
/// While exact positions arrive this often, beat numbers are not used.
const POSITION_FRESH: Duration = Duration::from_secs(1);

pub struct ProDjLinkSource {
    /// The name we announce, at most 20 bytes.
    pub device_name: String,
    /// The player number we announce. Above 4, so that it can never take the
    /// number of a real player.
    pub device: u8,
}

impl Default for ProDjLinkSource {
    fn default() -> Self {
        Self { device_name: "Face2Face DJ".to_string(), device: 5 }
    }
}

/// A question for a player's dbserver, to be asked off the main loop.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Lookup {
    deck: u8,
    track: TrackRef,
    /// The device whose media holds the track.
    ask: Ipv4Addr,
    posing_as: u8,
}

struct Device {
    ip: Ipv4Addr,
    last_seen: Instant,
}

#[derive(Default)]
struct Player {
    track: TrackRef,
    /// The latest status, applied again once the track's details are known.
    status: Option<Status>,
    beat: Option<u32>,
    beat_packet_at: Option<Instant>,
    position_at: Option<Instant>,
    track_length_s: u32,
}

/// The reader's logic, separate from the sockets so tests can feed it packets.
struct Link {
    board: DeckBoard,
    own_name: String,
    own_device: u8,
    /// Everything that announces itself, by device number: players, mixers, laptops.
    devices: BTreeMap<u8, Device>,
    players: [Player; MAX_DECKS as usize],
    reported: Option<(usize, u8)>,
}

impl Link {
    fn new(events: mpsc::UnboundedSender<SourceEvent>, own_name: &str, own_device: u8) -> Self {
        let mut link = Self {
            board: DeckBoard::new(events),
            own_name: own_name.to_string(),
            own_device,
            devices: BTreeMap::new(),
            players: Default::default(),
            reported: None,
        };
        link.report();
        link
    }

    fn status_message(connected: bool, decks: u8, reader_status: String) -> SourceStatus {
        SourceStatus {
            software: "pro dj link".to_string(),
            software_version: String::new(),
            connected,
            decks,
            reader: "virtual cdj".to_string(),
            reader_status,
        }
    }

    fn report(&mut self) {
        let players = self.devices.keys().filter(|&&device| (1..=MAX_DECKS).contains(&device)).count();
        let highest = self.devices.keys().copied().filter(|device| (1..=MAX_DECKS).contains(device)).max();
        let decks = highest.unwrap_or(0).max(2);
        if self.reported == Some((players, decks)) {
            return;
        }
        self.reported = Some((players, decks));
        let reader_status = match players {
            0 => "waiting for players".to_string(),
            1 => "1 player".to_string(),
            n => format!("{n} players"),
        };
        self.board.status(Self::status_message(players > 0, decks, reader_status));
    }

    fn is_own(&self, name: &str, device: u8) -> bool {
        device == self.own_device && name == self.own_name
    }

    fn seen(&mut self, device: u8, ip: Ipv4Addr, now: Instant) {
        self.devices.insert(device, Device { ip, last_seen: now });
        self.report();
    }

    /// Returns `true` for a device other than ourselves.
    fn keep_alive(&mut self, keep_alive: &KeepAlive, now: Instant) -> bool {
        if self.is_own(&keep_alive.name, keep_alive.device) {
            return false;
        }
        self.seen(keep_alive.device, keep_alive.ip, now);
        true
    }

    /// The number to give the dbserver of `target`: another real player's if
    /// there is one (see [`dbserver::track_metadata`]), else our own.
    fn posing_as(&self, target: u8) -> u8 {
        self.devices
            .keys()
            .copied()
            .find(|&device| (1..=MAX_DECKS).contains(&device) && device != target)
            .unwrap_or(self.own_device)
    }

    /// A status packet from `from`. Returns a lookup to start when the player
    /// holds a track we know nothing about yet.
    fn status(&mut self, status: Status, from: Ipv4Addr, now: Instant) -> Option<Lookup> {
        if self.is_own(&status.name, status.device) {
            return None;
        }
        self.seen(status.device, from, now);
        let deck = status.device;
        let index = usize::from(deck).checked_sub(1).filter(|&index| index < self.players.len())?;

        let changed = self.players[index].track != status.track;
        if changed {
            self.board.unload(deck);
            self.players[index] = Player { track: status.track, ..Player::default() };
        }
        self.players[index].status = Some(status.clone());
        if !status.track.is_loaded() {
            return None;
        }
        if changed {
            // The track's details come from the device whose media it is on.
            if let Some(source) = self.devices.get(&status.track.source_player) {
                let posing_as = self.posing_as(status.track.source_player);
                return Some(Lookup { deck, track: status.track, ask: source.ip, posing_as });
            }
            self.load(deck, status.track, None);
            return None;
        }
        self.apply_status(index, now);
        None
    }

    /// The answer to a [`Lookup`]; `None` if the player could not be asked.
    fn load(&mut self, deck: u8, track: TrackRef, found: Option<TrackMetadata>) {
        let Some(index) = usize::from(deck).checked_sub(1).filter(|&index| index < self.players.len()) else { return };
        let player = &self.players[index];
        if player.track != track || !track.is_loaded() {
            return; // the DJ loaded something else in the meantime
        }
        let found = found.unwrap_or_default();
        let bpm_track = player.status.as_ref().and_then(|status| status.bpm_track);
        let meta = TrackMeta {
            title: if found.title.is_empty() { "Unknown track".to_string() } else { found.title },
            artist: found.artist,
            album: found.album,
            // The database rounds to seconds, like the position packets.
            duration_s: if found.duration_s > 0.0 { found.duration_s } else { f64::from(player.track_length_s) },
            bpm_original: if found.bpm > 0.0 { found.bpm } else { bpm_track.unwrap_or(0.0) },
            key: parse_key(&found.key),
            file_name: String::new(),
        };
        self.board.load(deck, meta, None);
        self.players[index].beat = None;
        self.apply_status(index, Instant::now());
    }

    fn apply_status(&mut self, index: usize, now: Instant) {
        let deck = index as u8 + 1;
        let player = &mut self.players[index];
        let Some(status) = player.status.clone() else { return };
        if !self.board.is_loaded(deck) {
            return;
        }
        let mut update = DeckUpdate {
            playing: Some(status.playing),
            is_master: Some(status.is_master),
            on_air: Some(status.on_air),
            ..DeckUpdate::default()
        };
        let mut at = now;
        let exact = player.position_at.is_some_and(|seen| now.saturating_duration_since(seen) < POSITION_FRESH);
        if !exact {
            update.bpm_current = status.bpm_track.map(|bpm| bpm * status.pitch);
            // Re-anchor only when the beat number moves: it stays the same for
            // a whole beat, during which the board carries the playhead on.
            if let (Some(beat), Some(bpm)) = (status.beat, status.bpm_track) {
                if player.beat != Some(beat) {
                    player.beat = Some(beat);
                    update.pos_s = Some(f64::from(beat.saturating_sub(1)) * 60.0 / bpm);
                    // The status packet can be 200 ms late; the beat packet was on time.
                    if let Some(beat_at) = player.beat_packet_at.filter(|&at| now.saturating_duration_since(at) < BEAT_PACKET_WINDOW) {
                        at = beat_at;
                    }
                }
            }
        }
        self.board.update_at(deck, at, update);
    }

    fn beat(&mut self, beat: Beat, now: Instant) {
        if let Some(player) = usize::from(beat.device).checked_sub(1).and_then(|index| self.players.get_mut(index)) {
            player.beat_packet_at = Some(now);
        }
    }

    fn position(&mut self, position: Position, now: Instant) {
        let deck = position.device;
        let Some(player) = usize::from(deck).checked_sub(1).and_then(|index| self.players.get_mut(index)) else { return };
        player.position_at = Some(now);
        player.track_length_s = position.track_length_s;
        let length = f64::from(position.track_length_s);
        self.board.revise(deck, |meta| {
            if meta.duration_s <= 0.0 {
                meta.duration_s = length;
            }
        });
        let update = DeckUpdate { pos_s: Some(position.playhead_s), bpm_current: position.bpm_current, ..DeckUpdate::default() };
        self.board.update_at(deck, now, update);
    }

    /// Forgets devices that went quiet, and keeps playing decks moving.
    fn tick(&mut self, now: Instant) {
        let gone: Vec<u8> = self
            .devices
            .iter()
            .filter(|(_, device)| now.saturating_duration_since(device.last_seen) > DEVICE_GONE_AFTER)
            .map(|(&number, _)| number)
            .collect();
        for number in gone {
            self.devices.remove(&number);
            if let Some(player) = usize::from(number).checked_sub(1).and_then(|index| self.players.get_mut(index)) {
                *player = Player::default();
                self.board.unload(number);
            }
        }
        self.report();
        self.board.tick();
    }
}

/// A UDP socket on a Pro DJ Link port. The address is shared, so rekordbox
/// or PRO DJ LINK Bridge can run on the same computer.
fn bind(port: u16) -> std::io::Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    socket.set_broadcast(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port).into())?;
    UdpSocket::from_std(socket.into())
}

/// Our address, broadcast address and hardware address on the network that
/// `peer` is on.
fn interface_towards(peer: Ipv4Addr) -> Option<(Ipv4Addr, Ipv4Addr, [u8; 6])> {
    netdev::get_interfaces().into_iter().find_map(|interface| {
        let net = interface.ipv4.iter().find(|net| net.contains(&peer))?;
        Some((net.addr(), net.broadcast(), interface.mac_addr?.octets()))
    })
}

fn source_ip(from: SocketAddr) -> Option<Ipv4Addr> {
    match from {
        SocketAddr::V4(address) => Some(*address.ip()),
        SocketAddr::V6(_) => None,
    }
}

impl DeckSource for ProDjLinkSource {
    fn name(&self) -> &'static str {
        "pro_dj_link"
    }

    fn spawn(self: Box<Self>, events: mpsc::UnboundedSender<SourceEvent>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut link = Link::new(events, &self.device_name, self.device);
            // Say once why the ports cannot be opened, then keep trying.
            let mut reported = false;
            let (announce, beats, statuses) = loop {
                match bind(PORT_ANNOUNCE).and_then(|a| Ok((a, bind(PORT_BEAT)?, bind(PORT_STATUS)?))) {
                    Ok(sockets) => break sockets,
                    Err(error) => {
                        if !reported {
                            let message = format!("cannot listen on the Pro DJ Link ports: {error}");
                            link.board.status(Link::status_message(false, 2, message));
                            reported = true;
                        }
                        tokio::time::sleep(RETRY_OPEN).await;
                    }
                }
            };
            if reported {
                link.board.status(Link::status_message(false, 2, "waiting for players".to_string()));
            }
            let (found_tx, mut found_rx) = mpsc::unbounded_channel::<(Lookup, Option<TrackMetadata>)>();
            // Where to send our keep-alive, once a player has shown which network it is on.
            let mut announcement: Option<(SocketAddrV4, [u8; 0x36])> = None;
            let mut keep_alives = tokio::time::interval(KEEP_ALIVE_EVERY);
            let mut ticks = tokio::time::interval(TICK);
            ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
            let (mut a, mut b, mut s) = ([0u8; 1024], [0u8; 1024], [0u8; 2048]);

            loop {
                tokio::select! {
                    received = announce.recv_from(&mut a) => {
                        let Ok((length, _)) = received else { tokio::time::sleep(RECEIVE_ERROR_PAUSE).await; continue };
                        let Some(keep_alive) = parse_keep_alive(&a[..length]) else { continue };
                        if link.keep_alive(&keep_alive, Instant::now()) && announcement.is_none() {
                            announcement = interface_towards(keep_alive.ip).map(|(ip, broadcast, mac)| {
                                tracing::info!("Pro DJ Link: joining as player {} from {ip}", self.device);
                                (SocketAddrV4::new(broadcast, PORT_ANNOUNCE), build_keep_alive(&self.device_name, self.device, mac, ip))
                            });
                            keep_alives.reset_immediately();
                        }
                    }
                    received = beats.recv_from(&mut b) => {
                        let Ok((length, _)) = received else { tokio::time::sleep(RECEIVE_ERROR_PAUSE).await; continue };
                        let now = Instant::now();
                        if let Some(position) = parse_position(&b[..length]) {
                            link.position(position, now);
                        } else if let Some(beat) = parse_beat(&b[..length]) {
                            link.beat(beat, now);
                        }
                    }
                    received = statuses.recv_from(&mut s) => {
                        let Ok((length, from)) = received else { tokio::time::sleep(RECEIVE_ERROR_PAUSE).await; continue };
                        let (Some(status), Some(from)) = (parse_status(&s[..length]), source_ip(from)) else { continue };
                        if let Some(lookup) = link.status(status, from, Instant::now()) {
                            let found = found_tx.clone();
                            tokio::spawn(async move {
                                let finder = SocketAddr::from((lookup.ask, PORT_FINDER));
                                let asked = tokio::time::timeout(LOOKUP_TIME_LIMIT, track_metadata(finder, lookup.posing_as, lookup.track)).await;
                                let metadata = match asked {
                                    Ok(Ok(metadata)) => Some(metadata),
                                    Ok(Err(error)) => {
                                        tracing::warn!("Pro DJ Link: no details for the track on player {}: {error:#}", lookup.deck);
                                        None
                                    }
                                    Err(_) => {
                                        tracing::warn!("Pro DJ Link: player {} did not answer about its track", lookup.deck);
                                        None
                                    }
                                };
                                let _ = found.send((lookup, metadata));
                            });
                        }
                    }
                    Some((lookup, metadata)) = found_rx.recv() => link.load(lookup.deck, lookup.track, metadata),
                    _ = keep_alives.tick() => {
                        if let Some((to, packet)) = &announcement {
                            if let Err(error) = announce.send_to(packet, to).await {
                                tracing::warn!("Pro DJ Link: could not announce to {to}: {error}");
                                announcement = None; // the network changed; find it again
                            }
                        }
                    }
                    _ = ticks.tick() => link.tick(Instant::now()),
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::board::testing::{drain, last_state};
    use super::dbserver::testing::fake_player;
    use super::packet::testing as packets;
    use super::*;

    const USB_TRACK: TrackRef = TrackRef { source_player: 2, slot: 3, track_type: 1, rekordbox_id: 1234 };
    const PLAYING: u8 = 0x40;
    const MASTER: u8 = 0x20;
    const ON_AIR: u8 = 0x08;
    const NO_PITCH: u32 = 0x0010_0000;

    fn ip(last: u8) -> Ipv4Addr {
        Ipv4Addr::new(169, 254, 1, last)
    }

    fn link() -> (Link, mpsc::UnboundedReceiver<SourceEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Link::new(tx, "Face2Face DJ", 5), rx)
    }

    fn announce(link: &mut Link, device: u8, now: Instant) {
        let keep_alive = KeepAlive { name: "CDJ-2000nexus".to_string(), device, mac: [0; 6], ip: ip(device) };
        assert!(link.keep_alive(&keep_alive, now));
    }

    fn status(link: &mut Link, device: u8, track: TrackRef, flags: u8, beat: u32, now: Instant) -> Option<Lookup> {
        let packet = packets::status(device, track, flags, 12000, NO_PITCH, beat);
        link.status(parse_status(&packet).expect("valid"), ip(device), now)
    }

    fn details() -> TrackMetadata {
        TrackMetadata {
            title: "Neon Tide".to_string(),
            artist: "Mara Voss".to_string(),
            album: String::new(),
            duration_s: 372.0,
            bpm: 120.0,
            key: "8A".to_string(),
        }
    }

    #[test]
    fn a_new_track_is_looked_up_on_the_player_that_holds_it() {
        let (mut link, mut events) = link();
        let now = Instant::now();
        announce(&mut link, 2, now);
        announce(&mut link, 3, now);
        // Player 3 plays a track from the USB stick in player 2.
        let lookup = status(&mut link, 3, USB_TRACK, PLAYING, 9, now).expect("a lookup");
        assert_eq!(lookup, Lookup { deck: 3, track: USB_TRACK, ask: ip(2), posing_as: 3 });
        // The same status again does not ask again.
        assert_eq!(status(&mut link, 3, USB_TRACK, PLAYING, 9, now), None);
        let sent = drain(&mut events);
        assert!(!sent.iter().any(|event| matches!(event, SourceEvent::TrackLoaded { .. })), "nothing until the answer");
        assert!(matches!(sent.last(), Some(SourceEvent::Status(status)) if status.connected && status.decks == 3 && status.reader_status == "2 players"));

        link.load(3, USB_TRACK, Some(details()));
        let sent = drain(&mut events);
        let meta = sent.iter().find_map(|event| match event {
            SourceEvent::TrackLoaded { deck: 3, meta, .. } => Some(meta.clone()),
            _ => None,
        });
        let meta = meta.expect("deck 3 is loaded");
        assert_eq!((meta.title.as_str(), meta.duration_s, meta.bpm_original), ("Neon Tide", 372.0, 120.0));
        assert_eq!(meta.key.map(|key| key.musical).as_deref(), Some("Am"));
        // Beat 9 at 120 BPM is 4 s in.
        let state = last_state(&sent, 3).expect("the status is applied once the track is known");
        assert!(state.playing && !state.is_master && !state.on_air);
        assert!((state.pos_s - 4.0).abs() < 0.05, "pos {}", state.pos_s);
    }

    #[test]
    fn a_lone_player_is_asked_under_our_own_number() {
        let (mut link, _events) = link();
        let now = Instant::now();
        announce(&mut link, 1, now);
        let own_usb = TrackRef { source_player: 1, ..USB_TRACK };
        assert_eq!(status(&mut link, 1, own_usb, 0, 1, now).map(|lookup| lookup.posing_as), Some(5));
    }

    #[test]
    fn the_beat_number_moves_the_playhead_and_the_beat_packet_times_it() {
        let (mut link, mut events) = link();
        let start = Instant::now() - Duration::from_secs(1);
        announce(&mut link, 2, start);
        status(&mut link, 2, USB_TRACK, PLAYING | MASTER | ON_AIR, 9, start);
        link.load(2, USB_TRACK, Some(details()));
        drain(&mut events);

        // Same beat 200 ms later: no jump back to the start of the beat.
        status(&mut link, 2, USB_TRACK, PLAYING | MASTER | ON_AIR, 9, start + Duration::from_millis(200));
        // Beat 10 happened at +500 ms; its status packet comes at +650 ms.
        let beat_packet = parse_beat(&packets::beat(2, 2)).expect("valid");
        link.beat(beat_packet, start + Duration::from_millis(500));
        status(&mut link, 2, USB_TRACK, PLAYING | MASTER | ON_AIR, 10, start + Duration::from_millis(650));

        let state = last_state(&drain(&mut events), 2).expect("a state");
        assert!(state.is_master && state.on_air);
        // 4.5 s at the beat, plus the half second since.
        assert!((state.pos_s - 5.0).abs() < 0.05, "pos {}", state.pos_s);
    }

    #[test]
    fn exact_positions_take_over_from_beat_numbers() {
        let (mut link, mut events) = link();
        let now = Instant::now();
        announce(&mut link, 1, now);
        let own_usb = TrackRef { source_player: 1, ..USB_TRACK };
        status(&mut link, 1, own_usb, PLAYING, 9, now);
        link.load(1, own_usb, None); // the player could not be asked
        let sent = drain(&mut events);
        assert!(sent.iter().any(|event| matches!(event, SourceEvent::TrackLoaded { meta, .. } if meta.title == "Unknown track" && meta.bpm_original == 120.0)));

        let exact = parse_position(&packets::position(1, 372, 64_512, 1302)).expect("valid");
        link.position(exact, now);
        status(&mut link, 1, own_usb, PLAYING, 200, now);
        let sent = drain(&mut events);
        assert!(sent.iter().any(|event| matches!(event, SourceEvent::TrackLoaded { meta, .. } if meta.duration_s == 372.0)), "the length arrives with the position");
        let state = last_state(&sent, 1).expect("a state");
        assert!((state.pos_s - 64.512).abs() < 0.05, "beat 200 is ignored: pos {}", state.pos_s);
        assert_eq!(state.bpm_current, 130.2);
    }

    #[test]
    fn ejecting_changing_and_leaving_empty_the_deck() {
        let (mut link, mut events) = link();
        let now = Instant::now();
        announce(&mut link, 1, now);
        announce(&mut link, 2, now);
        status(&mut link, 2, USB_TRACK, 0, 1, now);
        link.load(2, USB_TRACK, Some(details()));

        // An answer for a track that is no longer on the deck is dropped.
        let other = TrackRef { rekordbox_id: 99, ..USB_TRACK };
        assert!(status(&mut link, 2, other, 0, 1, now).is_some());
        link.load(2, USB_TRACK, Some(details()));
        let sent = drain(&mut events);
        assert_eq!(sent.iter().filter(|event| matches!(event, SourceEvent::TrackLoaded { .. })).count(), 1);
        assert!(sent.contains(&SourceEvent::TrackUnloaded { deck: 2 }));

        link.load(2, other, Some(details()));
        assert_eq!(status(&mut link, 2, TrackRef::default(), 0, 0xffff_ffff, now), None);
        assert!(drain(&mut events).contains(&SourceEvent::TrackUnloaded { deck: 2 }));

        status(&mut link, 2, USB_TRACK, 0, 1, now);
        link.load(2, USB_TRACK, Some(details()));
        drain(&mut events);
        link.tick(now + DEVICE_GONE_AFTER + Duration::from_millis(1));
        let sent = drain(&mut events);
        assert!(sent.contains(&SourceEvent::TrackUnloaded { deck: 2 }));
        assert!(matches!(sent.last(), Some(SourceEvent::Status(status)) if !status.connected));
    }

    #[test]
    fn our_own_packets_mixers_and_players_five_and_six_are_not_decks() {
        let (mut link, mut events) = link();
        let now = Instant::now();
        let own = KeepAlive { name: "Face2Face DJ".to_string(), device: 5, mac: [0; 6], ip: ip(9) };
        assert!(!link.keep_alive(&own, now));
        announce(&mut link, 0x21, now); // a mixer
        assert_eq!(status(&mut link, 6, TrackRef { source_player: 6, ..USB_TRACK }, PLAYING, 1, now), None);
        let sent = drain(&mut events);
        assert!(sent.iter().all(|event| matches!(event, SourceEvent::Status(status) if !status.connected)));
    }

    #[tokio::test]
    async fn the_lookup_runs_against_a_dbserver() {
        let (finder, served) = fake_player(details()).await;
        let found = track_metadata(finder, 3, USB_TRACK).await.expect("found");
        assert_eq!(found.title, "Neon Tide");
        assert_eq!(served.await.expect("ran").0, 3);
    }

    #[test]
    fn the_ports_can_be_shared_with_other_software() {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a runtime");
        let _guard = runtime.enter();
        let first = bind(0).expect("binds");
        let port = first.local_addr().expect("an address").port();
        bind(port).expect("a second socket on the same port");
    }
}
