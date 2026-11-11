//! The Sender's view of "what is on the decks right now".
//!
//! Source events come in through [`Session::apply`]. Each one updates the
//! snapshot (so a Viewer that connects later can be brought up to date) and is
//! broadcast to the Viewers that are already connected.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::broadcast;

use crate::lock;
use crate::protocol::chunk::{chunk_waveform, WaveformKind};
use crate::protocol::{Message, ResolveStatus, TrackInfo, WaveformInfo, WaveformStatus};
use crate::sources::{SourceEvent, SourceStatus};
use crate::waveform::synth::track_hash;
use crate::waveform::DETAIL_FPS;

/// A Viewer this many frames behind is disconnected; it reconnects and gets a
/// fresh snapshot. One track load is about 15 frames, playback 60 per second per deck.
const BROADCAST_CAPACITY: usize = 1024;

/// One WebSocket frame, shared between all Viewer connections.
#[derive(Debug, Clone)]
pub enum Frame {
    Text(Arc<str>),
    Binary(Arc<[u8]>),
}

struct LoadedDeck {
    loaded: Arc<str>,
    chunks: Vec<Arc<[u8]>>,
    last_state: Option<Arc<str>>,
}

#[derive(Default)]
struct Inner {
    source_status: Option<(SourceStatus, Arc<str>)>,
    decks: BTreeMap<u8, LoadedDeck>,
}

pub struct Session {
    inner: Mutex<Inner>,
    live: broadcast::Sender<Frame>,
    started: Instant,
    viewers: AtomicUsize,
}

/// Counts a connected Viewer for as long as it is alive.
pub struct ViewerGuard(Arc<Session>);

impl Drop for ViewerGuard {
    fn drop(&mut self) {
        self.0.viewers.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Session {
    pub fn new() -> Arc<Self> {
        let (live, _) = broadcast::channel(BROADCAST_CAPACITY);
        Arc::new(Self { inner: Mutex::new(Inner::default()), live, started: Instant::now(), viewers: AtomicUsize::new(0) })
    }

    /// The Sender's monotonic clock in seconds. `deck_state.ts`, `pong.t_sender`
    /// and `heartbeat.ts` all come from here.
    pub fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    pub fn viewer_count(&self) -> usize {
        self.viewers.load(Ordering::Relaxed)
    }

    /// What the source last said about itself, for the Sender's own window.
    pub fn source_status(&self) -> Option<SourceStatus> {
        lock(&self.inner).source_status.as_ref().map(|(status, _)| status.clone())
    }

    pub fn viewer_guard(self: &Arc<Self>) -> ViewerGuard {
        self.viewers.fetch_add(1, Ordering::Relaxed);
        ViewerGuard(Arc::clone(self))
    }

    /// The frames that bring a new Viewer up to date, plus a receiver for
    /// everything after them. Taken under one lock, so nothing is missed or
    /// sent twice.
    pub fn subscribe(&self) -> (Vec<Frame>, broadcast::Receiver<Frame>) {
        let inner = lock(&self.inner);
        let receiver = self.live.subscribe();
        let mut snapshot = Vec::new();
        if let Some((_, status)) = &inner.source_status {
            snapshot.push(Frame::Text(Arc::clone(status)));
        }
        for deck in inner.decks.values() {
            snapshot.push(Frame::Text(Arc::clone(&deck.loaded)));
            snapshot.extend(deck.chunks.iter().cloned().map(Frame::Binary));
            if let Some(state) = &deck.last_state {
                snapshot.push(Frame::Text(Arc::clone(state)));
            }
        }
        (snapshot, receiver)
    }

    pub fn apply(&self, event: SourceEvent) {
        match event {
            SourceEvent::Status(status) => {
                let text: Arc<str> = Message::SourceStatus {
                    software: status.software.clone(),
                    software_version: status.software_version.clone(),
                    connected: status.connected,
                    decks: status.decks,
                    reader: status.reader.clone(),
                    reader_status: status.reader_status.clone(),
                }
                .to_json()
                .into();
                let mut inner = lock(&self.inner);
                inner.source_status = Some((status, Arc::clone(&text)));
                self.send(Frame::Text(text));
            }
            SourceEvent::TrackLoaded { deck, meta, file_path: _, waveform } => {
                // Ids come from artist and title until audio files are hashed
                // (Milestone 2). Only the mock supplies a waveform so far, hence
                // the `mock` prefixes that PROTOCOL.md documents for it.
                let hash = track_hash(&meta.artist, &meta.title);
                let waveform_id = format!("mockwf-{hash:08x}");
                let track_id = if waveform.is_some() { format!("mock-{hash:08x}") } else { format!("trk-{hash:08x}") };
                let loaded: Arc<str> = Message::DeckLoaded {
                    deck,
                    track: TrackInfo {
                        track_id,
                        title: meta.title,
                        artist: meta.artist,
                        album: meta.album,
                        duration_s: meta.duration_s,
                        bpm_original: meta.bpm_original,
                        key: meta.key,
                        file_name: meta.file_name,
                        local: true,
                        resolve_status: ResolveStatus::Ok,
                    },
                    waveform: waveform.as_ref().map(|waveform| WaveformInfo {
                        waveform_id: waveform_id.clone(),
                        detail_fps: DETAIL_FPS,
                        frames: waveform.frames,
                        status: WaveformStatus::Ready,
                    }),
                }
                .to_json()
                .into();
                // Overview first, so the Viewer can draw the whole track immediately.
                let chunks: Vec<Arc<[u8]>> = waveform
                    .iter()
                    .flat_map(|waveform| {
                        chunk_waveform(&waveform_id, WaveformKind::Overview, &waveform.overview)
                            .into_iter()
                            .chain(chunk_waveform(&waveform_id, WaveformKind::Detail, &waveform.detail))
                    })
                    .map(Arc::from)
                    .collect();

                let mut inner = lock(&self.inner);
                self.send(Frame::Text(Arc::clone(&loaded)));
                for chunk in &chunks {
                    self.send(Frame::Binary(Arc::clone(chunk)));
                }
                inner.decks.insert(deck, LoadedDeck { loaded, chunks, last_state: None });
            }
            SourceEvent::DeckState(state) => {
                let mut inner = lock(&self.inner);
                let Some(deck) = inner.decks.get_mut(&state.deck) else {
                    return; // a state for a deck with nothing loaded has no meaning
                };
                let text: Arc<str> = Message::DeckState {
                    deck: state.deck,
                    ts: self.now(),
                    pos_s: state.pos_s,
                    bpm_current: state.bpm_current,
                    playing: state.playing,
                    is_master: state.is_master,
                    on_air: state.on_air,
                }
                .to_json()
                .into();
                deck.last_state = Some(Arc::clone(&text));
                self.send(Frame::Text(text));
            }
            SourceEvent::TrackUnloaded { deck } => {
                let mut inner = lock(&self.inner);
                inner.decks.remove(&deck);
                self.send(Frame::Text(Message::DeckUnloaded { deck }.to_json().into()));
            }
        }
    }

    fn send(&self, frame: Frame) {
        // An error only means that no Viewer is connected right now.
        let _ = self.live.send(frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::chunk::decode_chunk;
    use crate::sources::{DeckStateEvent, TrackMeta};
    use crate::waveform::synth::synth_waveform;

    fn meta() -> TrackMeta {
        TrackMeta {
            title: "Title".to_string(),
            artist: "Artist".to_string(),
            album: String::new(),
            duration_s: 60.0,
            bpm_original: 120.0,
            key: None,
            file_name: "a.flac".to_string(),
        }
    }

    /// A load as the mock sends it, with a synthetic waveform.
    fn loaded(deck: u8) -> SourceEvent {
        let meta = meta();
        let waveform = synth_waveform(meta.duration_s, meta.bpm_original, 1);
        SourceEvent::TrackLoaded { deck, meta, file_path: None, waveform: Some(waveform) }
    }

    fn state(deck: u8, pos_s: f64) -> SourceEvent {
        SourceEvent::DeckState(DeckStateEvent {
            deck,
            pos_s,
            bpm_current: 120.0,
            playing: true,
            is_master: true,
            on_air: true,
        })
    }

    fn text(frame: &Frame) -> Message {
        match frame {
            Frame::Text(text) => Message::parse(text).expect("session frames are valid messages"),
            Frame::Binary(_) => panic!("expected a text frame"),
        }
    }

    #[test]
    fn a_late_viewer_gets_status_track_waveform_and_last_state() {
        let session = Session::new();
        session.apply(SourceEvent::Status(SourceStatus {
            software: "mock".to_string(),
            software_version: "0".to_string(),
            connected: true,
            decks: 2,
            reader: "mock".to_string(),
            reader_status: "running".to_string(),
        }));
        session.apply(loaded(1));
        session.apply(state(1, 1.0));
        session.apply(state(1, 2.0));

        let (snapshot, _live) = session.subscribe();
        assert!(matches!(text(&snapshot[0]), Message::SourceStatus { decks: 2, .. }));
        let (frames, waveform_id) = match text(&snapshot[1]) {
            Message::DeckLoaded { deck: 1, waveform: Some(info), .. } => (info.frames, info.waveform_id),
            other => panic!("unexpected: {other:?}"),
        };
        assert_eq!(frames, 9000);

        // Binary frames: overview first, then the detail, complete and in order.
        let mut overview = Vec::new();
        let mut detail = Vec::new();
        let mut seen_detail = false;
        for frame in &snapshot[2..snapshot.len() - 1] {
            let Frame::Binary(bytes) = frame else { panic!("expected binary frames") };
            let chunk = decode_chunk(bytes).expect("valid chunk");
            assert_eq!(chunk.waveform_id, waveform_id);
            match chunk.kind {
                WaveformKind::Overview => {
                    assert!(!seen_detail, "overview must come first");
                    assert_eq!(chunk.offset as usize, overview.len());
                    overview.extend_from_slice(&chunk.payload);
                }
                WaveformKind::Detail => {
                    seen_detail = true;
                    assert_eq!(chunk.offset as usize, detail.len());
                    detail.extend_from_slice(&chunk.payload);
                }
            }
        }
        assert_eq!(overview.len(), 3600);
        assert_eq!(detail.len(), 9000 * 3);

        // Only the latest state is replayed.
        match text(&snapshot[snapshot.len() - 1]) {
            Message::DeckState { deck: 1, pos_s, .. } => assert_eq!(pos_s, 2.0),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn live_frames_follow_the_snapshot_without_gaps() {
        let session = Session::new();
        session.apply(loaded(2));
        let (_snapshot, mut live) = session.subscribe();
        session.apply(state(2, 5.0));
        session.apply(SourceEvent::TrackUnloaded { deck: 2 });
        session.apply(state(2, 6.0)); // ignored: nothing is loaded any more

        let first = live.try_recv().expect("state frame");
        assert!(matches!(text(&first), Message::DeckState { deck: 2, .. }));
        let second = live.try_recv().expect("unload frame");
        assert_eq!(text(&second), Message::DeckUnloaded { deck: 2 });
        assert!(live.try_recv().is_err());
        assert!(session.subscribe().0.is_empty());
    }

    #[test]
    fn a_track_without_a_waveform_is_announced_with_none() {
        let session = Session::new();
        session.apply(SourceEvent::TrackLoaded { deck: 1, meta: meta(), file_path: None, waveform: None });
        session.apply(state(1, 3.0));

        let (snapshot, _live) = session.subscribe();
        assert_eq!(snapshot.len(), 2, "deck_loaded and deck_state, no binary frames");
        match text(&snapshot[0]) {
            Message::DeckLoaded { deck: 1, track, waveform: None } => assert!(track.track_id.starts_with("trk-")),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn viewer_guard_counts_connections() {
        let session = Session::new();
        let first = session.viewer_guard();
        let second = session.viewer_guard();
        assert_eq!(session.viewer_count(), 2);
        drop(first);
        assert_eq!(session.viewer_count(), 1);
        drop(second);
        assert_eq!(session.viewer_count(), 0);
    }
}
