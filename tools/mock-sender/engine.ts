// Simulated DJ decks. No I/O and no clock: the caller applies commands and
// advances time, and gets back the events a real source adapter would produce.
// The Rust twin is crates/face2face-core/src/sources/mock.rs; keep the command
// vocabulary and the rules below identical in both.

export interface MockTrack {
  title: string;
  artist: string;
  album: string;
  duration_s: number;
  bpm: number;
  camelot: string;
  musical: string;
  file_name: string;
}

/** Invented tracks. No real files are involved in Milestone 0. */
export const MOCK_LIBRARY: readonly MockTrack[] = [
  { title: 'Neon Tide', artist: 'Mara Voss', album: 'Mock Sessions', duration_s: 372.4, bpm: 124, camelot: '8A', musical: 'Am', file_name: 'Mara Voss - Neon Tide.flac' },
  { title: 'Glass Garden', artist: 'Okta & Finch', album: 'Mock Sessions', duration_s: 341.0, bpm: 126, camelot: '9A', musical: 'Em', file_name: 'Okta & Finch - Glass Garden.mp3' },
  { title: 'Low Orbit', artist: 'Hadley Kru', album: 'Mock Sessions', duration_s: 405.5, bpm: 122, camelot: '7A', musical: 'Dm', file_name: 'Hadley Kru - Low Orbit.wav' },
  { title: 'Paper Suns', artist: 'Ilse Marten', album: 'Mock Sessions', duration_s: 298.2, bpm: 128, camelot: '10B', musical: 'D', file_name: 'Ilse Marten - Paper Suns.aiff' },
];

export const MAX_PITCH_PERCENT = 50;

export type MockCommand =
  | { kind: 'load'; deck: number; track: number | null }
  | { kind: 'play'; deck: number }
  | { kind: 'pause'; deck: number }
  | { kind: 'pitch'; deck: number; percent: number }
  | { kind: 'jump'; deck: number; beats: number }
  | { kind: 'eject'; deck: number }
  | { kind: 'master'; deck: number }
  | { kind: 'demo'; on: boolean };

export const COMMAND_HELP = `Commands (deck is 1-4):
  load <deck> [track]    load the next mock track, or track number 1-${MOCK_LIBRARY.length}
  play <deck>            start playback
  pause <deck>           stop playback
  pitch <deck> <pct>     tempo change in percent, e.g. 1.6129 or -2
  jump <deck> <beats>    move by beats, e.g. -16 = a 4-bar loop jump back
  eject <deck>           unload the deck
  master <deck>          make the deck the tempo master
  demo on|off            run or stop the scripted demo session`;

/** Parses one command line. Returns a string describing the problem on failure. */
export function parseCommand(line: string): MockCommand | string {
  const parts = line.trim().split(/\s+/);
  const name = (parts[0] ?? '').toLowerCase();
  if (name === 'demo') {
    if (parts.length === 2 && (parts[1] === 'on' || parts[1] === 'off')) return { kind: 'demo', on: parts[1] === 'on' };
    return 'usage: demo on|off';
  }
  const deck = Number(parts[1]);
  if (!Number.isInteger(deck) || deck < 1 || deck > 4) return `"${line.trim()}": expected a deck number 1-4`;
  const arg = parts[2];
  switch (name) {
    case 'load': {
      if (parts.length === 2) return { kind: 'load', deck, track: null };
      const track = Number(arg);
      if (parts.length !== 3 || !Number.isInteger(track) || track < 1 || track > MOCK_LIBRARY.length) {
        return `usage: load <deck> [track 1-${MOCK_LIBRARY.length}]`;
      }
      return { kind: 'load', deck, track: track - 1 };
    }
    case 'play':
    case 'pause':
    case 'eject':
    case 'master':
      if (parts.length !== 2) return `usage: ${name} <deck>`;
      return { kind: name, deck };
    case 'pitch': {
      const percent = Number(arg);
      if (parts.length !== 3 || arg === '' || !Number.isFinite(percent)) return 'usage: pitch <deck> <percent>';
      return { kind: 'pitch', deck, percent };
    }
    case 'jump': {
      const beats = Number(arg);
      if (parts.length !== 3 || arg === '' || !Number.isFinite(beats)) return 'usage: jump <deck> <beats>';
      return { kind: 'jump', deck, beats };
    }
    default:
      return `unknown command "${name}"`;
  }
}

export type MockEvent =
  | { kind: 'loaded'; deck: number; track: MockTrack; trackIndex: number }
  | { kind: 'state'; deck: number; pos_s: number; bpm_current: number; playing: boolean; is_master: boolean; on_air: boolean }
  | { kind: 'unloaded'; deck: number };

interface DeckSim {
  track: MockTrack | null;
  trackIndex: number;
  pos: number;
  playing: boolean;
  pitch: number;
}

export interface ApplyResult {
  events: MockEvent[];
  error: string | null;
}

export class MockEngine {
  readonly deckCount: number;
  private decks: DeckSim[];
  private master: number | null = null;
  private nextTrack: number;

  /** `firstTrack`: library index the first `load` without a track number takes (as `starting_at` in Rust). */
  constructor(deckCount = 2, firstTrack = 0) {
    this.deckCount = deckCount;
    this.nextTrack = firstTrack;
    this.decks = Array.from({ length: deckCount }, () => ({ track: null, trackIndex: 0, pos: 0, playing: false, pitch: 0 }));
  }

  apply(cmd: MockCommand): ApplyResult {
    if (cmd.kind === 'demo') return { events: [], error: null }; // handled by the caller
    const sim = this.decks[cmd.deck - 1];
    if (sim === undefined) return { events: [], error: `this session has ${this.deckCount} decks` };
    const deck = cmd.deck;

    if (cmd.kind === 'load') {
      const index = cmd.track ?? this.nextTrack++ % MOCK_LIBRARY.length;
      const track = MOCK_LIBRARY[index]!;
      sim.track = track;
      sim.trackIndex = index;
      sim.pos = 0;
      sim.playing = false;
      sim.pitch = 0;
      if (this.master === deck) this.master = this.firstPlaying();
      return { events: [{ kind: 'loaded', deck, track, trackIndex: index }, ...this.allStates()], error: null };
    }

    const track = sim.track;
    if (track === null) return { events: [], error: `deck ${deck} is empty` };

    switch (cmd.kind) {
      case 'play':
        if (sim.pos >= track.duration_s) sim.pos = 0;
        sim.playing = true;
        if (this.master === null) this.master = deck;
        return { events: this.allStates(), error: null };
      case 'pause':
        sim.playing = false;
        return { events: this.allStates(), error: null };
      case 'pitch':
        sim.pitch = Math.min(Math.max(cmd.percent, -MAX_PITCH_PERCENT), MAX_PITCH_PERCENT);
        return { events: [this.state(deck, sim, track)], error: null };
      case 'jump':
        sim.pos = Math.min(Math.max(sim.pos + (cmd.beats * 60) / track.bpm, 0), track.duration_s);
        return { events: [this.state(deck, sim, track)], error: null };
      case 'eject':
        sim.track = null;
        sim.playing = false;
        if (this.master === deck) this.master = this.firstPlaying();
        return { events: [{ kind: 'unloaded', deck }, ...this.allStates()], error: null };
      case 'master':
        this.master = deck;
        return { events: this.allStates(), error: null };
    }
  }

  /** Advances playing decks by `dtS` seconds of wall time. */
  tick(dtS: number): MockEvent[] {
    const events: MockEvent[] = [];
    this.decks.forEach((sim, i) => {
      const track = sim.track;
      if (track === null || !sim.playing) return;
      sim.pos += dtS * (1 + sim.pitch / 100);
      if (sim.pos >= track.duration_s) {
        sim.pos = track.duration_s;
        sim.playing = false;
      }
      events.push(this.state(i + 1, sim, track));
    });
    return events;
  }

  private firstPlaying(): number | null {
    const i = this.decks.findIndex((d) => d.track !== null && d.playing);
    return i === -1 ? null : i + 1;
  }

  private state(deck: number, sim: DeckSim, track: MockTrack): MockEvent {
    return {
      kind: 'state',
      deck,
      pos_s: sim.pos,
      bpm_current: track.bpm * (1 + sim.pitch / 100),
      playing: sim.playing,
      is_master: this.master === deck,
      on_air: sim.playing, // the mock has no mixer: a playing deck counts as audible
    };
  }

  private allStates(): MockEvent[] {
    const out: MockEvent[] = [];
    this.decks.forEach((sim, i) => {
      if (sim.track !== null) out.push(this.state(i + 1, sim, sim.track));
    });
    return out;
  }
}

// ---------------------------------------------------------------------------
// Session files: timed commands, one per line: "<seconds> <command>".
// "#" starts a comment. "restart" loops the session from the top.
// ---------------------------------------------------------------------------

export interface SessionStep {
  t: number;
  /** A MockCommand, or null for "restart". */
  command: MockCommand | null;
}

export function parseSession(text: string): SessionStep[] {
  const steps: SessionStep[] = [];
  text.split(/\r?\n/).forEach((raw, i) => {
    const line = raw.replace(/#.*/, '').trim();
    if (line === '') return;
    const m = /^(\d+(?:\.\d+)?)\s+(.+)$/.exec(line);
    if (m === null) throw new Error(`session line ${i + 1}: expected "<seconds> <command>"`);
    const t = Number(m[1]);
    const last = steps[steps.length - 1];
    if (last !== undefined && t < last.t) throw new Error(`session line ${i + 1}: time goes backwards`);
    const rest = m[2]!.trim();
    if (rest === 'restart') {
      steps.push({ t, command: null });
      return;
    }
    const cmd = parseCommand(rest);
    if (typeof cmd === 'string') throw new Error(`session line ${i + 1}: ${cmd}`);
    steps.push({ t, command: cmd });
  });
  return steps;
}

/** Plays a session against an engine as time advances. */
export class SessionPlayer {
  private readonly steps: SessionStep[];
  private t = 0;
  private index = 0;

  constructor(steps: SessionStep[]) {
    this.steps = steps;
  }

  reset(): void {
    this.t = 0;
    this.index = 0;
  }

  /** Returns the commands that became due during the last `dtS` seconds. */
  advance(dtS: number): MockCommand[] {
    const due: MockCommand[] = [];
    this.t += dtS;
    while (this.index < this.steps.length && this.steps[this.index]!.t <= this.t) {
      const step = this.steps[this.index]!;
      this.index++;
      if (step.command === null) {
        this.reset();
        break;
      }
      due.push(step.command);
    }
    return due;
  }
}
