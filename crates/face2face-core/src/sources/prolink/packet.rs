//! Pro DJ Link UDP packets: the few we read, and the one we send.
//!
//! Layouts are from Deep Symmetry's "DJ Link Ecosystem Analysis"
//! (<https://djl-analysis.deepsymmetry.org>), which documents the protocol
//! from packet captures. All integers are big-endian.

use std::net::Ipv4Addr;

/// Every packet starts with these ten bytes ("Qspt1WmJOL").
pub const MAGIC: [u8; 10] = [0x51, 0x73, 0x70, 0x74, 0x31, 0x57, 0x6d, 0x4a, 0x4f, 0x4c];

/// Devices announce themselves here.
pub const PORT_ANNOUNCE: u16 = 50000;
/// Beats and precise positions are broadcast here.
pub const PORT_BEAT: u16 = 50001;
/// Players send their status here, to every device that announces itself.
pub const PORT_STATUS: u16 = 50002;

const TYPE_KEEP_ALIVE: u8 = 0x06;
const TYPE_STATUS: u8 = 0x0a;
const TYPE_POSITION: u8 = 0x0b;
const TYPE_BEAT: u8 = 0x28;

const NAME: std::ops::Range<usize> = 0x0c..0x20;
/// Pitch fields in status and beat packets: this value means no change.
const PITCH_NEUTRAL: f64 = 0x0010_0000 as f64;

fn kind(packet: &[u8], min_length: usize) -> Option<u8> {
    (packet.len() >= min_length && packet.starts_with(&MAGIC)).then(|| packet[0x0a])
}

fn u16_at(packet: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([packet[offset], packet[offset + 1]])
}

fn u32_at(packet: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([packet[offset], packet[offset + 1], packet[offset + 2], packet[offset + 3]])
}

fn name(packet: &[u8]) -> String {
    String::from_utf8_lossy(&packet[NAME]).trim_end_matches('\0').trim().to_string()
}

/// A device saying it is on the network (port 50000, about every 1.5 s).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeepAlive {
    pub name: String,
    pub device: u8,
    pub mac: [u8; 6],
    pub ip: Ipv4Addr,
}

pub fn parse_keep_alive(packet: &[u8]) -> Option<KeepAlive> {
    if kind(packet, 0x36)? != TYPE_KEEP_ALIVE {
        return None;
    }
    Some(KeepAlive {
        name: name(packet),
        device: packet[0x24],
        mac: packet[0x26..0x2c].try_into().ok()?,
        ip: Ipv4Addr::new(packet[0x2c], packet[0x2d], packet[0x2e], packet[0x2f]),
    })
}

/// Our own keep-alive: we appear as one more player, which is what makes the
/// real players send us their status. The layout is the one a CDJ sends.
pub fn build_keep_alive(device_name: &str, device: u8, mac: [u8; 6], ip: Ipv4Addr) -> [u8; 0x36] {
    let mut packet = [0u8; 0x36];
    packet[..10].copy_from_slice(&MAGIC);
    packet[0x0a] = TYPE_KEEP_ALIVE;
    let name = device_name.as_bytes();
    let length = name.len().min(NAME.len());
    packet[NAME.start..NAME.start + length].copy_from_slice(&name[..length]);
    packet[0x20] = 0x01;
    packet[0x21] = 0x02;
    packet[0x22..0x24].copy_from_slice(&0x0036u16.to_be_bytes()); // packet length
    packet[0x24] = device;
    packet[0x25] = 0x01;
    packet[0x26..0x2c].copy_from_slice(&mac);
    packet[0x2c..0x30].copy_from_slice(&ip.octets());
    packet[0x30] = 0x01;
    packet[0x34] = 0x01; // 1 = player, 2 = mixer
    packet
}

/// Which track a player holds: where it came from and its id there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TrackRef {
    /// The player whose media holds the track (it can be another player's USB stick).
    pub source_player: u8,
    /// 1 CD, 2 SD, 3 USB, 4 rekordbox on a laptop.
    pub slot: u8,
    /// 1 analysed by rekordbox, 2 not analysed, 5 audio CD. 0: nothing loaded.
    pub track_type: u8,
    pub rekordbox_id: u32,
}

impl TrackRef {
    pub fn is_loaded(&self) -> bool {
        self.track_type != 0 && self.source_player != 0
    }
}

/// A player's status (port 50002, about every 200 ms).
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub name: String,
    pub device: u8,
    pub track: TrackRef,
    pub playing: bool,
    pub is_master: bool,
    pub on_air: bool,
    /// The track's own tempo, `None` when it has not been analysed.
    pub bpm_track: Option<f64>,
    /// Tempo multiplier in effect, 1.0 = no pitch change.
    pub pitch: f64,
    /// Beats since the start of the track, `None` without a beat grid.
    pub beat: Option<u32>,
}

pub fn parse_status(packet: &[u8]) -> Option<Status> {
    // 0xd0 is the oldest players' length; newer ones are longer.
    if kind(packet, 0xd0)? != TYPE_STATUS {
        return None;
    }
    let flags = packet[0x89];
    let bpm = u16_at(packet, 0x92);
    let beat = u32_at(packet, 0xa0);
    Some(Status {
        name: name(packet),
        device: packet[0x21],
        track: TrackRef {
            source_player: packet[0x28],
            slot: packet[0x29],
            track_type: packet[0x2a],
            rekordbox_id: u32_at(packet, 0x2c),
        },
        playing: flags & 0x40 != 0,
        is_master: flags & 0x20 != 0,
        on_air: flags & 0x08 != 0,
        bpm_track: (bpm != 0xffff && bpm != 0).then(|| f64::from(bpm) / 100.0),
        pitch: f64::from(u32_at(packet, 0x8c)) / PITCH_NEUTRAL,
        beat: (beat != 0xffff_ffff).then_some(beat),
    })
}

/// A player crossing a beat (port 50001, sent on the beat).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Beat {
    pub device: u8,
    /// Position in the bar, 1 to 4.
    pub beat_in_bar: u8,
}

pub fn parse_beat(packet: &[u8]) -> Option<Beat> {
    if kind(packet, 0x60)? != TYPE_BEAT {
        return None;
    }
    Some(Beat { device: packet[0x21], beat_in_bar: packet[0x5c] })
}

/// The exact playhead (port 50001, every 30 ms while a track is loaded).
/// Sent by the CDJ-3000 and later players only.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position {
    pub device: u8,
    /// Whole seconds, rounded down.
    pub track_length_s: u32,
    pub playhead_s: f64,
    /// Tempo in effect, `None` when the track has no tempo.
    pub bpm_current: Option<f64>,
}

pub fn parse_position(packet: &[u8]) -> Option<Position> {
    if kind(packet, 0x3c)? != TYPE_POSITION {
        return None;
    }
    let bpm = u32_at(packet, 0x38);
    Some(Position {
        device: packet[0x21],
        track_length_s: u32_at(packet, 0x24),
        playhead_s: f64::from(u32_at(packet, 0x28)) / 1000.0,
        bpm_current: (bpm != 0xffff_ffff && bpm != 0).then(|| f64::from(bpm) / 10.0),
    })
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    fn blank(kind: u8, length: usize, name: &str, device: u8) -> Vec<u8> {
        let mut packet = vec![0u8; length];
        packet[..10].copy_from_slice(&MAGIC);
        packet[0x0a] = kind;
        packet[NAME.start..NAME.start + name.len()].copy_from_slice(name.as_bytes());
        packet[0x21] = device;
        packet
    }

    /// A nexus-length status packet with the fields we read filled in.
    pub fn status(device: u8, track: TrackRef, flags: u8, bpm_x100: u16, pitch: u32, beat: u32) -> Vec<u8> {
        let mut packet = blank(TYPE_STATUS, 0xd4, "CDJ-2000nexus", device);
        packet[0x20] = 0x03;
        packet[0x28] = track.source_player;
        packet[0x29] = track.slot;
        packet[0x2a] = track.track_type;
        packet[0x2c..0x30].copy_from_slice(&track.rekordbox_id.to_be_bytes());
        packet[0x89] = flags;
        packet[0x8c..0x90].copy_from_slice(&pitch.to_be_bytes());
        packet[0x92..0x94].copy_from_slice(&bpm_x100.to_be_bytes());
        packet[0xa0..0xa4].copy_from_slice(&beat.to_be_bytes());
        packet
    }

    pub fn beat(device: u8, beat_in_bar: u8) -> Vec<u8> {
        let mut packet = blank(TYPE_BEAT, 0x60, "CDJ-2000nexus", device);
        packet[0x5c] = beat_in_bar;
        packet
    }

    pub fn position(device: u8, track_length_s: u32, playhead_ms: u32, bpm_x10: u32) -> Vec<u8> {
        let mut packet = blank(TYPE_POSITION, 0x3c, "CDJ-3000", device);
        packet[0x24..0x28].copy_from_slice(&track_length_s.to_be_bytes());
        packet[0x28..0x2c].copy_from_slice(&playhead_ms.to_be_bytes());
        packet[0x38..0x3c].copy_from_slice(&bpm_x10.to_be_bytes());
        packet
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{beat, position, status};
    use super::*;

    /// A CDJ's keep-alive, written out by hand from the layout the analysis
    /// documents (addresses invented): player 2 at 169.254.29.90, which has
    /// seen two devices.
    const CDJ_KEEP_ALIVE: [u8; 0x36] = [
        0x51, 0x73, 0x70, 0x74, 0x31, 0x57, 0x6d, 0x4a, 0x4f, 0x4c, 0x06, 0x00, b'C', b'D', b'J', b'-', //
        b'2', b'0', b'0', b'0', b'n', b'e', b'x', b'u', b's', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
        0x01, 0x02, 0x00, 0x36, 0x02, 0x01, 0x74, 0x5e, 0x1c, 0x56, 0x2a, 0x10, 0xa9, 0xfe, 0x1d, 0x5a, //
        0x02, 0x00, 0x00, 0x00, 0x01, 0x00,
    ];

    #[test]
    fn a_keep_alive_is_read() {
        let keep_alive = parse_keep_alive(&CDJ_KEEP_ALIVE).expect("valid");
        assert_eq!(keep_alive.name, "CDJ-2000nexus");
        assert_eq!(keep_alive.device, 2);
        assert_eq!(keep_alive.mac, [0x74, 0x5e, 0x1c, 0x56, 0x2a, 0x10]);
        assert_eq!(keep_alive.ip, Ipv4Addr::new(169, 254, 29, 90));
    }

    #[test]
    fn our_keep_alive_has_a_players_layout() {
        let ours = build_keep_alive("CDJ-2000nexus", 2, [0x74, 0x5e, 0x1c, 0x56, 0x2a, 0x10], Ipv4Addr::new(169, 254, 29, 90));
        // Identical to the real one except the count of devices seen (0x30),
        // which we leave at 1.
        let mut expected = CDJ_KEEP_ALIVE;
        expected[0x30] = 0x01;
        assert_eq!(ours, expected);
        // A name longer than the field is cut, not spilled into the next one.
        let long = build_keep_alive("A name much longer than twenty bytes", 5, [0; 6], Ipv4Addr::LOCALHOST);
        assert_eq!(&long[0x20..0x25], &[0x01, 0x02, 0x00, 0x36, 0x05]);
    }

    #[test]
    fn a_status_is_read() {
        let track = TrackRef { source_player: 2, slot: 3, track_type: 1, rekordbox_id: 0x0001_e240 };
        // Playing, master, on air; 128.00 BPM at +6 %; beat 65.
        let packet = status(3, track, 0x40 | 0x20 | 0x08 | 0x84, 12800, 0x0010_f5c2, 65);
        let status = parse_status(&packet).expect("valid");
        assert_eq!((status.device, status.track), (3, track));
        assert!(status.playing && status.is_master && status.on_air);
        assert_eq!(status.bpm_track, Some(128.0));
        assert!((status.pitch - 1.06).abs() < 1e-5, "pitch {}", status.pitch);
        assert_eq!(status.beat, Some(65));
        assert_eq!(status.name, "CDJ-2000nexus");
    }

    #[test]
    fn an_empty_or_unanalysed_deck_has_no_tempo_or_beat() {
        let packet = status(1, TrackRef::default(), 0x84, 0xffff, 0x0010_0000, 0xffff_ffff);
        let status = parse_status(&packet).expect("valid");
        assert!(!status.track.is_loaded());
        assert!(!status.playing && !status.is_master && !status.on_air);
        assert_eq!((status.bpm_track, status.beat, status.pitch), (None, None, 1.0));
    }

    #[test]
    fn beats_and_positions_are_read() {
        assert_eq!(parse_beat(&beat(4, 3)), Some(Beat { device: 4, beat_in_bar: 3 }));
        let read = parse_position(&position(1, 372, 64_512, 1302)).expect("valid");
        assert_eq!(read, Position { device: 1, track_length_s: 372, playhead_s: 64.512, bpm_current: Some(130.2) });
        assert_eq!(parse_position(&position(1, 372, 0, 0xffff_ffff)).and_then(|read| read.bpm_current), None);
    }

    #[test]
    fn each_parser_takes_only_its_own_packets() {
        let status_packet = status(1, TrackRef::default(), 0, 0, 0, 0);
        let beat_packet = beat(1, 1);
        let position_packet = position(1, 1, 1, 1);
        assert!(parse_status(&beat_packet).is_none() && parse_status(&CDJ_KEEP_ALIVE).is_none());
        assert!(parse_beat(&position_packet).is_none() && parse_beat(&status_packet).is_none());
        assert!(parse_position(&beat_packet).is_none());
        assert!(parse_keep_alive(&status_packet).is_none());
        // Cut short, or not Pro DJ Link at all.
        assert!(parse_status(&status_packet[..0x90]).is_none());
        assert!(parse_keep_alive(&CDJ_KEEP_ALIVE[..0x30]).is_none());
        assert!(parse_beat(&[0u8; 0x60]).is_none());
        assert!(parse_position(b"").is_none());
    }
}
