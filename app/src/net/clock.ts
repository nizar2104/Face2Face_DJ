// Clock sync (brief 7.6): the Viewer pings, the Sender answers with its own
// monotonic time. We keep the last N samples and trust the one with the
// smallest round trip, because its midpoint estimate has the least error.
// All times are seconds on monotonic clocks.

export interface ClockSample {
  /** sender_time = viewer_time + offset */
  offset: number;
  rtt: number;
}

export const CLOCK_WINDOW = 20;

export class ClockSync {
  private samples: ClockSample[] = [];
  private readonly window: number;

  constructor(window: number = CLOCK_WINDOW) {
    this.window = window;
  }

  /**
   * @param tSend   viewer clock when the ping left
   * @param tSender sender clock when it answered
   * @param tRecv   viewer clock when the pong arrived
   */
  addSample(tSend: number, tSender: number, tRecv: number): void {
    const rtt = tRecv - tSend;
    if (!Number.isFinite(rtt) || !Number.isFinite(tSender) || rtt < 0) return;
    this.samples.push({ offset: tSender - (tSend + rtt / 2), rtt });
    if (this.samples.length > this.window) this.samples.shift();
  }

  /** The minimum-RTT sample in the window, or null before the first pong. */
  get best(): ClockSample | null {
    let best: ClockSample | null = null;
    for (const s of this.samples) {
      if (best === null || s.rtt < best.rtt) best = s;
    }
    return best;
  }

  get synced(): boolean {
    return this.samples.length > 0;
  }

  /** Converts a viewer-clock time to the sender clock. Identity until synced. */
  toSender(tViewer: number): number {
    const b = this.best;
    return b === null ? tViewer : tViewer + b.offset;
  }

  /** Converts a sender-clock time to the viewer clock. Identity until synced. */
  toViewer(tSender: number): number {
    const b = this.best;
    return b === null ? tSender : tSender - b.offset;
  }

  /** How many samples are in the window. */
  get sampleCount(): number {
    return this.samples.length;
  }

  /** One-way network latency estimate in milliseconds (half the best RTT). */
  get latencyMs(): number | null {
    const b = this.best;
    return b === null ? null : (b.rtt / 2) * 1000;
  }

  reset(): void {
    this.samples = [];
  }
}
