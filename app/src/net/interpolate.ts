// Position interpolation on the Viewer (brief 7.6).
//
//   pos_est = pos_s + (now - ts) * rate      while playing
//   pos_est = pos_s                          while paused
//
// `now` and `ts` only have to be on the same clock. The store converts each
// state's sender timestamp to the viewer clock when it arrives (see store.ts).
//
// A new deck_state rarely agrees exactly with what is on screen. If the
// difference is above SNAP_THRESHOLD_S we jump (loop, hot cue, seek); below it
// we keep the old picture and bleed the difference away over SMOOTH_TIME_S, so
// network jitter never shows as a twitch. Positions may go backwards.

export const SNAP_THRESHOLD_S = 0.05;
export const SMOOTH_TIME_S = 0.1;
/** Stop extrapolating if the Sender goes quiet for this long while "playing". */
export const MAX_EXTRAPOLATION_S = 1.0;

export interface InterpInput {
  ts: number;
  pos_s: number;
  playing: boolean;
  /** bpm_current / bpm_original */
  rate: number;
}

export class DeckInterpolator {
  private anchor: InterpInput | null = null;
  private corr = 0;
  private corrStart = 0;
  private duration = Number.POSITIVE_INFINITY;

  /** Track length, used to clamp the estimate. */
  setDuration(durationS: number): void {
    this.duration = durationS > 0 ? durationS : Number.POSITIVE_INFINITY;
  }

  reset(): void {
    this.anchor = null;
    this.corr = 0;
  }

  get hasState(): boolean {
    return this.anchor !== null;
  }

  /** Feeds a new deck_state. `now` is the current time on the clock `input.ts` uses. */
  update(input: InterpInput, now: number): void {
    const rate = Number.isFinite(input.rate) && input.rate > 0 ? input.rate : 1;
    const next: InterpInput = { ...input, rate };
    if (this.anchor === null) {
      this.anchor = next;
      this.corr = 0;
      return;
    }
    const shown = this.raw(now);
    this.anchor = next;
    const err = shown - this.target(now);
    if (Math.abs(err) > SNAP_THRESHOLD_S) {
      this.corr = 0;
    } else {
      this.corr = err;
      this.corrStart = now;
    }
  }

  /** Estimated track position in seconds at time `now`, or null with no state. */
  position(now: number): number | null {
    if (this.anchor === null) return null;
    return Math.min(Math.max(this.raw(now), 0), this.duration);
  }

  private target(now: number): number {
    const a = this.anchor!;
    if (!a.playing) return a.pos_s;
    const dt = Math.min(Math.max(now - a.ts, -MAX_EXTRAPOLATION_S), MAX_EXTRAPOLATION_S);
    return a.pos_s + dt * a.rate;
  }

  private raw(now: number): number {
    let c = 0;
    if (this.corr !== 0) {
      const k = 1 - (now - this.corrStart) / SMOOTH_TIME_S;
      c = k > 0 ? this.corr * Math.min(k, 1) : 0;
    }
    return this.target(now) + c;
  }
}
