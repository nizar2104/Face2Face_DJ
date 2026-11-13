//! Asking a player what a track is.
//!
//! Status packets only carry a track's id in the rekordbox database on the
//! player's USB stick or SD card. Players serve that database to each other
//! over TCP (the "dbserver"), and we ask the same way: find the port, greet,
//! say which player we are, request the track's metadata "menu", and read its
//! items. Protocol as documented by Deep Symmetry's DJ Link analysis
//! (<https://djl-analysis.deepsymmetry.org/djl-analysis/track_metadata.html>).

use std::io;
use std::net::SocketAddr;

use anyhow::{bail, ensure, Context};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::packet::TrackRef;

/// Players answer here with the port of their dbserver.
pub const PORT_FINDER: u16 = 12523;

const MESSAGE_MAGIC: u32 = 0x8723_49ae;
/// The transaction id of the setup message.
const TX_SETUP: u32 = 0xffff_fffe;

const KIND_SETUP: u16 = 0x0000;
const KIND_TRACK_METADATA: u16 = 0x2002;
const KIND_UNANALYSED_METADATA: u16 = 0x2202;
const KIND_RENDER_MENU: u16 = 0x3000;
const KIND_OK: u16 = 0x4000;
const KIND_MENU_ITEM: u16 = 0x4101;
const KIND_MENU_FOOTER: u16 = 0x4201;

const ITEM_ALBUM: u32 = 0x02;
const ITEM_TITLE: u32 = 0x04;
const ITEM_ARTIST: u32 = 0x07;
const ITEM_DURATION: u32 = 0x0b;
const ITEM_TEMPO: u32 = 0x0d;
const ITEM_KEY: u32 = 0x0f;

/// Limits on what a peer can make us allocate.
const MAX_BLOB: usize = 1 << 20;
const MAX_TEXT_CHARS: usize = 4096;
const MAX_ARGS: usize = 12;
const MAX_MENU_ITEMS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Field {
    Number(u32),
    Blob(Vec<u8>),
    Text(String),
}

impl Field {
    fn number(&self) -> Option<u32> {
        match self {
            Field::Number(value) => Some(*value),
            _ => None,
        }
    }

    fn text(&self) -> Option<&str> {
        match self {
            Field::Text(text) => Some(text),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub tx: u32,
    pub kind: u16,
    pub args: Vec<Field>,
}

/// On the wire: `11` magic, `11` transaction, `10` kind, `0f` argument count,
/// `14` a 12-byte list of argument types, then the arguments. Each field
/// starts with its type byte; numbers are big-endian.
pub fn encode(message: &Message) -> Vec<u8> {
    let mut bytes = Vec::new();
    let number = |bytes: &mut Vec<u8>, value: u32| {
        bytes.push(0x11);
        bytes.extend(value.to_be_bytes());
    };
    number(&mut bytes, MESSAGE_MAGIC);
    number(&mut bytes, message.tx);
    bytes.push(0x10);
    bytes.extend(message.kind.to_be_bytes());
    bytes.extend([0x0f, message.args.len() as u8]);
    let mut tags = [0u8; MAX_ARGS];
    for (tag, arg) in tags.iter_mut().zip(&message.args) {
        *tag = match arg {
            Field::Text(_) => 0x02,
            Field::Blob(_) => 0x03,
            Field::Number(_) => 0x06,
        };
    }
    bytes.extend([0x14, 0, 0, 0, MAX_ARGS as u8]);
    bytes.extend(tags);
    for arg in &message.args {
        match arg {
            Field::Number(value) => number(&mut bytes, *value),
            Field::Blob(blob) => {
                bytes.push(0x14);
                bytes.extend((blob.len() as u32).to_be_bytes());
                bytes.extend(blob);
            }
            Field::Text(text) => {
                // UTF-16, with a terminating NUL that the length includes.
                let units: Vec<u16> = text.encode_utf16().chain([0]).collect();
                bytes.push(0x26);
                bytes.extend((units.len() as u32).to_be_bytes());
                bytes.extend(units.iter().flat_map(|unit| unit.to_be_bytes()));
            }
        }
    }
    bytes
}

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

async fn read_field<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<Field> {
    match reader.read_u8().await? {
        0x0f => Ok(Field::Number(u32::from(reader.read_u8().await?))),
        0x10 => Ok(Field::Number(u32::from(reader.read_u16().await?))),
        0x11 => Ok(Field::Number(reader.read_u32().await?)),
        0x14 => {
            let length = reader.read_u32().await? as usize;
            if length > MAX_BLOB {
                return Err(invalid(format!("blob of {length} bytes")));
            }
            let mut blob = vec![0u8; length];
            reader.read_exact(&mut blob).await?;
            Ok(Field::Blob(blob))
        }
        0x26 => {
            let chars = reader.read_u32().await? as usize;
            if chars > MAX_TEXT_CHARS {
                return Err(invalid(format!("string of {chars} characters")));
            }
            let mut units = Vec::with_capacity(chars);
            for _ in 0..chars {
                units.push(reader.read_u16().await?);
            }
            // Besides the NUL, newer players wrap some text in U+FFFA and U+FFFB.
            let text: String = String::from_utf16_lossy(&units).chars().filter(|c| !matches!(c, '\0' | '\u{fffa}' | '\u{fffb}')).collect();
            Ok(Field::Text(text))
        }
        other => Err(invalid(format!("field type {other:#04x}"))),
    }
}

async fn read_number<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<u32> {
    read_field(reader).await?.number().ok_or_else(|| invalid("expected a number"))
}

pub async fn read_message<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<Message> {
    if read_number(reader).await? != MESSAGE_MAGIC {
        return Err(invalid("not a dbserver message"));
    }
    let tx = read_number(reader).await?;
    let kind = read_number(reader).await? as u16;
    let count = read_number(reader).await? as usize;
    let Field::Blob(tags) = read_field(reader).await? else {
        return Err(invalid("expected the argument types"));
    };
    if count > MAX_ARGS || tags.len() < count {
        return Err(invalid(format!("{count} arguments")));
    }
    let mut args: Vec<Field> = Vec::with_capacity(count);
    for &tag in &tags[..count] {
        // A blob of length 0 is left out of the message altogether; the
        // number before it is its length.
        if tag == 0x03 && args.last().and_then(Field::number) == Some(0) {
            args.push(Field::Blob(Vec::new()));
        } else {
            args.push(read_field(reader).await?);
        }
    }
    Ok(Message { tx, kind, args })
}

/// What a player's database says about a track. Absent fields are empty or 0.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrackMetadata {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_s: f64,
    pub bpm: f64,
    /// As the DJ's rekordbox writes keys, e.g. "Am" or "8A".
    pub key: String,
}

fn metadata_from_items(items: &[Message]) -> TrackMetadata {
    let mut metadata = TrackMetadata::default();
    for item in items {
        // Arguments: parent id, main id, label length, label, second label
        // length, second label, item type, ...
        let (Some(value), Some(label), Some(kind)) = (
            item.args.get(1).and_then(Field::number),
            item.args.get(3).and_then(Field::text),
            item.args.get(6).and_then(Field::number),
        ) else {
            continue;
        };
        // The CDJ-3000 puts something else in the upper half of the type.
        match kind & 0xffff {
            ITEM_TITLE => metadata.title = label.to_string(),
            ITEM_ARTIST => metadata.artist = label.to_string(),
            ITEM_ALBUM => metadata.album = label.to_string(),
            ITEM_KEY => metadata.key = label.to_string(),
            ITEM_DURATION => metadata.duration_s = f64::from(value),
            ITEM_TEMPO => metadata.bpm = f64::from(value) / 100.0,
            _ => {}
        }
    }
    metadata
}

async fn request(stream: &mut TcpStream, message: &Message) -> anyhow::Result<Message> {
    stream.write_all(&encode(message)).await?;
    let reply = read_message(stream).await?;
    ensure!(reply.kind == KIND_OK, "the player answered {:#06x} to request {:#06x}", reply.kind, message.kind);
    Ok(reply)
}

/// Asks the player at `finder` (its address with [`PORT_FINDER`]) for `track`.
///
/// `posing_as` is the player number we claim. Players before the CDJ-3000 only
/// answer numbers 1 to 4 that are on the network and are not themselves, which
/// is why the caller borrows another player's number. The caller sets the time limit.
pub async fn track_metadata(finder: SocketAddr, posing_as: u8, track: TrackRef) -> anyhow::Result<TrackMetadata> {
    let mut stream = TcpStream::connect(finder).await.context("connecting to the player")?;
    stream.write_all(b"\x00\x00\x00\x0fRemoteDBServer\x00").await?;
    let port = stream.read_u16().await.context("asking for the dbserver port")?;
    ensure!(port != 0 && port != 0xffff, "the player has no dbserver");

    let mut stream = TcpStream::connect((finder.ip(), port)).await.context("connecting to the dbserver")?;
    stream.set_nodelay(true)?;
    stream.write_all(&[0x11, 0, 0, 0, 1]).await?;
    let mut greeting = [0u8; 5];
    stream.read_exact(&mut greeting).await.context("greeting the dbserver")?;
    ensure!(greeting == [0x11, 0, 0, 0, 1], "unexpected greeting");

    let setup = Message { tx: TX_SETUP, kind: KIND_SETUP, args: vec![Field::Number(u32::from(posing_as))] };
    request(&mut stream, &setup).await.context("setting up")?;

    // Requesting player, menu (1 = main), slot, track type.
    let location = u32::from_be_bytes([posing_as, 1, track.slot, track.track_type]);
    let kind = if track.track_type == 1 { KIND_TRACK_METADATA } else { KIND_UNANALYSED_METADATA };
    let found = Message { tx: 1, kind, args: vec![Field::Number(location), Field::Number(track.rekordbox_id)] };
    let found = request(&mut stream, &found).await.context("requesting the track")?;
    let count = found.args.get(1).and_then(Field::number).unwrap_or(0);
    if count == 0 || count == 0xffff_ffff {
        bail!("the player does not know track {}", track.rekordbox_id);
    }
    let count = count.min(MAX_MENU_ITEMS as u32);

    // Location, offset, limit, 0, total, 0.
    let args = [location, 0, count, 0, count, 0].map(Field::Number).to_vec();
    stream.write_all(&encode(&Message { tx: 2, kind: KIND_RENDER_MENU, args })).await?;
    let mut items = Vec::new();
    loop {
        let message = read_message(&mut stream).await.context("reading the track's details")?;
        match message.kind {
            KIND_MENU_FOOTER => break,
            KIND_MENU_ITEM if items.len() < MAX_MENU_ITEMS => items.push(message),
            _ => {} // the header, or more items than we asked for
        }
    }
    Ok(metadata_from_items(&items))
}

#[cfg(test)]
pub(crate) mod testing {
    use std::net::Ipv4Addr;

    use tokio::net::TcpListener;

    use super::*;

    fn item(kind: u32, value: u32, label: &str) -> Message {
        let args = vec![
            Field::Number(0),
            Field::Number(value),
            Field::Number(label.encode_utf16().count() as u32 * 2 + 2),
            Field::Text(label.to_string()),
            Field::Number(2),
            Field::Text(String::new()),
            Field::Number(kind),
            Field::Number(0),
            Field::Number(0),
            Field::Number(0),
            Field::Number(0),
            Field::Number(0),
        ];
        Message { tx: 2, kind: KIND_MENU_ITEM, args }
    }

    /// A player's dbserver that knows one track, for one client. Returns the
    /// finder address to query and what the client claimed and asked for:
    /// (posing as, location, rekordbox id).
    pub async fn fake_player(metadata: TrackMetadata) -> (SocketAddr, tokio::task::JoinHandle<(u32, u32, u32)>) {
        let finder = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.expect("a port");
        let database = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.expect("a port");
        let finder_address = finder.local_addr().expect("an address");
        let database_port = database.local_addr().expect("an address").port();

        let served = tokio::spawn(async move {
            let (mut stream, _) = finder.accept().await.expect("a client");
            let mut query = [0u8; 19];
            stream.read_exact(&mut query).await.expect("the port query");
            assert_eq!(&query[4..], b"RemoteDBServer\0");
            stream.write_all(&database_port.to_be_bytes()).await.expect("answers");

            let (mut stream, _) = database.accept().await.expect("a client");
            let mut greeting = [0u8; 5];
            stream.read_exact(&mut greeting).await.expect("the greeting");
            stream.write_all(&greeting).await.expect("answers");

            let setup = read_message(&mut stream).await.expect("setup");
            assert_eq!((setup.tx, setup.kind), (TX_SETUP, KIND_SETUP));
            let posing_as = setup.args[0].number().expect("a player number");
            let ok = |tx, args: Vec<u32>| encode(&Message { tx, kind: KIND_OK, args: args.into_iter().map(Field::Number).collect() });
            stream.write_all(&ok(setup.tx, vec![0, 2])).await.expect("answers");

            let wanted = read_message(&mut stream).await.expect("the request");
            assert_eq!(wanted.kind, KIND_TRACK_METADATA);
            let (location, id) = (wanted.args[0].number().expect("location"), wanted.args[1].number().expect("id"));
            stream.write_all(&ok(wanted.tx, vec![u32::from(KIND_TRACK_METADATA), 6])).await.expect("answers");

            let render = read_message(&mut stream).await.expect("render");
            assert_eq!(render.kind, KIND_RENDER_MENU);
            assert_eq!(render.args, [location, 0, 6, 0, 6, 0].map(Field::Number).to_vec());
            let mut reply = encode(&Message { tx: render.tx, kind: 0x4001, args: vec![Field::Number(1), Field::Number(0)] });
            for message in [
                // The CDJ-3000 sets upper bits in the type.
                item(0x0100_0000 | ITEM_TITLE, id, &metadata.title),
                item(ITEM_ARTIST, 7, &metadata.artist),
                item(ITEM_ALBUM, 3, &metadata.album),
                item(ITEM_DURATION, metadata.duration_s as u32, ""),
                item(ITEM_TEMPO, (metadata.bpm * 100.0).round() as u32, ""),
                item(ITEM_KEY, 1, &metadata.key),
            ] {
                reply.extend(encode(&message));
            }
            reply.extend(encode(&Message { tx: render.tx, kind: KIND_MENU_FOOTER, args: Vec::new() }));
            stream.write_all(&reply).await.expect("answers");
            (posing_as, location, id)
        });
        (finder_address, served)
    }
}

#[cfg(test)]
mod tests {
    use super::testing::fake_player;
    use super::*;

    #[test]
    fn the_setup_message_has_the_documented_bytes() {
        let setup = Message { tx: TX_SETUP, kind: KIND_SETUP, args: vec![Field::Number(3)] };
        #[rustfmt::skip]
        let expected = [
            0x11, 0x87, 0x23, 0x49, 0xae, // magic
            0x11, 0xff, 0xff, 0xff, 0xfe, // transaction
            0x10, 0x00, 0x00,             // kind
            0x0f, 0x01,                   // one argument
            0x14, 0x00, 0x00, 0x00, 0x0c, 0x06, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // its type: number
            0x11, 0x00, 0x00, 0x00, 0x03, // player 3
        ];
        assert_eq!(encode(&setup), expected);
    }

    #[tokio::test]
    async fn messages_round_trip_with_every_field_type() {
        let message = Message {
            tx: 7,
            kind: KIND_MENU_ITEM,
            args: vec![Field::Number(0xdead_beef), Field::Text("Übung 🎧".to_string()), Field::Number(3), Field::Blob(vec![1, 2, 3])],
        };
        let bytes = encode(&message);
        assert_eq!(read_message(&mut bytes.as_slice()).await.expect("decodes"), message);
        // A string field: type, length in UTF-16 units with the NUL, the units.
        assert_eq!(&encode(&Message { tx: 0, kind: 0, args: vec![Field::Text("A".to_string())] })[32..], [0x26, 0, 0, 0, 2, 0, b'A', 0, 0]);
    }

    #[tokio::test]
    async fn an_empty_blob_is_absent_from_the_stream() {
        let mut bytes = encode(&Message { tx: 1, kind: KIND_OK, args: vec![Field::Number(0)] });
        bytes[14] = 2; // two arguments: the length 0, then a blob that is not there
        bytes[21] = 0x03;
        let message = read_message(&mut bytes.as_slice()).await.expect("decodes");
        assert_eq!(message.args, vec![Field::Number(0), Field::Blob(Vec::new())]);
    }

    #[tokio::test]
    async fn malformed_messages_are_errors() {
        let good = encode(&Message { tx: 1, kind: KIND_OK, args: vec![Field::Text("abc".to_string())] });
        assert!(read_message(&mut &good[..good.len() - 1]).await.is_err(), "cut short");
        let mut wrong_magic = good.clone();
        wrong_magic[1] = 0;
        assert!(read_message(&mut wrong_magic.as_slice()).await.is_err());
        let mut huge = good.clone();
        huge[33..37].copy_from_slice(&u32::MAX.to_be_bytes()); // the string's length
        assert!(read_message(&mut huge.as_slice()).await.is_err());
        let mut many = good;
        many[14] = 200; // argument count
        assert!(read_message(&mut many.as_slice()).await.is_err());
    }

    #[tokio::test]
    async fn a_track_is_looked_up_on_a_player() {
        let known = TrackMetadata {
            title: "Neon Tide".to_string(),
            artist: "Mara Voss".to_string(),
            album: "Sessions".to_string(),
            duration_s: 372.0,
            bpm: 124.5,
            key: "Am".to_string(),
        };
        let (finder, served) = fake_player(known.clone()).await;
        let track = TrackRef { source_player: 2, slot: 3, track_type: 1, rekordbox_id: 1234 };
        let found = track_metadata(finder, 4, track).await.expect("the lookup works");
        assert_eq!(found, known);
        // Posing as player 4; location = player 4, main menu, USB, rekordbox track.
        assert_eq!(served.await.expect("the fake player ran"), (4, 0x0401_0301, 1234));
    }
}
