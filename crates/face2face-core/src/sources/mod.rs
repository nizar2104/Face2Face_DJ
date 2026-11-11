//! Source adapters: where deck data comes from (brief, section 9).
//!
//! A source turns whatever its DJ software exposes into [`SourceEvent`]s.
//! Milestone 0 has [`mock::MockSource`]; Traktor (M3) and Rekordbox (M4)
//! implement the same trait.

pub mod mock;

use std::path::PathBuf;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::protocol::KeyInfo;

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
