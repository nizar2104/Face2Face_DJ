//! Face2Face DJ wire protocol v1. Source of truth: `docs/PROTOCOL.md`.
//! The TypeScript mirror is `app/src/protocol/messages.ts`; both are tested
//! against the shared fixtures in `test-assets/protocol/`.

pub mod chunk;

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;
pub const DEFAULT_PORT: u16 = 47801;
/// Milestone 0, two linked instances on one computer: the first one listens on
/// the first port, the second on the second, and each views the other. With
/// two computers (Milestone 1) both listen on [`DEFAULT_PORT`].
pub const LINK_PORTS: [u16; 2] = [DEFAULT_PORT, DEFAULT_PORT + 1];
pub const MAX_DECKS: u8 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Sender,
    Viewer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolveStatus {
    Ok,
    Ambiguous,
    FileNotFound,
    NotLocal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaveformStatus {
    Ready,
    Analysing,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyInfo {
    pub camelot: String,
    pub musical: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackInfo {
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_s: f64,
    pub bpm_original: f64,
    pub key: Option<KeyInfo>,
    pub file_name: String,
    pub local: bool,
    pub resolve_status: ResolveStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaveformInfo {
    pub waveform_id: String,
    pub detail_fps: u32,
    pub frames: u32,
    pub status: WaveformStatus,
}

/// One JSON text frame. The `type` field selects the variant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    Hello {
        protocol: u32,
        device_id: String,
        name: String,
        role: Role,
        app_version: String,
        os: String,
        /// Viewer to Sender: the Sender's pairing code, when it has one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pair_code: Option<String>,
    },
    SourceStatus {
        software: String,
        software_version: String,
        connected: bool,
        decks: u8,
        reader: String,
        reader_status: String,
    },
    DeckLoaded {
        deck: u8,
        track: TrackInfo,
        waveform: Option<WaveformInfo>,
    },
    DeckState {
        deck: u8,
        /// Sender monotonic clock, seconds.
        ts: f64,
        pos_s: f64,
        bpm_current: f64,
        playing: bool,
        is_master: bool,
        on_air: bool,
    },
    DeckUnloaded {
        deck: u8,
    },
    WaveformStatus {
        waveform_id: String,
        status: WaveformStatus,
        progress: f64,
    },
    Ping {
        t_viewer: f64,
    },
    Pong {
        t_viewer: f64,
        t_sender: f64,
    },
    Heartbeat {
        ts: f64,
    },
    Error {
        code: String,
        message: String,
    },
}

impl Message {
    /// Parses one text frame. Malformed or unknown messages give `None`;
    /// network input must never be able to crash the app.
    pub fn parse(text: &str) -> Option<Message> {
        serde_json::from_str(text).ok()
    }

    pub fn to_json(&self) -> String {
        // Serialising these plain data types cannot fail; the fallback only
        // exists so that this function has no panic path at all.
        serde_json::to_string(self).unwrap_or_else(|_| String::from("{}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURES: &str = include_str!("../../../../test-assets/protocol/messages.jsonl");

    fn type_of(line: &str) -> String {
        let value: serde_json::Value = serde_json::from_str(line).expect("fixture is JSON");
        value["type"].as_str().expect("fixture has a type").to_string()
    }

    #[test]
    fn every_fixture_message_round_trips() {
        let mut types = std::collections::BTreeSet::new();
        for line in FIXTURES.lines().filter(|l| !l.trim().is_empty()) {
            let msg = Message::parse(line).unwrap_or_else(|| panic!("should parse: {line}"));
            let json = msg.to_json();
            assert_eq!(Message::parse(&json), Some(msg), "round trip of {line}");
            assert_eq!(type_of(&json), type_of(line), "type tag of {line}");
            types.insert(type_of(line));
        }
        let expected = [
            "deck_loaded",
            "deck_state",
            "deck_unloaded",
            "error",
            "heartbeat",
            "hello",
            "ping",
            "pong",
            "source_status",
            "waveform_status",
        ];
        assert_eq!(types.iter().map(String::as_str).collect::<Vec<_>>(), expected);
    }

    #[test]
    fn field_names_match_the_wire_format() {
        let json = Message::DeckState {
            deck: 1,
            ts: 2.5,
            pos_s: 64.5,
            bpm_current: 126.0,
            playing: true,
            is_master: true,
            on_air: false,
        }
        .to_json();
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(value["type"], "deck_state");
        assert_eq!(value["deck"], 1);
        assert_eq!(value["pos_s"], 64.5);
        assert_eq!(value["is_master"], true);
        assert_eq!(value["on_air"], false);
    }

    #[test]
    fn malformed_and_unknown_messages_are_rejected() {
        for text in [
            "",
            "not json",
            "null",
            "[]",
            "{}",
            r#"{"type":"nope"}"#,
            r#"{"type":"pair_request","device_id":"x","nonce":"y"}"#,
            r#"{"type":"deck_state","deck":1}"#,
            r#"{"type":"deck_state","deck":300,"ts":1,"pos_s":1,"bpm_current":120,"playing":true,"is_master":false,"on_air":false}"#,
            r#"{"type":"ping"}"#,
            r#"{"type":"hello","protocol":1,"device_id":"a","name":"b","role":"dj","app_version":"1","os":"x"}"#,
        ] {
            assert_eq!(Message::parse(text), None, "{text}");
        }
    }

    #[test]
    fn integer_numbers_are_accepted_for_float_fields() {
        // JavaScript writes 124.0 as 124.
        let msg = Message::parse(r#"{"type":"pong","t_viewer":1,"t_sender":2}"#);
        assert_eq!(msg, Some(Message::Pong { t_viewer: 1.0, t_sender: 2.0 }));
    }
}
