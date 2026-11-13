//! Asks Mixxx's library what the Mixxx reader would ask it, to see why a deck
//! shows no title or waveform:
//!
//! ```sh
//! cargo run --example mixxx_lookup -- 257.561 123 19
//! ```
//!
//! The arguments are what the deck reports: length in seconds, the track's
//! tempo (0 if unknown) and Mixxx's key number (0 if unknown; 1 to 12 are C to
//! B major, 13 to 24 C to B minor).

use face2face_core::sources::mixxx_library::{default_dir, find_track};

fn main() -> anyhow::Result<()> {
    let number = |index: usize| std::env::args().nth(index).and_then(|text| text.parse::<f64>().ok());
    let Some(duration_s) = number(1) else {
        anyhow::bail!("usage: mixxx_lookup <length in seconds> [tempo] [key number]");
    };
    let (bpm, key) = (number(2).unwrap_or(0.0), number(3).unwrap_or(0.0) as u8);
    let Some(dir) = default_dir() else {
        anyhow::bail!("Mixxx's library was not found on this computer");
    };
    println!("Library: {}", dir.display());
    match find_track(&dir, duration_s, bpm, key)? {
        Some(track) => {
            println!("Track:    {} - {}", track.artist, track.title);
            println!("File:     {}", track.file_path.display());
            match track.waveform {
                Some(waveform) => println!("Waveform: {} frames ({:.1} s)", waveform.frames, f64::from(waveform.frames) / 150.0),
                None => println!("Waveform: none yet (Mixxx has not analysed the track)"),
            }
        }
        None => println!("No track of that length, tempo and key, or more than one."),
    }
    Ok(())
}
