//! Simulated DJ decks (Milestone 0).
//!
//! [`MockEngine`] is the pure simulation: commands in, [`SourceEvent`]s out, no
//! clock and no I/O. [`MockSource`] runs it at 60 Hz as a [`DeckSource`], with
//! an optional scripted session, and [`MockHandle`] feeds it text commands.
//!
//! The TypeScript twin is `tools/mock-sender/engine.ts`; keep the command
//! vocabulary and the rules identical in both.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use super::{DeckSource, DeckStateEvent, SourceEvent, SourceStatus, TrackMeta};
use crate::lock;
use crate::protocol::KeyInfo;

pub const TICK_HZ: f64 = 60.0;
pub const MAX_PITCH_PERCENT: f64 = 50.0;

/// The scripted session shared with the TypeScript mock sender.
pub const DEMO_SESSION: &str = include_str!("../../../../test-assets/sessions/demo.session");

pub struct MockTrack {
    pub title: &'static str,
    pub artist: &'static str,
    pub album: &'static str,
    pub duration_s: f64,
    pub bpm: f64,
    pub camelot: &'static str,
    pub musical: &'static str,
    pub file_name: &'static str,
}

impl MockTrack {
    fn meta(&self) -> TrackMeta {
        TrackMeta {
            title: self.title.to_string(),
            artist: self.artist.to_string(),
            album: self.album.to_string(),
            duration_s: self.duration_s,
            bpm_original: self.bpm,
            key: Some(KeyInfo { camelot: self.camelot.to_string(), musical: self.musical.to_string() }),
            file_name: self.file_name.to_string(),
        }
    }
}

/// Invented tracks. No real files are involved in Milestone 0.
pub static MOCK_LIBRARY: [MockTrack; 4] = [
    MockTrack {
        title: "Neon Tide",
        artist: "Mara Voss",
        album: "Mock Sessions",
        duration_s: 372.4,
        bpm: 124.0,
        camelot: "8A",
        musical: "Am",
        file_name: "Mara Voss - Neon Tide.flac",
    },
    MockTrack {
        title: "Glass Garden",
        artist: "Okta & Finch",
        album: "Mock Sessions",
        duration_s: 341.0,
        bpm: 126.0,
        camelot: "9A",
        musical: "Em",
        file_name: "Okta & Finch - Glass Garden.mp3",
    },
    MockTrack {
        title: "Low Orbit",
        artist: "Hadley Kru",
        album: "Mock Sessions",
        duration_s: 405.5,
        bpm: 122.0,
        camelot: "7A",
        musical: "Dm",
        file_name: "Hadley Kru - Low Orbit.wav",
    },
    MockTrack {
        title: "Paper Suns",
        artist: "Ilse Marten",
        album: "Mock Sessions",
        duration_s: 298.2,
        bpm: 128.0,
        camelot: "10B",
        musical: "D",
        file_name: "Ilse Marten - Paper Suns.aiff",
    },
];

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MockCommand {
    /// `track` is an index into [`MOCK_LIBRARY`]; `None` takes the next one.
    Load { deck: u8, track: Option<usize> },
    Play { deck: u8 },
    Pause { deck: u8 },
    Pitch { deck: u8, percent: f64 },
    Jump { deck: u8, beats: f64 },
    Eject { deck: u8 },
    Master { deck: u8 },
    Demo(bool),
}

fn number_arg(parts: &[&str]) -> Option<f64> {
    if parts.len() != 3 {
        return None;
    }
    parts[2].parse::<f64>().ok().filter(|value| value.is_finite())
}

/// Parses one command line, e.g. `play 1`, `pitch 2 -1.5`, `jump 1 -16`.
pub fn parse_command(line: &str) -> Result<MockCommand, String> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    let name = parts.first().map(|part| part.to_ascii_lowercase()).unwrap_or_default();

    if name == "demo" {
        return match parts.get(1).copied() {
            Some("on") if parts.len() == 2 => Ok(MockCommand::Demo(true)),
            Some("off") if parts.len() == 2 => Ok(MockCommand::Demo(false)),
            _ => Err("usage: demo on|off".to_string()),
        };
    }

    let deck = parts
        .get(1)
        .and_then(|part| part.parse::<u8>().ok())
        .filter(|deck| (1..=4).contains(deck))
        .ok_or_else(|| format!("\"{}\": expected a deck number 1-4", line.trim()))?;

    match name.as_str() {
        "load" => {
            let usage = || format!("usage: load <deck> [track 1-{}]", MOCK_LIBRARY.len());
            match parts.len() {
                2 => Ok(MockCommand::Load { deck, track: None }),
                3 => {
                    let number = parts[2]
                        .parse::<usize>()
                        .ok()
                        .filter(|number| (1..=MOCK_LIBRARY.len()).contains(number))
                        .ok_or_else(usage)?;
                    Ok(MockCommand::Load { deck, track: Some(number - 1) })
                }
                _ => Err(usage()),
            }
        }
        "play" | "pause" | "eject" | "master" => {
            if parts.len() != 2 {
                return Err(format!("usage: {name} <deck>"));
            }
            Ok(match name.as_str() {
                "play" => MockCommand::Play { deck },
                "pause" => MockCommand::Pause { deck },
                "eject" => MockCommand::Eject { deck },
                _ => MockCommand::Master { deck },
            })
        }
        "pitch" => {
            let percent = number_arg(&parts).ok_or_else(|| "usage: pitch <deck> <percent>".to_string())?;
            Ok(MockCommand::Pitch { deck, percent })
        }
        "jump" => {
            let beats = number_arg(&parts).ok_or_else(|| "usage: jump <deck> <beats>".to_string())?;
            Ok(MockCommand::Jump { deck, beats })
        }
        _ => Err(format!("unknown command \"{name}\"")),
    }
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct DeckSim {
    track: Option<usize>,
    pos: f64,
    playing: bool,
    pitch: f64,
}

const EMPTY_DECK: DeckSim = DeckSim { track: None, pos: 0.0, playing: false, pitch: 0.0 };

pub struct MockEngine {
    decks: Vec<DeckSim>,
    master: Option<u8>,
    next_track: usize,
}

impl MockEngine {
    pub fn new(deck_count: u8) -> Self {
        Self::starting_at(deck_count, 0)
    }

    /// Like [`MockEngine::new`], but the first `load` without a track number
    /// takes library index `first_track`. Two linked laptops use different
    /// values so that each shows the other different tracks.
    pub fn starting_at(deck_count: u8, first_track: usize) -> Self {
        let count = usize::from(deck_count.clamp(1, 4));
        Self { decks: vec![EMPTY_DECK; count], master: None, next_track: first_track }
    }

    pub fn deck_count(&self) -> u8 {
        self.decks.len() as u8
    }

    /// Applies a command. Returns the resulting events and, if the command
    /// could not be carried out, why.
    pub fn apply(&mut self, cmd: MockCommand) -> (Vec<SourceEvent>, Option<String>) {
        let deck = match cmd {
            MockCommand::Load { deck, .. }
            | MockCommand::Play { deck }
            | MockCommand::Pause { deck }
            | MockCommand::Pitch { deck, .. }
            | MockCommand::Jump { deck, .. }
            | MockCommand::Eject { deck }
            | MockCommand::Master { deck } => deck,
            MockCommand::Demo(_) => return (Vec::new(), None), // handled by MockSource
        };
        let index = usize::from(deck).wrapping_sub(1);
        if index >= self.decks.len() {
            return (Vec::new(), Some(format!("this session has {} decks", self.decks.len())));
        }

        if let MockCommand::Load { track, .. } = cmd {
            let track_index = match track {
                Some(track_index) => track_index % MOCK_LIBRARY.len(),
                None => {
                    let next = self.next_track % MOCK_LIBRARY.len();
                    self.next_track += 1;
                    next
                }
            };
            self.decks[index] = DeckSim { track: Some(track_index), ..EMPTY_DECK };
            if self.master == Some(deck) {
                self.master = self.first_playing();
            }
            let mut events =
                vec![SourceEvent::TrackLoaded { deck, meta: MOCK_LIBRARY[track_index].meta(), file_path: None }];
            events.extend(self.all_states());
            return (events, None);
        }

        let Some(track_index) = self.decks[index].track else {
            return (Vec::new(), Some(format!("deck {deck} is empty")));
        };
        let track = &MOCK_LIBRARY[track_index];

        match cmd {
            MockCommand::Play { .. } => {
                let sim = &mut self.decks[index];
                if sim.pos >= track.duration_s {
                    sim.pos = 0.0;
                }
                sim.playing = true;
                if self.master.is_none() {
                    self.master = Some(deck);
                }
                (self.all_states(), None)
            }
            MockCommand::Pause { .. } => {
                self.decks[index].playing = false;
                (self.all_states(), None)
            }
            MockCommand::Pitch { percent, .. } => {
                self.decks[index].pitch = percent.clamp(-MAX_PITCH_PERCENT, MAX_PITCH_PERCENT);
                (self.state(deck).into_iter().collect(), None)
            }
            MockCommand::Jump { beats, .. } => {
                let sim = &mut self.decks[index];
                sim.pos = (sim.pos + beats * 60.0 / track.bpm).clamp(0.0, track.duration_s);
                (self.state(deck).into_iter().collect(), None)
            }
            MockCommand::Eject { .. } => {
                self.decks[index] = EMPTY_DECK;
                if self.master == Some(deck) {
                    self.master = self.first_playing();
                }
                let mut events = vec![SourceEvent::TrackUnloaded { deck }];
                events.extend(self.all_states());
                (events, None)
            }
            MockCommand::Master { .. } => {
                self.master = Some(deck);
                (self.all_states(), None)
            }
            MockCommand::Load { .. } | MockCommand::Demo(_) => (Vec::new(), None), // handled above
        }
    }

    /// Advances playing decks by `dt_s` seconds of wall time.
    pub fn tick(&mut self, dt_s: f64) -> Vec<SourceEvent> {
        let mut moved = Vec::new();
        for (index, sim) in self.decks.iter_mut().enumerate() {
            let Some(track_index) = sim.track else { continue };
            if !sim.playing {
                continue;
            }
            let track = &MOCK_LIBRARY[track_index];
            sim.pos += dt_s * (1.0 + sim.pitch / 100.0);
            if sim.pos >= track.duration_s {
                sim.pos = track.duration_s;
                sim.playing = false;
            }
            moved.push(index as u8 + 1);
        }
        moved.into_iter().filter_map(|deck| self.state(deck)).collect()
    }

    fn first_playing(&self) -> Option<u8> {
        self.decks.iter().position(|sim| sim.track.is_some() && sim.playing).map(|index| index as u8 + 1)
    }

    fn state(&self, deck: u8) -> Option<SourceEvent> {
        let sim = self.decks.get(usize::from(deck).checked_sub(1)?)?;
        let track = &MOCK_LIBRARY[sim.track?];
        Some(SourceEvent::DeckState(DeckStateEvent {
            deck,
            pos_s: sim.pos,
            bpm_current: track.bpm * (1.0 + sim.pitch / 100.0),
            playing: sim.playing,
            is_master: self.master == Some(deck),
            on_air: sim.playing, // the mock has no mixer: a playing deck counts as audible
        }))
    }

    fn all_states(&self) -> Vec<SourceEvent> {
        (1..=self.deck_count()).filter_map(|deck| self.state(deck)).collect()
    }
}

// ---------------------------------------------------------------------------
// Session files: timed commands, one per line: "<seconds> <command>".
// "#" starts a comment. "restart" loops the session from the top.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SessionStep {
    pub t: f64,
    /// `None` means "restart".
    pub command: Option<MockCommand>,
}

pub fn parse_session(text: &str) -> Result<Vec<SessionStep>, String> {
    let mut steps: Vec<SessionStep> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let number = index + 1;
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let malformed = || format!("session line {number}: expected \"<seconds> <command>\"");
        let (time, rest) = line.split_once(char::is_whitespace).ok_or_else(malformed)?;
        let t = time.parse::<f64>().ok().filter(|t| t.is_finite() && *t >= 0.0).ok_or_else(malformed)?;
        if steps.last().is_some_and(|last| t < last.t) {
            return Err(format!("session line {number}: time goes backwards"));
        }
        let rest = rest.trim();
        let command = if rest == "restart" {
            None
        } else {
            Some(parse_command(rest).map_err(|error| format!("session line {number}: {error}"))?)
        };
        steps.push(SessionStep { t, command });
    }
    Ok(steps)
}

/// Plays a session as time advances.
pub struct SessionPlayer {
    steps: Vec<SessionStep>,
    t: f64,
    index: usize,
}

impl SessionPlayer {
    pub fn new(steps: Vec<SessionStep>) -> Self {
        Self { steps, t: 0.0, index: 0 }
    }

    pub fn reset(&mut self) {
        self.t = 0.0;
        self.index = 0;
    }

    /// Returns the commands that became due during the last `dt_s` seconds.
    pub fn advance(&mut self, dt_s: f64) -> Vec<MockCommand> {
        let mut due = Vec::new();
        self.t += dt_s;
        while let Some(step) = self.steps.get(self.index).copied() {
            if step.t > self.t {
                break;
            }
            self.index += 1;
            match step.command {
                Some(command) => due.push(command),
                None => {
                    self.reset();
                    break;
                }
            }
        }
        due
    }
}

// ---------------------------------------------------------------------------
// MockSource: the engine as a running DeckSource
// ---------------------------------------------------------------------------

struct MockState {
    engine: MockEngine,
    player: Option<SessionPlayer>,
    demo: bool,
    events: Option<mpsc::UnboundedSender<SourceEvent>>,
}

impl MockState {
    /// Sends events on. Returns false once the receiver has gone away.
    fn emit(&mut self, events: Vec<SourceEvent>) -> bool {
        match self.events.as_ref() {
            Some(sender) => events.into_iter().all(|event| sender.send(event).is_ok()),
            None => true,
        }
    }

    fn run(&mut self, cmd: MockCommand) -> Result<(), String> {
        if let MockCommand::Demo(on) = cmd {
            let Some(player) = self.player.as_mut() else {
                return Err("no session loaded".to_string());
            };
            if on {
                player.reset();
            }
            self.demo = on;
            return Ok(());
        }
        let (events, error) = self.engine.apply(cmd);
        self.emit(events);
        match error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn advance(&mut self, dt_s: f64) -> bool {
        let due = match (self.demo, self.player.as_mut()) {
            (true, Some(player)) => player.advance(dt_s),
            _ => Vec::new(),
        };
        for cmd in due {
            if let Err(error) = self.run(cmd) {
                tracing::warn!("mock session: {error}");
            }
        }
        let events = self.engine.tick(dt_s);
        self.emit(events)
    }
}

pub struct MockSource {
    state: Arc<Mutex<MockState>>,
}

/// Control handle for a [`MockSource`]; cheap to clone.
#[derive(Clone)]
pub struct MockHandle {
    state: Arc<Mutex<MockState>>,
}

impl MockSource {
    /// `session` is the text of a session file. `demo` starts it playing.
    pub fn new(deck_count: u8, session: Option<&str>, demo: bool) -> Result<Self, String> {
        Self::starting_at(deck_count, session, demo, 0)
    }

    /// Like [`MockSource::new`]; `first_track` as in [`MockEngine::starting_at`].
    pub fn starting_at(deck_count: u8, session: Option<&str>, demo: bool, first_track: usize) -> Result<Self, String> {
        let player = match session {
            Some(text) => Some(SessionPlayer::new(parse_session(text)?)),
            None => None,
        };
        let demo = demo && player.is_some();
        let state = MockState { engine: MockEngine::starting_at(deck_count, first_track), player, demo, events: None };
        Ok(Self { state: Arc::new(Mutex::new(state)) })
    }

    pub fn handle(&self) -> MockHandle {
        MockHandle { state: Arc::clone(&self.state) }
    }
}

impl MockHandle {
    /// Runs one command line. Errors are messages meant for the user.
    pub fn command(&self, line: &str) -> Result<(), String> {
        let cmd = parse_command(line)?;
        let result = lock(&self.state).run(cmd);
        result
    }

    pub fn demo(&self) -> bool {
        lock(&self.state).demo
    }

    pub fn deck_count(&self) -> u8 {
        lock(&self.state).engine.deck_count()
    }
}

impl DeckSource for MockSource {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn spawn(self: Box<Self>, events: mpsc::UnboundedSender<SourceEvent>) -> JoinHandle<()> {
        let MockSource { state } = *self;
        // Wire up the event channel before returning, so a command issued
        // right after spawn() cannot lose its events.
        {
            let mut guard = lock(&state);
            let status = SourceStatus {
                software: "mock".to_string(),
                software_version: env!("CARGO_PKG_VERSION").to_string(),
                connected: true,
                decks: guard.engine.deck_count(),
                reader: "mock".to_string(),
                reader_status: "running".to_string(),
            };
            let _ = events.send(SourceEvent::Status(status));
            guard.events = Some(events);
        }

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs_f64(1.0 / TICK_HZ));
            interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
            let mut last = Instant::now();
            loop {
                interval.tick().await;
                let now = Instant::now();
                let dt_s = now.duration_since(last).as_secs_f64();
                last = now;
                let alive = lock(&state).advance(dt_s);
                if !alive {
                    break;
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(engine: &mut MockEngine, line: &str) -> Vec<SourceEvent> {
        let cmd = parse_command(line).unwrap_or_else(|error| panic!("{line}: {error}"));
        let (events, error) = engine.apply(cmd);
        assert_eq!(error, None, "{line}");
        events
    }

    fn state_of(events: &[SourceEvent], deck: u8) -> DeckStateEvent {
        events
            .iter()
            .find_map(|event| match event {
                SourceEvent::DeckState(state) if state.deck == deck => Some(state.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no state for deck {deck}"))
    }

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} vs {b}");
    }

    #[test]
    fn parse_command_accepts_the_documented_commands() {
        assert_eq!(parse_command("load 1"), Ok(MockCommand::Load { deck: 1, track: None }));
        assert_eq!(parse_command("load 2 3"), Ok(MockCommand::Load { deck: 2, track: Some(2) }));
        assert_eq!(parse_command("  PLAY   4 "), Ok(MockCommand::Play { deck: 4 }));
        assert_eq!(parse_command("pause 1"), Ok(MockCommand::Pause { deck: 1 }));
        assert_eq!(parse_command("pitch 1 1.6129"), Ok(MockCommand::Pitch { deck: 1, percent: 1.6129 }));
        assert_eq!(parse_command("pitch 1 -2"), Ok(MockCommand::Pitch { deck: 1, percent: -2.0 }));
        assert_eq!(parse_command("jump 1 -16"), Ok(MockCommand::Jump { deck: 1, beats: -16.0 }));
        assert_eq!(parse_command("eject 2"), Ok(MockCommand::Eject { deck: 2 }));
        assert_eq!(parse_command("master 2"), Ok(MockCommand::Master { deck: 2 }));
        assert_eq!(parse_command("demo off"), Ok(MockCommand::Demo(false)));
    }

    #[test]
    fn parse_command_rejects_bad_input() {
        for line in [
            "", "play", "play 0", "play 5", "play x", "play 1 2", "load 1 9", "pitch 1", "pitch 1 fast", "pitch 1 inf",
            "jump 1", "demo", "demo maybe", "demo on now", "scratch 1",
        ] {
            assert!(parse_command(line).is_err(), "\"{line}\" should be rejected");
        }
    }

    #[test]
    fn starting_at_picks_the_first_track_and_wraps_around() {
        let mut engine = MockEngine::starting_at(2, MOCK_LIBRARY.len() - 1);
        match &run(&mut engine, "load 1")[0] {
            SourceEvent::TrackLoaded { meta, .. } => assert_eq!(meta.title, MOCK_LIBRARY[MOCK_LIBRARY.len() - 1].title),
            other => panic!("unexpected event: {other:?}"),
        }
        match &run(&mut engine, "load 2")[0] {
            SourceEvent::TrackLoaded { meta, .. } => assert_eq!(meta.title, "Neon Tide"),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn load_cycles_through_the_library_and_starts_paused_at_zero() {
        let mut engine = MockEngine::new(2);
        let first = run(&mut engine, "load 1");
        match &first[0] {
            SourceEvent::TrackLoaded { deck: 1, meta, file_path: None } => assert_eq!(meta.title, "Neon Tide"),
            other => panic!("unexpected first event: {other:?}"),
        }
        let state = state_of(&first, 1);
        close(state.pos_s, 0.0);
        assert!(!state.playing);
        close(state.bpm_current, 124.0);
        match &run(&mut engine, "load 2")[0] {
            SourceEvent::TrackLoaded { meta, .. } => assert_eq!(meta.title, "Glass Garden"),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn commands_on_an_empty_or_missing_deck_report_an_error() {
        let mut engine = MockEngine::new(2);
        assert_eq!(engine.apply(MockCommand::Play { deck: 1 }).1, Some("deck 1 is empty".to_string()));
        assert_eq!(
            engine.apply(MockCommand::Load { deck: 3, track: None }).1,
            Some("this session has 2 decks".to_string())
        );
    }

    #[test]
    fn playback_advances_with_pitch_and_stops_at_the_end() {
        let mut engine = MockEngine::new(2);
        run(&mut engine, "load 1 1");
        run(&mut engine, "play 1");
        close(state_of(&engine.tick(1.0), 1).pos_s, 1.0);
        close(state_of(&run(&mut engine, "pitch 1 10"), 1).bpm_current, 136.4);
        close(state_of(&engine.tick(1.0), 1).pos_s, 2.1);
        let end = state_of(&engine.tick(10_000.0), 1);
        close(end.pos_s, 372.4);
        assert!(!end.playing);
        assert!(engine.tick(1.0).is_empty()); // paused decks send nothing on tick
        close(state_of(&run(&mut engine, "play 1"), 1).pos_s, 0.0); // play at the end restarts
    }

    #[test]
    fn pitch_is_clamped() {
        let mut engine = MockEngine::new(2);
        run(&mut engine, "load 1 1");
        close(state_of(&run(&mut engine, "pitch 1 400"), 1).bpm_current, 124.0 * 1.5);
    }

    #[test]
    fn jump_moves_by_beats_and_clamps_to_the_track() {
        let mut engine = MockEngine::new(2);
        run(&mut engine, "load 1 1"); // 124 BPM
        run(&mut engine, "play 1");
        engine.tick(20.0);
        close(state_of(&run(&mut engine, "jump 1 -16"), 1).pos_s, 20.0 - 16.0 * 60.0 / 124.0);
        close(state_of(&run(&mut engine, "jump 1 -4000"), 1).pos_s, 0.0);
        close(state_of(&run(&mut engine, "jump 1 40000"), 1).pos_s, 372.4);
    }

    #[test]
    fn master_rules() {
        let mut engine = MockEngine::new(2);
        run(&mut engine, "load 1");
        run(&mut engine, "load 2");
        assert!(state_of(&run(&mut engine, "play 1"), 1).is_master); // first deck to play
        let both = run(&mut engine, "play 2");
        assert!(state_of(&both, 1).is_master);
        assert!(!state_of(&both, 2).is_master);
        let swapped = run(&mut engine, "master 2"); // explicit change
        assert!(!state_of(&swapped, 1).is_master);
        assert!(state_of(&swapped, 2).is_master);
        let ejected = run(&mut engine, "eject 2"); // handover to the deck still playing
        assert_eq!(ejected[0], SourceEvent::TrackUnloaded { deck: 2 });
        assert!(state_of(&ejected, 1).is_master);
        assert_eq!(ejected.len(), 2);
    }

    #[test]
    fn the_demo_session_parses_and_runs_without_errors() {
        let steps = parse_session(DEMO_SESSION).expect("demo session parses");
        assert!(steps.iter().any(|step| step.command.is_none()), "has a restart");
        let mut engine = MockEngine::new(2);
        let mut player = SessionPlayer::new(steps);
        for _ in 0..60 * 100 {
            for cmd in player.advance(1.0 / 60.0) {
                assert_eq!(engine.apply(cmd).1, None, "{cmd:?}");
            }
            engine.tick(1.0 / 60.0);
        }
    }

    #[test]
    fn parse_session_reports_bad_lines() {
        assert!(parse_session("play 1").unwrap_err().contains("line 1"));
        assert!(parse_session("1 play 1\n0 pause 1").unwrap_err().contains("backwards"));
        assert!(parse_session("# ok\n2 fly 1").unwrap_err().contains("line 2"));
        assert_eq!(parse_session("# only comments\n\n"), Ok(Vec::new()));
    }

    #[test]
    fn session_player_fires_due_commands_once_and_loops_on_restart() {
        let steps = parse_session("0 load 1\n1 play 1\n2 restart").expect("parses");
        let mut player = SessionPlayer::new(steps);
        assert_eq!(player.advance(0.5), vec![MockCommand::Load { deck: 1, track: None }]);
        assert_eq!(player.advance(0.6), vec![MockCommand::Play { deck: 1 }]);
        assert_eq!(player.advance(1.0), Vec::new()); // restart
        assert_eq!(player.advance(0.1), vec![MockCommand::Load { deck: 1, track: None }]);
    }
}
