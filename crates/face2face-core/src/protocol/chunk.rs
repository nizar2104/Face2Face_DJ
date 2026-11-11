//! Binary frame `waveform_chunk` (layout in `docs/PROTOCOL.md`, section 4).
//!
//! ```text
//! 0   4  magic "TDWF"
//! 4   1  version (1)
//! 5   1  kind: 0 = overview, 1 = detail
//! 6   1  id length N (1..=64)
//! 7   1  reserved (0)
//! 8   4  offset  u32 LE  byte offset of this payload within the kind's data
//! 12  4  length  u32 LE  payload bytes in this frame
//! 16  4  total   u32 LE  total bytes of the kind's data
//! 20  N  waveform id, printable ASCII
//! 20+N   payload
//! ```

pub const WAVEFORM_MAGIC: [u8; 4] = *b"TDWF";
pub const CHUNK_VERSION: u8 = 1;
pub const HEADER_LEN: usize = 20;
pub const MAX_ID_LEN: usize = 64;
/// 16 MiB of detail data is about 10 hours of audio at 150 fps x 3 bytes.
pub const MAX_WAVEFORM_BYTES: u32 = 16 * 1024 * 1024;
pub const CHUNK_PAYLOAD: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaveformKind {
    Overview = 0,
    Detail = 1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaveformChunk {
    pub waveform_id: String,
    pub kind: WaveformKind,
    pub offset: u32,
    pub total: u32,
    pub payload: Vec<u8>,
}

/// Waveform ids are 1 to 64 printable ASCII characters without spaces.
pub fn valid_waveform_id(id: &str) -> bool {
    (1..=MAX_ID_LEN).contains(&id.len()) && id.bytes().all(|b| (0x21..=0x7e).contains(&b))
}

/// Encodes one frame. `id` must satisfy [`valid_waveform_id`].
pub fn encode_chunk(id: &str, kind: WaveformKind, offset: u32, total: u32, payload: &[u8]) -> Vec<u8> {
    debug_assert!(valid_waveform_id(id), "invalid waveform id");
    let mut out = Vec::with_capacity(HEADER_LEN + id.len() + payload.len());
    out.extend_from_slice(&WAVEFORM_MAGIC);
    out.push(CHUNK_VERSION);
    out.push(kind as u8);
    out.push(id.len() as u8);
    out.push(0);
    out.extend_from_slice(&offset.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(id.as_bytes());
    out.extend_from_slice(payload);
    out
}

/// Decodes one frame, or `None` if it is not a valid waveform chunk.
pub fn decode_chunk(bytes: &[u8]) -> Option<WaveformChunk> {
    if bytes.len() < HEADER_LEN || bytes[..4] != WAVEFORM_MAGIC || bytes[4] != CHUNK_VERSION {
        return None;
    }
    let kind = match bytes[5] {
        0 => WaveformKind::Overview,
        1 => WaveformKind::Detail,
        _ => return None,
    };
    let id_len = usize::from(bytes[6]);
    if id_len == 0 || id_len > MAX_ID_LEN {
        return None;
    }
    let u32_at = |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    let offset = u32_at(8);
    let length = u32_at(12);
    let total = u32_at(16);
    if total > MAX_WAVEFORM_BYTES || offset.checked_add(length)? > total {
        return None;
    }
    if bytes.len() != HEADER_LEN + id_len + length as usize {
        return None;
    }
    let id = std::str::from_utf8(&bytes[HEADER_LEN..HEADER_LEN + id_len]).ok()?;
    if !valid_waveform_id(id) {
        return None;
    }
    Some(WaveformChunk {
        waveform_id: id.to_string(),
        kind,
        offset,
        total,
        payload: bytes[HEADER_LEN + id_len..].to_vec(),
    })
}

/// Splits one kind's data into wire frames, in order.
pub fn chunk_waveform(id: &str, kind: WaveformKind, data: &[u8]) -> Vec<Vec<u8>> {
    data.chunks(CHUNK_PAYLOAD)
        .enumerate()
        .map(|(i, part)| encode_chunk(id, kind, (i * CHUNK_PAYLOAD) as u32, data.len() as u32, part))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE_HEX: &str = include_str!("../../../../test-assets/protocol/waveform_chunk.hex");

    fn fixture() -> Vec<u8> {
        let hex: String = FIXTURE_HEX.chars().filter(|c| !c.is_whitespace()).collect();
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("fixture is hex"))
            .collect()
    }

    #[test]
    fn the_shared_fixture_decodes_and_re_encodes_byte_for_byte() {
        let bytes = fixture();
        let chunk = decode_chunk(&bytes).expect("fixture decodes");
        assert_eq!(chunk.waveform_id, "wf1");
        assert_eq!(chunk.kind, WaveformKind::Detail);
        assert_eq!(chunk.offset, 6);
        assert_eq!(chunk.total, 12);
        assert_eq!(chunk.payload, vec![1, 2, 3, 4, 5, 6]);
        let again = encode_chunk(&chunk.waveform_id, chunk.kind, chunk.offset, chunk.total, &chunk.payload);
        assert_eq!(again, bytes);
    }

    #[test]
    fn corrupt_frames_are_rejected() {
        let good = encode_chunk("abc", WaveformKind::Detail, 0, 6, &[1, 2, 3]);
        assert!(decode_chunk(&good).is_some());
        assert!(decode_chunk(&[]).is_none());
        assert!(decode_chunk(&good[..10]).is_none());
        assert!(decode_chunk(&good[..good.len() - 1]).is_none());
        // (byte index, bad value): magic, version, kind, id length, offset, total, id character
        for (index, value) in [(0, 0), (4, 2), (5, 7), (6, 0), (8, 5), (19, 0xff), (20, 0x20)] {
            let mut bad = good.clone();
            bad[index] = value;
            assert!(decode_chunk(&bad).is_none(), "byte {index} = {value}");
        }
    }

    #[test]
    fn chunking_covers_the_data_exactly_and_in_order() {
        let data: Vec<u8> = (0..CHUNK_PAYLOAD * 2 + 100).map(|i| (i % 251) as u8).collect();
        let frames = chunk_waveform("id", WaveformKind::Detail, &data);
        assert_eq!(frames.len(), 3);
        let mut rebuilt = Vec::new();
        for frame in &frames {
            let chunk = decode_chunk(frame).expect("own frames decode");
            assert_eq!(chunk.offset as usize, rebuilt.len());
            assert_eq!(chunk.total as usize, data.len());
            rebuilt.extend_from_slice(&chunk.payload);
        }
        assert_eq!(rebuilt, data);
    }

    #[test]
    fn waveform_id_rules() {
        assert!(valid_waveform_id("mockwf-1a2b3c4d"));
        assert!(!valid_waveform_id(""));
        assert!(!valid_waveform_id("has space"));
        assert!(!valid_waveform_id(&"x".repeat(65)));
        assert!(!valid_waveform_id("caf\u{e9}"));
    }
}
