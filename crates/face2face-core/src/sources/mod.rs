//! Source adapters: where deck data comes from (brief, section 9).
//!
//! A source turns whatever its DJ software exposes into [`SourceEvent`]s.
//! [`mock::MockSource`] simulates decks. The readers for real software are
//! [`traktor`], [`rekordbox`], [`prolink`] and [`mixxx`]; how each one gets its
//! data, and what it cannot know, is in `docs/SOURCES.md`.

pub mod board;
pub mod key;
pub mod mixxx;
pub mod mock;
pub mod osc;
pub mod prolink;
pub mod rekordbox;
pub mod traktor;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::protocol::KeyInfo;
use crate::waveform::Waveform;

/// Which source the Sender reads. The UI sends these names (`app/src/tauri.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Mock,
    Traktor,
    Rekordbox,
    ProDjLink,
    Mixxx,
}

impl SourceKind {
    /// The reader for this kind, with its default addresses. `None` for the
    /// mock, which the Sender builds itself because it also takes commands.
    pub fn reader(self) -> Option<Box<dyn DeckSource>> {
        match self {
            SourceKind::Mock => None,
            SourceKind::Traktor => Some(Box::new(traktor::TraktorSource::default())),
            SourceKind::Rekordbox => Some(Box::new(rekordbox::RekordboxSource::default())),
            SourceKind::ProDjLink => Some(Box::new(prolink::ProDjLinkSource::default())),
            SourceKind::Mixxx => Some(Box::new(mixxx::MixxxSource)),
        }
    }
}

/// How long a reader waits before trying again to open a port or device that
/// was not available (another program had it, or it was not plugged in yet).
pub(crate) const RETRY_OPEN: std::time::Duration = std::time::Duration::from_secs(2);

/// The pause after a failed receive on a reader's socket.
pub(crate) const RECEIVE_ERROR_PAUSE: std::time::Duration = std::time::Duration::from_millis(100);

/// What a source knows about a track when it is loaded. Rekordbox will leave
/// `key` empty and get it from the library through the resolver (M4).
#[derive(Debug, Clone, PartialEq)]
pub struct TrackMeta {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_s: f64,
    pub bpm_original: f64,
    pub key: Option<KeyInfo>,
    pub file_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceStatus {
    pub software: String,
    pub software_version: String,
    pub connected: bool,
    pub decks: u8,
    pub reader: String,
    pub reader_status: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeckStateEvent {
    pub deck: u8,
    pub pos_s: f64,
    pub bpm_current: f64,
    pub playing: bool,
    pub is_master: bool,
    pub on_air: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SourceEvent {
    Status(SourceStatus),
    TrackLoaded {
        deck: u8,
        meta: TrackMeta,
        /// `None` until a source can name the file (mock tracks have no file).
        file_path: Option<PathBuf>,
        /// `None` when the source has no waveform: every real reader until
        /// audio analysis exists (Milestone 2). The Viewer then shows "No waveform".
        waveform: Option<Waveform>,
    },
    DeckState(DeckStateEvent),
    TrackUnloaded {
        deck: u8,
    },
}

/// A running connection to DJ software (or a simulation of one).
pub trait DeckSource: Send + 'static {
    fn name(&self) -> &'static str;

    /// Starts the source on the current Tokio runtime. Events are sent to
    /// `events`; the task ends when the receiver is dropped or the returned
    /// handle is aborted.
    fn spawn(self: Box<Self>, events: mpsc::UnboundedSender<SourceEvent>) -> JoinHandle<()>;
}
