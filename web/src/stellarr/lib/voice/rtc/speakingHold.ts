/**
 * How long a voice is still shown speaking after the detector says it went
 * quiet. In a measured call the detector closed inside words for 50 to 250 ms
 * at a time, while a real pause lasted a second or more; a quarter of a
 * second bridges the one without blurring the other.
 */
export const RING_HOLD_MS = 250;

/**
 * The voices the detector has just heard go quiet, still shown speaking until
 * they have stayed quiet for `RING_HOLD_MS`. A voice that speaks again within
 * that time was never shown to stop.
 *
 * Only the detector's own decisions are held. Whatever has to end the ring at
 * once, which is a mute, a voice that is gone, a person who left and the end
 * of the call, clears it instead, so nothing is held for them and no timer
 * outlives them.
 */
export class SpeakingHold {
  private readonly timers = new Map<string, ReturnType<typeof setTimeout>>();

  /** `released` runs when a held voice is let go on its own. */
  constructor(
    private readonly released: () => void,
    private readonly holdMs = RING_HOLD_MS,
  ) {}

  /** Still shown speaking, although the detector says it stopped. */
  holds(id: string): boolean {
    return this.timers.has(id);
  }

  /** The detector heard this voice stop: keep it shown for a moment. */
  hold(id: string): void {
    if (this.timers.has(id)) return;
    this.timers.set(
      id,
      setTimeout(() => {
        this.timers.delete(id);
        this.released();
      }, this.holdMs),
    );
  }

  /** Nothing held for this voice any more; true when something was. */
  clear(id: string): boolean {
    const timer = this.timers.get(id);
    if (timer === undefined) return false;
    clearTimeout(timer);
    this.timers.delete(id);
    return true;
  }

  /** Nothing held for anyone, and every timer gone with it. */
  reset(): void {
    for (const timer of this.timers.values()) clearTimeout(timer);
    this.timers.clear();
  }
}
