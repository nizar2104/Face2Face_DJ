//! Traktor Pro, through the traktor-api-client mod.
//!
//! Traktor runs QML scripts for its controllers, and those scripts can read
//! every deck property and make HTTP requests.
//! [traktor-api-client](https://github.com/ErikMinekus/traktor-api-client)
//! (MIT) is such a script: installed into Traktor, it POSTs JSON to
//! `http://localhost:8080` whenever something changes. This reader is the
//! server for those requests. Installing the mod is in docs/SOURCES.md.
//!
//! | Request | Body fields used |
//! |---|---|
//! | `POST /deckLoaded/<A-D>` | `title`, `artist`, `album`, `filePath`, `trackLength` (s), `bpm`, `tempo` (1.0 = no pitch), `key`, `keyText`, `elapsedTime` (s), `isPlaying` |
//! | `POST /updateDeck/<A-D>` | any of `elapsedTime`, `isPlaying`, `tempo` |
//! | `POST /updateMasterClock` | `deck` (letter or null) |
//! | `POST /updateChannel/<1-4>` | `isOnAir` |
//!
//! The mod reports the position when play state changes and once a second
//! while playing, so a jump inside a playing track (hot cue, loop, seek) shows
//! up to a second late.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{StatusCode, Uri};
use axum::Router;
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use super::board::{DeckBoard, DeckUpdate, TICK};
use super::key::parse_key;
use super::{DeckSource, SourceEvent, SourceStatus, TrackMeta, RETRY_OPEN};
use crate::lock;
use crate::protocol::MAX_DECKS;

/// Where traktor-api-client sends unless its `ApiClient.js` is edited.
pub const DEFAULT_HTTP_PORT: u16 = 8080;

pub struct TraktorSource {
    pub listen: SocketAddr,
}

impl Default for TraktorSource {
    fn default() -> Self {
        Self { listen: (Ipv4Addr::LOCALHOST, DEFAULT_HTTP_PORT).into() }
    }
}

fn status(connected: bool, decks: u8, reader_status: String) -> SourceStatus {
    SourceStatus {
        software: "traktor".to_string(),
        software_version: String::new(),
        connected,
        decks,
        reader: "traktor-api-client".to_string(),
        reader_status,
    }
}

/// Deck "A" to "D" (or "1" to "4", which is how channels are numbered) as 1 to 4.
fn deck_number(name: &str) -> Option<u8> {
    let &[byte] = name.as_bytes() else { return None };
    let deck = match byte.to_ascii_uppercase() {
        letter @ b'A'..=b'D' => letter - b'A' + 1,
        digit @ b'1'..=b'4' => digit - b'0',
        _ => return None,
    };
    (deck <= MAX_DECKS).then_some(deck)
}

fn number(body: &Value, field: &str) -> Option<f64> {
    body.get(field)?.as_f64().filter(|value| value.is_finite())
}

fn text(body: &Value, field: &str) -> String {
    body.get(field).and_then(Value::as_str).unwrap_or_default().trim().to_string()
}

/// The reader's logic, separate from the HTTP server so tests can call it.
struct Reader {
    board: DeckBoard,
    master: Option<u8>,
    on_air: [bool; MAX_DECKS as usize],
    connected: bool,
    reported_decks: u8,
}

impl Reader {
    fn new(events: mpsc::UnboundedSender<SourceEvent>) -> Self {
        let reader = Self {
            board: DeckBoard::new(events),
            master: None,
            // Until the mod says otherwise. It only reports changes, and a
            // channel fader that was up before we started never changes.
            on_air: [true; MAX_DECKS as usize],
            connected: false,
            reported_decks: 2,
        };
        reader.board.status(status(false, 2, "waiting for Traktor".to_string()));
        reader
    }

    /// One request from the mod. `false` if the path is not one of its endpoints.
    fn request(&mut self, path: &str, body: &Value) -> bool {
        let mut parts = path.trim_matches('/').split('/');
        let known = match (parts.next(), parts.next().and_then(deck_number), parts.next()) {
            (Some("deckLoaded"), Some(deck), None) => {
                self.deck_loaded(deck, body);
                true
            }
            (Some("updateDeck"), Some(deck), None) => {
                let tempo = number(body, "tempo");
                let bpm_original = self.board.meta(deck).map(|meta| meta.bpm_original);
                let update = DeckUpdate {
                    pos_s: number(body, "elapsedTime"),
                    playing: body.get("isPlaying").and_then(Value::as_bool),
                    bpm_current: tempo.zip(bpm_original).map(|(tempo, bpm)| tempo * bpm),
                    ..DeckUpdate::default()
                };
                // The mod also reports sync, key lock and cue changes, which we do not show.
                if update != DeckUpdate::default() {
                    self.board.update(deck, update);
                }
                true
            }
            (Some("updateChannel"), Some(deck), None) => {
                if let Some(on_air) = body.get("isOnAir").and_then(Value::as_bool) {
                    self.on_air[usize::from(deck - 1)] = on_air;
                    self.board.update(deck, DeckUpdate { on_air: Some(on_air), ..DeckUpdate::default() });
                }
                true
            }
            (Some("updateMasterClock"), None, None) if path.trim_matches('/') == "updateMasterClock" => {
                self.master = body.get("deck").and_then(Value::as_str).and_then(deck_number);
                self.board.set_master(self.master);
                true
            }
            _ => false,
        };
        if known {
            let decks = self.board.deck_span();
            if !self.connected || decks != self.reported_decks {
                self.connected = true;
                self.reported_decks = decks;
                self.board.status(status(true, decks, "running".to_string()));
            }
        }
        known
    }

    fn deck_loaded(&mut self, deck: u8, body: &Value) {
        let file_path = text(body, "filePath");
        let file_name =
            Path::new(&file_path).file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let mut title = text(body, "title");
        let duration_s = number(body, "trackLength").unwrap_or(0.0).max(0.0);
        // The mod sends this request for an emptied deck too, with nothing in it.
        if title.is_empty() && file_path.is_empty() && duration_s == 0.0 {
            self.board.unload(deck);
            return;
        }
        if title.is_empty() {
            title = Path::new(&file_name).file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
        }
        let bpm_original = number(body, "bpm").unwrap_or(0.0).max(0.0);
        let meta = TrackMeta {
            title,
            artist: text(body, "artist"),
            album: text(body, "album"),
            duration_s,
            bpm_original,
            // `key` follows Traktor's display preference, `keyText` is the file's tag.
            key: parse_key(&text(body, "key")).or_else(|| parse_key(&text(body, "keyText"))),
            file_name,
        };
        self.board.load(deck, meta, (!file_path.is_empty()).then(|| PathBuf::from(file_path)));
        let playing = body.get("isPlaying").and_then(Value::as_bool).unwrap_or(false);
        let update = DeckUpdate {
            pos_s: Some(number(body, "elapsedTime").unwrap_or(0.0)),
            bpm_current: Some(bpm_original * number(body, "tempo").unwrap_or(1.0)),
            playing: Some(playing),
            is_master: Some(self.master == Some(deck)),
            on_air: Some(self.on_air[usize::from(deck - 1)]),
        };
        self.board.update(deck, update);
    }
}

type Shared = Arc<Mutex<Reader>>;

async fn handle(State(reader): State<Shared>, uri: Uri, body: Bytes) -> StatusCode {
    // A body that is not JSON is treated as an empty report, not an error:
    // nothing reads the mod's error responses.
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    if lock(&reader).request(uri.path(), &body) {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

impl DeckSource for TraktorSource {
    fn name(&self) -> &'static str {
        "traktor"
    }

    fn spawn(self: Box<Self>, events: mpsc::UnboundedSender<SourceEvent>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let reader: Shared = Arc::new(Mutex::new(Reader::new(events)));
            // The port may be taken for a while (8080 is popular): say so once,
            // then keep trying, so the reader recovers without being restarted.
            let mut reported = false;
            let listener = loop {
                match TcpListener::bind(self.listen).await {
                    Ok(listener) => break listener,
                    Err(error) => {
                        if !reported {
                            let message = format!("cannot listen on {}: {error}", self.listen);
                            lock(&reader).board.status(status(false, 2, message));
                            reported = true;
                        }
                        tokio::time::sleep(RETRY_OPEN).await;
                    }
                }
            };
            if reported {
                lock(&reader).board.status(status(false, 2, "waiting for Traktor".to_string()));
            }
            let ticking = Arc::clone(&reader);
            let ticker = tokio::spawn(async move {
                let mut ticks = tokio::time::interval(TICK);
                ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
                loop {
                    ticks.tick().await;
                    lock(&ticking).board.tick();
                }
            });
            let app = Router::new().fallback(handle).with_state(reader);
            if let Err(error) = axum::serve(listener, app).await {
                tracing::warn!("Traktor receiver stopped: {error}");
            }
            ticker.abort();
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    use super::super::board::testing::{drain, last_state};
    use super::*;

    /// What the mod sends for a loaded deck (field list from its ApiDeck.qml).
    fn deck_loaded_body() -> Value {
        json!({
            "filePath": "/Users/dj/Music/Mara Voss - Neon Tide.flac",
            "title": "Neon Tide", "artist": "Mara Voss", "album": "Sessions", "genre": "House",
            "comment": "", "comment2": "", "label": "", "mix": "", "remixer": "",
            "key": "8m", "keyText": "F#m", "gridOffset": 0.012,
            "trackLength": 372.4, "elapsedTime": 12.5,
            "nextCuePos": null, "nextCueName": null, "nextCueType": null,
            "prevCuePos": null, "prevCueName": null, "prevCueType": null,
            "bpm": 124.0, "tempo": 1.05, "resultingKey": 21.0,
            "isPlaying": true, "isSynced": false, "isKeyLockOn": true
        })
    }

    fn reader() -> (Reader, mpsc::UnboundedReceiver<SourceEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Reader::new(tx), rx)
    }

    #[test]
    fn a_loaded_deck_is_reported_with_its_track_and_state() {
        let (mut reader, mut events) = reader();
        assert!(reader.request("/updateMasterClock", &json!({"deck": "B", "bpm": 130.2})));
        assert!(reader.request("/deckLoaded/B", &deck_loaded_body()));

        let sent = drain(&mut events);
        assert!(matches!(&sent[0], SourceEvent::Status(status) if !status.connected && status.software == "traktor"));
        assert!(sent.iter().any(|event| matches!(event, SourceEvent::Status(status) if status.connected && status.decks == 2)));
        let (meta, file_path) = sent
            .iter()
            .find_map(|event| match event {
                SourceEvent::TrackLoaded { deck: 2, meta, file_path, waveform: None } => Some((meta.clone(), file_path.clone())),
                _ => None,
            })
            .expect("deck B is deck 2");
        assert_eq!(meta.title, "Neon Tide");
        assert_eq!(meta.file_name, "Mara Voss - Neon Tide.flac");
        assert_eq!((meta.duration_s, meta.bpm_original), (372.4, 124.0));
        assert_eq!(meta.key.map(|key| key.camelot).as_deref(), Some("3A"), "Open Key 8m, not the tag");
        assert_eq!(file_path, Some(PathBuf::from("/Users/dj/Music/Mara Voss - Neon Tide.flac")));

        let state = last_state(&sent, 2).expect("a state");
        assert!(state.playing && state.is_master && state.on_air);
        assert!((state.bpm_current - 130.2).abs() < 1e-9);
        assert!((state.pos_s - 12.5).abs() < 0.05);
    }

    #[test]
    fn partial_updates_change_only_what_they_name() {
        let (mut reader, mut events) = reader();
        reader.request("/deckLoaded/A", &deck_loaded_body());
        drain(&mut events);

        reader.request("/updateDeck/A", &json!({"elapsedTime": 200.0, "isPlaying": false}));
        let state = last_state(&drain(&mut events), 1).expect("a state");
        assert_eq!((state.pos_s, state.playing), (200.0, false));
        assert!((state.bpm_current - 130.2).abs() < 1e-9);

        reader.request("/updateDeck/A", &json!({"tempo": 1.0}));
        assert_eq!(last_state(&drain(&mut events), 1).map(|state| state.bpm_current), Some(124.0));

        reader.request("/updateChannel/1", &json!({"isOnAir": false}));
        assert_eq!(last_state(&drain(&mut events), 1).map(|state| state.on_air), Some(false));
        // The level reports that follow carry nothing we show.
        reader.request("/updateChannel/1", &json!({"onAirLevel": 0.4}));
        reader.request("/updateDeck/A", &json!({"isSynced": true}));
        assert!(drain(&mut events).is_empty());

        reader.request("/updateMasterClock", &json!({"deck": null, "bpm": 120.0}));
        assert!(drain(&mut events).is_empty(), "deck A was not master");
    }

    #[test]
    fn an_emptied_deck_unloads_and_decks_c_and_d_widen_the_layout() {
        let (mut reader, mut events) = reader();
        reader.request("/deckLoaded/D", &deck_loaded_body());
        assert!(drain(&mut events).iter().any(|event| matches!(event, SourceEvent::Status(status) if status.decks == 4)));

        let empty = json!({"filePath": "", "title": "", "artist": "", "trackLength": 0, "bpm": 0, "isPlaying": false});
        reader.request("/deckLoaded/D", &empty);
        assert!(drain(&mut events).contains(&SourceEvent::TrackUnloaded { deck: 4 }));
    }

    #[test]
    fn a_track_without_tags_is_named_after_its_file() {
        let (mut reader, mut events) = reader();
        reader.request("/deckLoaded/A", &json!({"filePath": "C:\\Music\\untitled.wav", "title": "", "trackLength": 60}));
        // A Windows path on this (non-Windows) test machine stays one name; on Windows it splits.
        let title = drain(&mut events).iter().find_map(|event| match event {
            SourceEvent::TrackLoaded { meta, .. } => Some(meta.title.clone()),
            _ => None,
        });
        assert!(title.is_some_and(|title| title.ends_with("untitled")));
    }

    #[test]
    fn other_paths_are_not_found() {
        let (mut reader, mut events) = reader();
        drain(&mut events);
        for path in ["/", "/deckLoaded", "/deckLoaded/E", "/deckLoaded/A/x", "/updateChannel/5", "/updateMasterClock/A", "/favicon.ico"] {
            assert!(!reader.request(path, &Value::Null), "{path}");
        }
        assert!(drain(&mut events).is_empty(), "still waiting for Traktor");
    }

    #[tokio::test]
    async fn the_mods_http_requests_reach_the_reader() {
        let probe = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.expect("a free port");
        let listen = probe.local_addr().expect("an address");
        drop(probe);
        let (tx, mut events) = mpsc::unbounded_channel();
        let task = Box::new(TraktorSource { listen }).spawn(tx);

        // The request as QML's XMLHttpRequest writes it.
        let body = deck_loaded_body().to_string();
        let request = format!(
            "POST /deckLoaded/A HTTP/1.1\r\nHost: localhost:8080\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let mut stream = loop {
            match TcpStream::connect(listen).await {
                Ok(stream) => break stream,
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
            }
        };
        stream.write_all(request.as_bytes()).await.expect("writes");
        let mut response = String::new();
        stream.read_to_string(&mut response).await.expect("reads");
        assert!(response.starts_with("HTTP/1.1 204"), "{response}");

        let sent = drain(&mut events);
        task.abort();
        assert!(sent.iter().any(|event| matches!(event, SourceEvent::TrackLoaded { deck: 1, meta, .. } if meta.title == "Neon Tide")));
    }

    #[tokio::test]
    async fn a_busy_port_is_reported_and_taken_once_it_is_free() {
        let holder = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.expect("a port");
        let listen = holder.local_addr().expect("an address");
        let (tx, mut events) = mpsc::unbounded_channel();
        let task = Box::new(TraktorSource { listen }).spawn(tx);

        async fn wait_for(events: &mut mpsc::UnboundedReceiver<SourceEvent>, wanted: &str) {
            let found = tokio::time::timeout(RETRY_OPEN * 3, async {
                loop {
                    match events.recv().await {
                        Some(SourceEvent::Status(status)) if status.reader_status.contains(wanted) => return,
                        Some(_) => {}
                        None => panic!("the reader stopped"),
                    }
                }
            })
            .await;
            assert!(found.is_ok(), "no status containing {wanted:?}");
        }
        wait_for(&mut events, "cannot listen").await;
        drop(holder);
        wait_for(&mut events, "waiting for Traktor").await;
        assert!(TcpStream::connect(listen).await.is_ok(), "the reader now holds the port");
        task.abort();
    }
}
