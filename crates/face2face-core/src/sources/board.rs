//! What the readers for real DJ software share: remembering what is on each
//! deck, and turning occasional reports into a steady stream of `DeckState`s.
//!
//! DJ software reports the playhead far less often than a Viewer wants it
//! (Traktor once a second, a CDJ five times). The board keeps the last report
//! as an anchor and, on every [`DeckBoard::tick`], sends where the playhead
//! must be by now. A new report replaces the anchor.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use super::{DeckStateEvent, SourceEvent, SourceStatus, TrackMeta};
use crate::protocol::MAX_DECKS;

/// How often a reader calls [`DeckBoard::tick`]. The Viewer interpolates
/// between states and gives up after a second without one, so this only has
/// to be comfortably faster than that.
pub const TICK: Duration = Duration::from_millis(50);

/// The changes one report makes to a deck. Fields left `None` keep their value.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DeckUpdate {
    pub pos_s: Option<f64>,
    pub bpm_current: Option<f64>,
    pub playing: Option<bool>,
    pub is_master: Option<bool>,
    pub on_air: Option<bool>,
}

struct Slot {
    meta: TrackMeta,
    file_path: Option<PathBuf>,
    /// The playhead was at `pos_s` at `at`.
    pos_s: f64,
    at: Instant,
    bpm_current: f64,
    playing: bool,
    is_master: bool,
    on_air: bool,
}

impl Slot {
    fn pos_at(&self, now: Instant) -> f64 {
        let mut pos = self.pos_s;
        if self.playing {
            let original = self.meta.bpm_original;
            let rate = if original > 0.0 && self.bpm_current > 0.0 { self.bpm_current / original } else { 1.0 };
            pos += now.saturating_duration_since(self.at).as_secs_f64() * rate;
        }
        // A duration of 0 means the source does not know it.
        if self.meta.duration_s > 0.0 {
            pos = pos.min(self.meta.duration_s);
        }
        pos.max(0.0)
    }

    fn state(&self, deck: u8, now: Instant) -> SourceEvent {
        SourceEvent::DeckState(DeckStateEvent {
            deck,
            pos_s: self.pos_at(now),
            bpm_current: self.bpm_current,
            playing: self.playing,
            is_master: self.is_master,
            on_air: self.on_air,
        })
    }
}

pub struct DeckBoard {
    events: mpsc::UnboundedSender<SourceEvent>,
    slots: [Option<Slot>; MAX_DECKS as usize],
}

impl DeckBoard {
    pub fn new(events: mpsc::UnboundedSender<SourceEvent>) -> Self {
        Self { events, slots: [None, None, None, None] }
    }

    fn send(&self, event: SourceEvent) {
        // An error means the Sender has stopped; it aborts the reader's task itself.
        let _ = self.events.send(event);
    }

    fn slot(&mut self, deck: u8) -> Option<&mut Slot> {
        self.slots.get_mut(usize::from(deck).checked_sub(1)?)?.as_mut()
    }

    pub fn status(&self, status: SourceStatus) {
        self.send(SourceEvent::Status(status));
    }

    pub fn is_loaded(&self, deck: u8) -> bool {
        usize::from(deck).checked_sub(1).and_then(|index| self.slots.get(index)).is_some_and(Option::is_some)
    }

    pub fn meta(&self, deck: u8) -> Option<&TrackMeta> {
        self.slots.get(usize::from(deck).checked_sub(1)?)?.as_ref().map(|slot| &slot.meta)
    }

    /// The highest deck with a track, at least 2: how many decks to report
    /// when the software does not say.
    pub fn deck_span(&self) -> u8 {
        self.slots.iter().rposition(Option::is_some).map_or(2, |index| (index as u8 + 1).max(2))
    }

    /// A track is on `deck` (1 to 4; other numbers are ignored): paused at
    /// the start until the next [`DeckBoard::update`].
    pub fn load(&mut self, deck: u8, meta: TrackMeta, file_path: Option<PathBuf>) {
        let Some(place) = usize::from(deck).checked_sub(1).and_then(|index| self.slots.get_mut(index)) else {
            return;
        };
        let bpm_current = meta.bpm_original;
        *place = Some(Slot {
            meta: meta.clone(),
            file_path: file_path.clone(),
            pos_s: 0.0,
            at: Instant::now(),
            bpm_current,
            playing: false,
            is_master: false,
            on_air: false,
        });
        self.send(SourceEvent::TrackLoaded { deck, meta, file_path, waveform: None });
    }

    /// Corrects what is known about the loaded track (a duration or tempo that
    /// arrived after the title), keeping the playhead. Does nothing if `change`
    /// changes nothing.
    pub fn revise(&mut self, deck: u8, change: impl FnOnce(&mut TrackMeta)) {
        let now = Instant::now();
        let Some(slot) = self.slot(deck) else { return };
        let mut meta = slot.meta.clone();
        change(&mut meta);
        if meta == slot.meta {
            return;
        }
        slot.pos_s = slot.pos_at(now);
        slot.at = now;
        slot.meta = meta.clone();
        let (file_path, state) = (slot.file_path.clone(), slot.state(deck, now));
        // `deck_loaded` resets the Viewer's deck, so the state follows it.
        self.send(SourceEvent::TrackLoaded { deck, meta, file_path, waveform: None });
        self.send(state);
    }

    pub fn unload(&mut self, deck: u8) {
        let Some(place) = usize::from(deck).checked_sub(1).and_then(|index| self.slots.get_mut(index)) else {
            return;
        };
        if place.take().is_some() {
            self.send(SourceEvent::TrackUnloaded { deck });
        }
    }

    /// Applies a report that describes the deck as of `at`, and sends the new
    /// state. Ignored while nothing is loaded.
    pub fn update_at(&mut self, deck: u8, at: Instant, update: DeckUpdate) {
        let now = Instant::now();
        let Some(slot) = self.slot(deck) else { return };
        // Move the anchor to `at` first, so a change of tempo or play state
        // takes effect from there and not from the previous report.
        slot.pos_s = update.pos_s.unwrap_or_else(|| slot.pos_at(at));
        slot.at = at;
        if let Some(bpm) = update.bpm_current {
            slot.bpm_current = bpm;
        }
        if let Some(playing) = update.playing {
            slot.playing = playing;
        }
        if let Some(is_master) = update.is_master {
            slot.is_master = is_master;
        }
        if let Some(on_air) = update.on_air {
            slot.on_air = on_air;
        }
        let state = slot.state(deck, now);
        self.send(state);
    }

    pub fn update(&mut self, deck: u8, update: DeckUpdate) {
        self.update_at(deck, Instant::now(), update);
    }

    /// Makes `master` the only tempo master (`None`: there is none).
    pub fn set_master(&mut self, master: Option<u8>) {
        for deck in 1..=MAX_DECKS {
            let is_master = master == Some(deck);
            if self.slot(deck).is_some_and(|slot| slot.is_master != is_master) {
                self.update(deck, DeckUpdate { is_master: Some(is_master), ..DeckUpdate::default() });
            }
        }
    }

    /// Sends the current state of every playing deck. Call every [`TICK`].
    pub fn tick(&self) {
        let now = Instant::now();
        for (index, slot) in self.slots.iter().enumerate() {
            if let Some(slot) = slot.as_ref().filter(|slot| slot.playing) {
                self.send(slot.state(index as u8 + 1, now));
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    pub fn meta(title: &str, duration_s: f64, bpm: f64) -> TrackMeta {
        TrackMeta {
            title: title.to_string(),
            artist: "Artist".to_string(),
            album: String::new(),
            duration_s,
            bpm_original: bpm,
            key: None,
            file_name: String::new(),
        }
    }

    /// Everything a reader has sent so far.
    pub fn drain(events: &mut mpsc::UnboundedReceiver<SourceEvent>) -> Vec<SourceEvent> {
        std::iter::from_fn(|| events.try_recv().ok()).collect()
    }

    /// The last state sent for `deck`.
    pub fn last_state(events: &[SourceEvent], deck: u8) -> Option<DeckStateEvent> {
        events.iter().rev().find_map(|event| match event {
            SourceEvent::DeckState(state) if state.deck == deck => Some(state.clone()),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{drain, last_state, meta};
    use super::*;

    fn board() -> (DeckBoard, mpsc::UnboundedReceiver<SourceEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (DeckBoard::new(tx), rx)
    }

    #[test]
    fn a_playing_deck_moves_at_its_tempo_between_reports() {
        let (mut board, mut events) = board();
        board.load(1, meta("A", 300.0, 120.0), None);
        let start = Instant::now() - Duration::from_secs(10);
        board.update_at(1, start, DeckUpdate { pos_s: Some(20.0), bpm_current: Some(126.0), playing: Some(true), ..Default::default() });

        let sent = drain(&mut events);
        assert!(matches!(sent[0], SourceEvent::TrackLoaded { deck: 1, waveform: None, .. }));
        // 10 s later at +5 %: 20 + 10.5.
        let pos = last_state(&sent, 1).expect("a state").pos_s;
        assert!((pos - 30.5).abs() < 0.1, "pos {pos}");

        board.tick();
        assert!(last_state(&drain(&mut events), 1).is_some(), "ticks send playing decks");
    }

    #[test]
    fn a_paused_deck_stays_put_and_is_not_repeated() {
        let (mut board, mut events) = board();
        board.load(2, meta("A", 300.0, 120.0), None);
        board.update_at(2, Instant::now() - Duration::from_secs(5), DeckUpdate { pos_s: Some(42.0), ..Default::default() });
        assert_eq!(last_state(&drain(&mut events), 2).map(|state| state.pos_s), Some(42.0));
        board.tick();
        assert!(drain(&mut events).is_empty());
    }

    #[test]
    fn pausing_keeps_the_position_reached_so_far() {
        let (mut board, mut events) = board();
        board.load(1, meta("A", 300.0, 100.0), None);
        let then = Instant::now() - Duration::from_secs(4);
        board.update_at(1, then, DeckUpdate { pos_s: Some(10.0), playing: Some(true), ..Default::default() });
        board.update(1, DeckUpdate { playing: Some(false), ..Default::default() });
        let state = last_state(&drain(&mut events), 1).expect("a state");
        assert!(!state.playing);
        assert!((state.pos_s - 14.0).abs() < 0.1, "pos {}", state.pos_s);
    }

    #[test]
    fn the_playhead_stops_at_the_end_of_the_track() {
        let (mut board, mut events) = board();
        board.load(1, meta("A", 60.0, 120.0), None);
        let then = Instant::now() - Duration::from_secs(30);
        board.update_at(1, then, DeckUpdate { pos_s: Some(50.0), playing: Some(true), ..Default::default() });
        assert_eq!(last_state(&drain(&mut events), 1).map(|state| state.pos_s), Some(60.0));
    }

    #[test]
    fn updates_for_empty_or_impossible_decks_are_ignored() {
        let (mut board, mut events) = board();
        board.update(1, DeckUpdate { pos_s: Some(1.0), ..Default::default() });
        board.load(0, meta("A", 60.0, 120.0), None);
        board.load(5, meta("A", 60.0, 120.0), None);
        board.unload(3);
        assert!(drain(&mut events).is_empty());
        assert_eq!(board.deck_span(), 2);
    }

    #[test]
    fn there_is_one_master_at_a_time() {
        let (mut board, mut events) = board();
        board.load(1, meta("A", 60.0, 120.0), None);
        board.load(3, meta("B", 60.0, 120.0), None);
        assert_eq!(board.deck_span(), 3);
        board.set_master(Some(1));
        board.set_master(Some(3));
        let sent = drain(&mut events);
        assert_eq!(last_state(&sent, 1).map(|state| state.is_master), Some(false));
        assert_eq!(last_state(&sent, 3).map(|state| state.is_master), Some(true));
    }

    #[test]
    fn revising_the_track_sends_it_again_with_its_state() {
        let (mut board, mut events) = board();
        board.load(1, meta("A", 0.0, 120.0), None);
        board.update(1, DeckUpdate { pos_s: Some(12.0), ..Default::default() });
        drain(&mut events);

        board.revise(1, |meta| meta.duration_s = 200.0);
        let sent = drain(&mut events);
        assert!(matches!(&sent[0], SourceEvent::TrackLoaded { meta, .. } if meta.duration_s == 200.0));
        assert_eq!(last_state(&sent, 1).map(|state| state.pos_s), Some(12.0));

        board.revise(1, |meta| meta.duration_s = 200.0);
        assert!(drain(&mut events).is_empty(), "no change, nothing sent");
    }
}
