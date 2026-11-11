//! Musical keys. Every DJ program writes them differently ("8A", "1m", "Am",
//! "A minor", a number); the protocol wants Camelot and musical notation.

use crate::protocol::KeyInfo;

const MAJOR: [&str; 12] = ["C", "Db", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"];
const MINOR: [&str; 12] = ["Cm", "C#m", "Dm", "Ebm", "Em", "Fm", "F#m", "Gm", "G#m", "Am", "Bbm", "Bm"];

/// The key with tonic `pitch_class` (0 = C, 1 = C#, ... 11 = B).
pub fn key_from_pitch_class(pitch_class: u8, minor: bool) -> KeyInfo {
    let tonic = usize::from(pitch_class % 12);
    // Camelot numbers walk the circle of fifths from C major = 8B; a minor key
    // shares the number of its relative major, three semitones up.
    let relative_major = if minor { (tonic + 3) % 12 } else { tonic };
    let number = (relative_major * 7 + 7) % 12 + 1;
    KeyInfo {
        camelot: format!("{number}{}", if minor { 'A' } else { 'B' }),
        musical: (if minor { MINOR } else { MAJOR })[tonic].to_string(),
    }
}

/// Camelot number (1 to 12) and letter back to a key.
fn from_camelot(number: usize, minor: bool) -> KeyInfo {
    let relative_major = (number + 4) * 7 % 12; // inverse of `* 7 + 7` above
    let tonic = if minor { (relative_major + 9) % 12 } else { relative_major };
    key_from_pitch_class(tonic as u8, minor)
}

/// Reads a key as DJ software displays it: Camelot ("8A"), Open Key ("1m",
/// "1d"), or a note name ("Am", "F#", "Bb min", "A minor"). `None` for
/// anything else, including an empty string.
pub fn parse_key(text: &str) -> Option<KeyInfo> {
    let text = text.trim();
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 {
        let number: usize = text[..digits].parse().ok().filter(|n| (1..=12).contains(n))?;
        return match text[digits..].trim() {
            "A" | "a" => Some(from_camelot(number, true)),
            "B" | "b" => Some(from_camelot(number, false)),
            // Open Key 1 is Camelot 8.
            "m" | "M" => Some(from_camelot((number + 6) % 12 + 1, true)),
            "d" | "D" => Some(from_camelot((number + 6) % 12 + 1, false)),
            _ => None,
        };
    }

    let mut chars = text.chars();
    let natural: i32 = match chars.next()?.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let rest = chars.as_str();
    let (shift, rest) = match rest.chars().next() {
        Some('#' | '♯') => (1, &rest[rest.chars().next().map_or(0, char::len_utf8)..]),
        Some('b' | '♭') => (-1, &rest[rest.chars().next().map_or(0, char::len_utf8)..]),
        _ => (0, rest),
    };
    let minor = match rest.trim().to_ascii_lowercase().as_str() {
        "" | "maj" | "major" => false,
        "m" | "min" | "minor" => true,
        _ => return None,
    };
    Some(key_from_pitch_class((natural + shift).rem_euclid(12) as u8, minor))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(key: Option<KeyInfo>) -> Option<(String, String)> {
        key.map(|key| (key.camelot, key.musical))
    }

    fn expect(text: &str, camelot: &str, musical: &str) {
        assert_eq!(pair(parse_key(text)), Some((camelot.to_string(), musical.to_string())), "parsing {text:?}");
    }

    #[test]
    fn the_camelot_wheel_is_right_all_the_way_round() {
        let majors = ["8B", "3B", "10B", "5B", "12B", "7B", "2B", "9B", "4B", "11B", "6B", "1B"];
        let minors = ["5A", "12A", "7A", "2A", "9A", "4A", "11A", "6A", "1A", "8A", "3A", "10A"];
        for tonic in 0..12u8 {
            assert_eq!(key_from_pitch_class(tonic, false).camelot, majors[usize::from(tonic)]);
            assert_eq!(key_from_pitch_class(tonic, true).camelot, minors[usize::from(tonic)]);
        }
    }

    #[test]
    fn every_notation_gives_the_same_key() {
        for text in ["8A", "08A", "8a", "1m", "Am", "A min", "A minor", "am"] {
            expect(text, "8A", "Am");
        }
        for text in ["8B", "1d", "C", "C maj", "C major"] {
            expect(text, "8B", "C");
        }
        expect("F#m", "11A", "F#m");
        expect("Gbm", "11A", "F#m");
        expect("Bb", "6B", "Bb");
        expect("B♭m", "3A", "Bbm");
        expect("12m", "7A", "Dm");
        expect("6d", "1B", "B");
        expect("Cb", "1B", "B");
    }

    #[test]
    fn camelot_round_trips() {
        for number in 1..=12 {
            for letter in ['A', 'B'] {
                let text = format!("{number}{letter}");
                assert_eq!(parse_key(&text).map(|key| key.camelot), Some(text));
            }
        }
    }

    #[test]
    fn nonsense_is_not_a_key() {
        for text in ["", "  ", "13A", "0B", "8C", "H", "Amoll", "none", "8"] {
            assert_eq!(parse_key(text), None, "parsing {text:?}");
        }
    }
}
