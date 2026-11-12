//! What Mixxx's controller scripts cannot tell us, read from Mixxx's own files.
//!
//! The mapping script reports a deck's length and tempo, but not which track
//! it is. Mixxx's library database (`mixxxdb.sqlite`) knows every track's
//! length, so the track is found by its length (and tempo, if two tracks are
//! equally long). That gives title, artist, album and file, and the waveform
//! Mixxx itself analysed, stored next to the database in `analysis/<id>`.
//!
//! Everything here only reads. The database is opened read-only while Mixxx
//! runs, which SQLite allows.
//!
//! The waveform file is a Qt-compressed (`qCompress`) protocol buffer:
//!
//! ```text
//! 4 bytes   length of the data, big-endian     then a zlib stream of:
//! field 1   double   visual samples per second (441 for the detail waveform)
//! field 4   message  the three bands:
//!   field 1, 2, 3   message  low, mid, high:
//!     field 1   repeated integer, 0 to 255, left and right interleaved
//! ```

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use flate2::read::ZlibDecoder;
use rusqlite::{Connection, OpenFlags};

use crate::waveform::synth::build_overview;
use crate::waveform::{Waveform, BYTES_PER_FRAME, DETAIL_FPS};

const DATABASE: &str = "mixxxdb.sqlite";
/// The script sends the length in whole milliseconds.
const LENGTH_TOLERANCE_S: f64 = 0.002;
/// And the tempo in hundredths of a BPM.
const TEMPO_TOLERANCE: f64 = 0.011;
/// Mixxx's detail waveform is a few megabytes; anything far beyond is not one.
const MAX_WAVEFORM_BYTES: u64 = 64 * 1024 * 1024;
/// Mixxx analyses left and right separately.
const CHANNELS: usize = 2;

/// A track as Mixxx's library has it.
#[derive(Debug, Clone, PartialEq)]
pub struct LibraryTrack {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub file_path: PathBuf,
    /// `None` until Mixxx has analysed the track (it does so on first load).
    pub waveform: Option<Waveform>,
}

/// The folder that holds Mixxx's database on this computer, if Mixxx has one.
pub fn default_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut candidates = Vec::new();
    if let Some(home) = &home {
        // macOS: the build from mixxx.org is sandboxed, others are not.
        candidates.push(home.join("Library/Containers/org.mixxx.mixxx/Data/Library/Application Support/Mixxx"));
        candidates.push(home.join("Library/Application Support/Mixxx"));
        candidates.push(home.join(".mixxx")); // Linux
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local).join("Mixxx")); // Windows
    }
    candidates.into_iter().find(|dir| dir.join(DATABASE).is_file())
}

/// Finds the track that is `duration_s` long. With several of that length, the
/// one whose tempo is `bpm`. `None` if there is no such track or no single one.
pub fn find_track(dir: &Path, duration_s: f64, bpm: f64) -> anyhow::Result<Option<LibraryTrack>> {
    let database = Connection::open_with_flags(dir.join(DATABASE), OpenFlags::SQLITE_OPEN_READ_ONLY)
        .context("opening Mixxx's library")?;
    database.busy_timeout(Duration::from_millis(500))?;

    struct Row {
        title: Option<String>,
        artist: Option<String>,
        album: Option<String>,
        location: String,
        location_id: i64,
        bpm: Option<f64>,
    }
    let mut query = database.prepare(
        "SELECT l.title, l.artist, l.album, t.location, t.id, l.bpm
         FROM library l JOIN track_locations t ON t.id = l.location
         WHERE l.mixxx_deleted = 0 AND ABS(l.duration - ?1) < ?2",
    )?;
    let mut rows: Vec<Row> = query
        .query_map((duration_s, LENGTH_TOLERANCE_S), |row| {
            Ok(Row {
                title: row.get(0)?,
                artist: row.get(1)?,
                album: row.get(2)?,
                location: row.get(3)?,
                location_id: row.get(4)?,
                bpm: row.get(5)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    if rows.len() > 1 {
        rows.retain(|row| row.bpm.is_some_and(|tempo| (tempo - bpm).abs() < TEMPO_TOLERANCE));
    }
    let [row] = rows.as_slice() else { return Ok(None) };

    // Type 1 is the detail waveform, type 2 the overview. The newest wins.
    let analysis: Option<i64> = database
        .query_row(
            "SELECT id FROM track_analysis WHERE track_id = ?1 AND CAST(type AS INTEGER) = 1 ORDER BY id DESC LIMIT 1",
            [row.location_id],
            |found| found.get(0),
        )
        .ok();
    let waveform = analysis.and_then(|id| {
        let file = dir.join("analysis").join(id.to_string());
        match std::fs::read(&file).map_err(anyhow::Error::from).and_then(|bytes| decode_waveform(&bytes)) {
            Ok(waveform) => Some(waveform),
            Err(error) => {
                tracing::warn!("Mixxx: cannot use the waveform in {}: {error:#}", file.display());
                None
            }
        }
    });

    let file_path = PathBuf::from(&row.location);
    let title = row.title.clone().filter(|title| !title.trim().is_empty()).unwrap_or_else(|| {
        file_path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default()
    });
    Ok(Some(LibraryTrack {
        title,
        artist: row.artist.clone().unwrap_or_default(),
        album: row.album.clone().unwrap_or_default(),
        file_path,
        waveform,
    }))
}

/// One protocol-buffer field: its number, and its value by wire type.
enum Field<'a> {
    Integer(u64),
    Double(f64),
    Bytes(&'a [u8]),
    /// A 32-bit value, which nothing here reads.
    Other,
}

fn read_varint(bytes: &[u8], at: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *bytes.get(*at)?;
        *at += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return Some(value);
        }
    }
    None
}

/// The fields of a message, or `None` if it is cut short or malformed.
fn fields(bytes: &[u8]) -> Option<Vec<(u64, Field<'_>)>> {
    let mut found = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let key = read_varint(bytes, &mut at)?;
        let field = match key & 7 {
            0 => Field::Integer(read_varint(bytes, &mut at)?),
            1 => {
                let raw: [u8; 8] = bytes.get(at..at + 8)?.try_into().ok()?;
                at += 8;
                Field::Double(f64::from_le_bytes(raw))
            }
            2 => {
                let length = usize::try_from(read_varint(bytes, &mut at)?).ok()?;
                let end = at.checked_add(length)?;
                let value = bytes.get(at..end)?;
                at = end;
                Field::Bytes(value)
            }
            5 => {
                bytes.get(at..at + 4)?;
                at += 4;
                Field::Other
            }
            _ => return None,
        };
        found.push((key >> 3, field));
    }
    Some(found)
}

/// One band's values. Written one by one by Mixxx; a packed list is read too.
fn band(message: &[u8]) -> Option<Vec<u8>> {
    let mut values = Vec::new();
    for (number, field) in fields(message)? {
        match (number, field) {
            (1, Field::Integer(value)) => values.push(value.min(255) as u8),
            (1, Field::Bytes(packed)) => {
                let mut at = 0;
                while at < packed.len() {
                    values.push(read_varint(packed, &mut at)?.min(255) as u8);
                }
            }
            _ => {}
        }
    }
    Some(values)
}

/// Turns one of Mixxx's `analysis` files into our waveform: 150 frames a
/// second, each the loudest of the visual samples (and of left and right) it
/// covers, scaled so the loudest value of the track is 255.
pub fn decode_waveform(file: &[u8]) -> anyhow::Result<Waveform> {
    let compressed = file.get(4..).context("too short")?;
    let mut data = Vec::new();
    ZlibDecoder::new(compressed).take(MAX_WAVEFORM_BYTES).read_to_end(&mut data).context("not compressed data")?;

    let mut rate = 0.0;
    let mut bands: Option<[Vec<u8>; 3]> = None;
    for (number, field) in fields(&data).context("not a Mixxx waveform")? {
        match (number, field) {
            (1, Field::Double(visual_rate)) => rate = visual_rate,
            (4, Field::Bytes(filtered)) => {
                let mut found: [Option<Vec<u8>>; 3] = [None, None, None];
                for (number, field) in fields(filtered).context("not a Mixxx waveform")? {
                    if let (1..=3, Field::Bytes(message)) = (number, field) {
                        found[number as usize - 1] = Some(band(message).context("not a Mixxx waveform")?);
                    }
                }
                if let [Some(low), Some(mid), Some(high)] = found {
                    bands = Some([low, mid, high]);
                }
            }
            _ => {}
        }
    }
    let bands = bands.context("no low, mid and high bands")?;
    anyhow::ensure!((1.0..=100_000.0).contains(&rate), "visual sample rate {rate}");
    let visual_frames = bands.iter().map(Vec::len).min().unwrap_or(0) / CHANNELS;
    anyhow::ensure!(visual_frames > 0, "empty");

    let per_frame = rate / f64::from(DETAIL_FPS);
    let frames = ((visual_frames as f64 / per_frame).ceil() as usize).max(1);
    let mut detail = vec![0u8; frames * BYTES_PER_FRAME];
    for frame in 0..frames {
        let first = ((frame as f64 * per_frame) as usize).min(visual_frames - 1);
        let end = (((frame + 1) as f64 * per_frame) as usize).clamp(first + 1, visual_frames);
        for (index, values) in bands.iter().enumerate() {
            let loudest = values[first * CHANNELS..end * CHANNELS].iter().copied().max().unwrap_or(0);
            detail[frame * BYTES_PER_FRAME + index] = loudest;
        }
    }
    let peak = detail.iter().copied().max().unwrap_or(0);
    if peak > 0 && peak < 255 {
        for value in &mut detail {
            *value = (u32::from(*value) * 255 / u32::from(peak)) as u8;
        }
    }
    let overview = build_overview(&detail);
    Ok(Waveform { frames: frames as u32, detail, overview })
}

#[cfg(test)]
pub(crate) mod testing {
    use std::io::Write;
    use std::sync::atomic::{AtomicU32, Ordering};

    use flate2::write::ZlibEncoder;
    use flate2::Compression;

    use super::*;

    fn varint(mut value: u64, out: &mut Vec<u8>) {
        while value >= 0x80 {
            out.push(value as u8 | 0x80);
            value >>= 7;
        }
        out.push(value as u8);
    }

    fn message(number: u64, body: &[u8], out: &mut Vec<u8>) {
        varint(number << 3 | 2, out);
        varint(body.len() as u64, out);
        out.extend_from_slice(body);
    }

    /// An `analysis` file as Mixxx writes it, from interleaved stereo bands.
    pub fn analysis_file(rate: f64, low: &[u8], mid: &[u8], high: &[u8]) -> Vec<u8> {
        let mut filtered = Vec::new();
        for (number, values) in [(1, low), (2, mid), (3, high)] {
            let mut signal = Vec::new();
            for &value in values {
                varint(1 << 3, &mut signal);
                varint(u64::from(value), &mut signal);
            }
            message(number, &signal, &mut filtered);
        }
        // The cut-off frequencies, which we skip.
        filtered.extend([4 << 3 | 1]);
        filtered.extend(600f64.to_le_bytes());

        let mut data = vec![1 << 3 | 1];
        data.extend(rate.to_le_bytes());
        data.extend([2 << 3 | 1]);
        data.extend(100f64.to_le_bytes());
        message(3, &[1 << 3, 7], &mut data); // the unfiltered signal, unused
        message(4, &filtered, &mut data);

        let mut file = (data.len() as u32).to_be_bytes().to_vec();
        let mut encoder = ZlibEncoder::new(&mut file, Compression::default());
        encoder.write_all(&data).expect("compresses");
        encoder.finish().expect("compresses");
        file
    }

    /// A Mixxx settings folder with the parts of the schema we read.
    pub fn library() -> (PathBuf, Connection) {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let name = format!("face2face-mixxx-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
        let dir = std::env::temp_dir().join(name);
        std::fs::create_dir_all(dir.join("analysis")).expect("a folder");
        let database = Connection::open(dir.join(DATABASE)).expect("a database");
        database
            .execute_batch(
                "CREATE TABLE track_locations (id INTEGER PRIMARY KEY, location varchar(512));
                 CREATE TABLE library (id INTEGER PRIMARY KEY, artist, title, album, location INTEGER,
                                       duration FLOAT, bpm FLOAT, mixxx_deleted INTEGER);
                 CREATE TABLE track_analysis (id INTEGER PRIMARY KEY, track_id INTEGER, type varchar(512));",
            )
            .expect("tables");
        (dir, database)
    }

    pub fn add_track(database: &Connection, id: i64, title: Option<&str>, file: &str, duration: f64, bpm: f64) {
        database.execute("INSERT INTO track_locations VALUES (?1, ?2)", (id + 100, file)).expect("inserts");
        database
            .execute("INSERT INTO library VALUES (?1, 'Mara Voss', ?2, 'Sessions', ?3, ?4, ?5, 0)", (id, title, id + 100, duration, bpm))
            .expect("inserts");
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{add_track, analysis_file, library};
    use super::*;

    #[test]
    fn a_waveform_is_resampled_to_150_frames_a_second_and_scaled() {
        // 300 visual samples a second: two per frame. Three seconds, stereo.
        let low: Vec<u8> = (0..900).flat_map(|i| [(i % 100) as u8, 0]).collect();
        let mid: Vec<u8> = (0..900).flat_map(|i| [0, if i == 451 { 50 } else { 10 }]).collect();
        let high = vec![1u8; 1800];
        let waveform = decode_waveform(&analysis_file(300.0, &low, &mid, &high)).expect("decodes");

        assert_eq!(waveform.frames, 450);
        assert_eq!(waveform.detail.len(), 450 * 3);
        assert_eq!(waveform.overview.len(), 3600);
        // Loudest value anywhere is 99 (low band): it becomes 255, the rest in proportion.
        let frame = |index: usize| &waveform.detail[index * 3..index * 3 + 3];
        // 1, 10 and 1, each times 255 / 99 and rounded down.
        assert_eq!(frame(0), [2, 25, 2], "visual samples 0 and 1: the louder one, left or right");
        assert_eq!(frame(49)[0], 255, "visual samples 98 and 99");
        assert_eq!(frame(225)[1], 128, "the one loud mid sample (50), in the right channel");
    }

    #[test]
    fn a_waveform_at_mixxxs_own_rate_has_the_tracks_length() {
        // 441 visual samples a second for 10 s.
        let band = vec![200u8; 4410 * 2];
        let waveform = decode_waveform(&analysis_file(441.0, &band, &band, &band)).expect("decodes");
        assert_eq!(waveform.frames, 1500);
        assert!(waveform.detail.iter().all(|&value| value == 255));
    }

    #[test]
    fn files_that_are_not_waveforms_are_refused() {
        let good = analysis_file(441.0, &[1, 2], &[3, 4], &[5, 6]);
        assert!(decode_waveform(&good).is_ok());
        assert!(decode_waveform(b"").is_err());
        assert!(decode_waveform(b"\0\0\0\x04junk").is_err());
        assert!(decode_waveform(&good[..good.len() / 2]).is_err(), "cut short");
        assert!(decode_waveform(&analysis_file(0.0, &[1, 2], &[3, 4], &[5, 6])).is_err(), "no sample rate");
        assert!(decode_waveform(&analysis_file(441.0, &[], &[], &[])).is_err(), "empty");
    }

    #[test]
    fn a_track_is_found_by_its_length_with_its_waveform() {
        let (dir, database) = library();
        add_track(&database, 1, Some("Neon Tide"), "/Music/Mara Voss - Neon Tide.flac", 372.4, 124.0);
        add_track(&database, 2, Some("Glass Garden"), "/Music/glass.mp3", 331.25, 126.0);
        database.execute("INSERT INTO track_analysis VALUES (7, 101, '2'), (8, 101, '1')", []).expect("inserts");
        let band = vec![9u8; 882];
        std::fs::write(dir.join("analysis/8"), analysis_file(441.0, &band, &band, &band)).expect("writes");

        let found = find_track(&dir, 372.4004, 0.0).expect("reads").expect("found");
        assert_eq!((found.title.as_str(), found.artist.as_str(), found.album.as_str()), ("Neon Tide", "Mara Voss", "Sessions"));
        assert_eq!(found.file_path, PathBuf::from("/Music/Mara Voss - Neon Tide.flac"));
        assert_eq!(found.waveform.map(|waveform| waveform.frames), Some(150));

        // Not analysed yet: found, without a waveform.
        let other = find_track(&dir, 331.25, 126.0).expect("reads").expect("found");
        assert_eq!((other.title.as_str(), other.waveform), ("Glass Garden", None));
        assert_eq!(find_track(&dir, 100.0, 124.0).expect("reads"), None);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn equal_lengths_are_told_apart_by_tempo_or_not_at_all() {
        let (dir, database) = library();
        add_track(&database, 1, Some("Original"), "/a.flac", 300.0, 124.0);
        add_track(&database, 2, Some("Remix"), "/b.flac", 300.0, 128.0);
        add_track(&database, 3, None, "/Music/untagged file.wav", 200.0, 0.0);
        let title = |bpm: f64| find_track(&dir, 300.0, bpm).expect("reads").map(|track| track.title);
        assert_eq!(title(128.0).as_deref(), Some("Remix"));
        assert_eq!(title(124.004).as_deref(), Some("Original"));
        assert_eq!(title(0.0), None, "two candidates and no tempo to choose by");
        // A track without tags is named after its file.
        assert_eq!(find_track(&dir, 200.0, 0.0).expect("reads").map(|track| track.title).as_deref(), Some("untagged file"));
        // A deleted track is not a candidate.
        database.execute("UPDATE library SET mixxx_deleted = 1 WHERE id = 1", []).expect("updates");
        assert_eq!(title(0.0).as_deref(), Some("Remix"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_missing_library_is_an_error_not_a_panic() {
        assert!(find_track(Path::new("/nonexistent/face2face"), 1.0, 1.0).is_err());
    }
}
