//! Just enough Open Sound Control 1.0 to read what rkbx_link sends: messages
//! (and bundles of them) with float, integer and string arguments.

#[derive(Debug, Clone, PartialEq)]
pub enum OscArg {
    Float(f32),
    Int(i32),
    Text(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct OscMessage {
    pub address: String,
    pub args: Vec<OscArg>,
}

/// Bundles may nest; real senders nest once at most.
const MAX_BUNDLE_DEPTH: u8 = 4;

/// The messages in one UDP datagram. A malformed datagram, or one with
/// argument types other than the three above, yields what could be read
/// before the problem.
pub fn decode(packet: &[u8]) -> Vec<OscMessage> {
    let mut messages = Vec::new();
    decode_into(packet, MAX_BUNDLE_DEPTH, &mut messages);
    messages
}

fn decode_into(packet: &[u8], depth: u8, messages: &mut Vec<OscMessage>) {
    if packet.starts_with(b"#bundle\0") {
        let mut rest = &packet[16.min(packet.len())..]; // after the name and the time tag
        while depth > 0 && rest.len() >= 4 {
            let size = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
            let Some(element) = rest.get(4..4 + size) else { return };
            decode_into(element, depth - 1, messages);
            rest = &rest[4 + size..];
        }
    } else if let Some(message) = decode_message(packet) {
        messages.push(message);
    }
}

/// A NUL-terminated string padded to a multiple of 4 bytes, and what follows it.
fn take_string(bytes: &[u8]) -> Option<(&str, &[u8])> {
    let end = bytes.iter().position(|&byte| byte == 0)?;
    let text = std::str::from_utf8(&bytes[..end]).ok()?;
    Some((text, bytes.get((end + 4) & !3..)?))
}

fn decode_message(packet: &[u8]) -> Option<OscMessage> {
    let (address, rest) = take_string(packet)?;
    if !address.starts_with('/') {
        return None;
    }
    let (tags, mut rest) = take_string(rest)?;
    let mut args = Vec::new();
    for tag in tags.strip_prefix(',')?.chars() {
        match tag {
            'f' | 'i' => {
                let bytes: [u8; 4] = rest.get(..4)?.try_into().ok()?;
                args.push(if tag == 'f' { OscArg::Float(f32::from_be_bytes(bytes)) } else { OscArg::Int(i32::from_be_bytes(bytes)) });
                rest = &rest[4..];
            }
            's' => {
                let (text, after) = take_string(rest)?;
                args.push(OscArg::Text(text.to_string()));
                rest = after;
            }
            _ => return None,
        }
    }
    Some(OscMessage { address: address.to_string(), args })
}

#[cfg(test)]
pub(crate) mod testing {
    /// Encodes a message the way the `rosc` crate (which rkbx_link uses) does.
    pub fn encode(address: &str, tags: &str, args: &[&[u8]]) -> Vec<u8> {
        fn padded(text: &str) -> Vec<u8> {
            let mut bytes = text.as_bytes().to_vec();
            bytes.resize((bytes.len() + 4) & !3, 0);
            bytes
        }
        let mut packet = padded(address);
        packet.extend(padded(&format!(",{tags}")));
        for arg in args {
            packet.extend_from_slice(arg);
        }
        packet
    }

    pub fn float(address: &str, value: f32) -> Vec<u8> {
        encode(address, "f", &[&value.to_be_bytes()])
    }

    pub fn text(address: &str, value: &str) -> Vec<u8> {
        let mut bytes = value.as_bytes().to_vec();
        bytes.resize((bytes.len() + 4) & !3, 0);
        encode(address, "s", &[&bytes])
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{encode, float, text};
    use super::*;

    #[test]
    fn floats_strings_and_integers_decode() {
        assert_eq!(
            decode(&float("/1/time", 64.5)),
            vec![OscMessage { address: "/1/time".to_string(), args: vec![OscArg::Float(64.5)] }]
        );
        // A string whose length is a multiple of 4 still gets four NULs.
        assert_eq!(
            decode(&text("/0/track/title", "Neon")),
            vec![OscMessage { address: "/0/track/title".to_string(), args: vec![OscArg::Text("Neon".to_string())] }]
        );
        let mixed = encode("/x", "if", &[&7i32.to_be_bytes(), &1.5f32.to_be_bytes()]);
        assert_eq!(decode(&mixed)[0].args, vec![OscArg::Int(7), OscArg::Float(1.5)]);
    }

    #[test]
    fn a_known_byte_sequence_decodes() {
        // "/a" ",f" 1.0, written out by hand from the OSC 1.0 specification.
        let packet = [b'/', b'a', 0, 0, b',', b'f', 0, 0, 0x3f, 0x80, 0, 0];
        assert_eq!(decode(&packet), vec![OscMessage { address: "/a".to_string(), args: vec![OscArg::Float(1.0)] }]);
    }

    #[test]
    fn bundles_are_unpacked() {
        let mut bundle = b"#bundle\0".to_vec();
        bundle.extend([0u8; 8]);
        for element in [float("/1/time", 1.0), text("/1/track/title", "Two")] {
            bundle.extend((element.len() as u32).to_be_bytes());
            bundle.extend(element);
        }
        let messages = decode(&bundle);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].args, vec![OscArg::Text("Two".to_string())]);
    }

    #[test]
    fn garbage_yields_nothing_and_does_not_panic() {
        for packet in [&b""[..], b"/", b"/abc", b"/a\0\0,f\0\0\x01", b"hello\0\0\0,f\0\0", b"#bundle\0", b"#bundle\0\0\0\0\0\0\0\0\0\xff\xff\xff\xff"] {
            assert!(decode(packet).is_empty(), "{packet:?}");
        }
        let unknown_type = encode("/a", "b", &[&[0, 0, 0, 0]]);
        assert!(decode(&unknown_type).is_empty());
    }
}
