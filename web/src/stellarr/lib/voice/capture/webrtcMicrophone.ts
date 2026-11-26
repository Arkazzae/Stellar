import type { Translator } from "@/i18n";
import type { UserId } from "@/lib/api/types";
import { profileFor, usesWorkletSuppression } from "../audioProfiles";
import { captureMicrophone, type MicCapture } from "./micPipeline";
import { needsRecapture, type InputMode, type VoiceSettings } from "../settings";
import type { VoiceStateStore } from "../store";
import type { SpeechDetector } from "./vad";
import { degradedNotice, micError, micSwapFailedError } from "../rtc/webrtcErrors";
import { addSender, type Link } from "../rtc/webrtcPeer";

export interface LocalMicrophoneEnv {
  readonly selfUserId: UserId;
  /** Words what goes wrong, in the language the person has now. */
  readonly translate: Translator;
  readonly store: VoiceStateStore;
  /** Lights the speaking ring from the audio before the input gate. */
  readonly speech: SpeechDetector;
  /** The stream the microphone is sent in, shared with the camera so the two play in sync. */
  voiceStream(): MediaStream;
  /** The links the microphone is sent on. */
  links(): Iterable<Link>;
  /** Cap every microphone sender at the adaptive controller's ceiling. */
  applyEncoding(): Promise<void>;
  /** Replace a link whose sender refused to restore the surviving microphone. */
  rebuildLink(link: Link): Promise<void>;
  /** The device ended underneath the call, e.g. a headset was unplugged. */
  onDeviceLost(): void;
  /** `talking` or `transmits` changed: push to talk opened or closed the microphone. */
  talkingChanged(): void;
}

interface MicrophoneTarget {
  readonly link: Link;
  readonly sender: RTCRtpSender;
}

/**
 * How long push to talk keeps the microphone open after its key comes up.
 *
 * The gate sends what it lets through 80 ms late (its lookahead, see
 * `GATE_DEFAULTS`), so the first 80 ms of any tail only let out what was said
 * before the key came up; anything shorter would cut the last word. 200 ms
 * leaves another 120 ms after the release for the end of that word, a final
 * consonant or a vowel dying away, and the gate's own 90 ms release then
 * fades it rather than snapping it off. Much longer and the breath after the
 * sentence goes out with it.
 */
export const TALK_RELEASE_TAIL_MS = 200;

/**
 * The microphone this client sends: its capture, mute state, and the swap to
 * a new capture without leaving the call.
 *
 * The capture owns every disposable thing behind it, so replacing it is a
 * matter of building a new one and releasing the old; a leaked device handle
 * would keep the operating system's recording indicator lit.
 */
export class LocalMicrophone {
  private active: MicCapture | null = null;
  /** The settings that produced `active`. */
  private activeSettings: VoiceSettings | null = null;
  /** The mode chosen now, which may be newer than `activeSettings`. */
  private pushToTalk = false;
  private keyHeld = false;
  /** Running while the microphone stays open after the key came up. */
  private tail: ReturnType<typeof setTimeout> | null = null;
  /** What `talking` and `transmits` were when last reported. */
  private shownTalking = false;
  private shownTransmits = true;
  /**
   * Captures whose gate push to talk holds, or which were made for push to
   * talk and so start held shut: back in voice activity mode, each has to be
   * handed back to its detector.
   */
  private readonly heldGates = new WeakSet<MicCapture>();

  constructor(private readonly env: LocalMicrophoneEnv) {}

  get capture(): MicCapture | null {
    return this.active;
  }

  /** Push to talk lets the microphone through: its key is down, or just came up. */
  get talking(): boolean {
    return this.pushToTalk && (this.keyHeld || this.tail !== null);
  }

  /** False only while push to talk keeps the microphone shut; a mute is separate. */
  get transmits(): boolean {
    return !this.pushToTalk || this.talking;
  }

  /**
   * Follow the chosen mode at once, ahead of any reopen it may need. Either
   * way the talk key starts released: a mode left while talking closes the
   * microphone, and one entered opens nothing until the key is pressed.
   */
  setInputMode(mode: InputMode): void {
    const pushToTalk = mode === "pushToTalk";
    if (pushToTalk === this.pushToTalk) return;
    this.pushToTalk = pushToTalk;
    this.keyHeld = false;
    this.stopTail();
    this.applyTalk();
  }

  /**
   * The talk key went down or came up. Coming up keeps the microphone open
   * for `TALK_RELEASE_TAIL_MS` unless `cut`; going down again within that
   * time simply carries on. Ignored in voice activity mode.
   */
  setTalking(held: boolean, cut = false): void {
    if (!this.pushToTalk) return;
    const tailing = this.tail !== null;
    if (held === this.keyHeld && !(cut && tailing)) return;
    const wasOpen = this.talking;
    this.keyHeld = held;
    this.stopTail();
    if (!held && !cut && wasOpen) {
      this.tail = setTimeout(() => {
        this.tail = null;
        this.applyTalk();
      }, TALK_RELEASE_TAIL_MS);
    }
    this.applyTalk();
  }

  /** Start sending `capture`, which was opened with `settings`. */
  adopt(capture: MicCapture, settings: VoiceSettings): void {
    this.active = capture;
    this.activeSettings = settings;
    capture.rawTrack.contentHint = profileFor(settings).contentHint;
    this.noteBuiltFor(capture, settings);
    this.applyMute(capture, this.env.store.getSnapshot().muted);
    // The detector listens before the gate: the ring must light from the same
    // decision that opens the gate, not from audio the gate already cut.
    void this.env.speech.add(this.env.selfUserId, capture.monitorStream, true);

    this.env.store.set({
      noiseSuppressionActive: usesWorkletSuppression(capture.noiseSuppression),
      // Reports what is actually running, not what was requested. Claiming
      // "browser suppression took over" while the device is running with
      // suppression switched off would be worse than saying nothing.
      degraded: degradedNotice(
        this.env.translate,
        settings.noiseSuppression,
        capture.noiseSuppression,
        // Push to talk without a gate still keeps the microphone shut, by
        // switching the track, so a missing gate is news only to the gate.
        {
          requested: settings.inputGate && settings.inputMode !== "pushToTalk",
          active: capture.gateActive,
        },
        { requested: settings.autoGainControl, active: capture.autoGain },
      ),
    });

    capture.rawTrack.onended = () => this.env.onDeviceLost();
  }

  /** Put the microphone on a link that was just made. */
  addTo(link: Link): void {
    const capture = this.active;
    if (!capture || link.micSender) return;
    try {
      link.micSender = addSender(link, capture.track, this.env.voiceStream());
    } catch {
      // A connection that cannot take the track is about to be rebuilt.
    }
  }

  /** Stop the device and forget the capture, and the talk key with it: the next call starts released. */
  release(): void {
    this.env.speech.remove(this.env.selfUserId);
    if (this.active) this.active.rawTrack.onended = null;
    this.active?.dispose();
    this.active = null;
    this.activeSettings = null;
    this.keyHeld = false;
    this.stopTail();
    this.applyTalk();
  }

  /** Only the microphone: shared system sound is content, not the person's voice. */
  setMuted(muted: boolean): void {
    if (this.active) this.applyMute(this.active, muted);
  }

  /** Retune the running capture without reopening the device. */
  tune(settings: VoiceSettings): void {
    this.active?.setSensitivity(settings.vadSensitivity);
    this.active?.setInputGain(settings.inputGainDb);
  }

  /** True when `next` needs a capture different from the one running. */
  needsReopen(next: VoiceSettings): boolean {
    return this.activeSettings !== null && needsRecapture(this.activeSettings, next);
  }

  /**
   * Wake a capture graph the browser suspended. A suspended graph sends a
   * valid but silent track, so false means the device has to be reopened.
   */
  async resume(): Promise<boolean> {
    const capture = this.active;
    if (!capture) return true;
    return (await capture.resume()) || !capture.processed;
  }

  /**
   * Reopen the microphone for `next` and move the sender onto it.
   *
   * Without `force`, only a capture-affecting change does anything. If the new
   * device cannot be opened, or the sender cannot take it, the old capture is
   * still running, so the call goes on and the error is reported as
   * recoverable rather than fatal. A newer request discards the capture, undoing
   * sender changes while the previous capture still owns the call. A capture
   * belonging to a later call is never changed. Resolves to whether a new
   * capture took over.
   */
  async replace(
    next: VoiceSettings,
    force: boolean,
    superseded: () => boolean,
  ): Promise<boolean> {
    const previous = this.active;
    if (!previous || (!force && !this.needsReopen(next))) return false;

    let capture: MicCapture;
    try {
      capture = await captureMicrophone(next, { translate: this.env.translate });
    } catch (cause) {
      if (this.active === previous && !superseded()) {
        this.env.store.set({ error: { ...micError(this.env.translate, cause), recoverable: true } });
      }
      return false;
    }
    if (this.active !== previous || superseded()) {
      capture.dispose();
      return false;
    }

    // Before the track reaches any sender: a capture for push to talk goes
    // on the wire shut unless the key is down.
    this.noteBuiltFor(capture, next);
    this.applyMute(capture, this.env.store.getSnapshot().muted);
    // Up from silence rather than straight in at level: a suppressor or gate
    // change swaps the track under a running call, and two graphs never meet
    // at the same sample or settle at the same loudness.
    capture.fadeIn();
    const targets = [...this.env.links()].flatMap((link) =>
      link.micSender ? [{ link, sender: link.micSender }] : [],
    );
    if (!(await this.moveAllLinks(capture.track, previous, targets))) {
      capture.dispose();
      if (this.active === previous && !superseded()) {
        this.env.store.set({ error: micSwapFailedError(this.env.translate) });
      }
      return false;
    }
    // A newer settings request needs its old microphone back. A later call
    // already has its own capture and senders, which this operation cannot own.
    if (this.active !== previous || superseded()) {
      try {
        await this.moveBackTo(previous, targets);
      } finally {
        capture.dispose();
      }
      return false;
    }

    this.adopt(capture, next);
    previous.rawTrack.onended = null;
    previous.dispose();
    this.env.store.set({ error: null });
    return true;
  }

  /**
   * Restore only the senders this swap moved, while its original capture is
   * still active. Recheck ownership after each await: leaving and rejoining
   * must not restore a disposed track or rebuild links of a later call.
   */
  private async moveBackTo(
    previous: MicCapture,
    targets: readonly MicrophoneTarget[],
  ): Promise<void> {
    for (const { link, sender } of targets) {
      if (this.active !== previous) return;
      if (link.micSender !== sender || ![...this.env.links()].includes(link)) continue;
      try {
        await sender.replaceTrack(previous.track);
      } catch {
        if (
          this.active === previous &&
          link.micSender === sender &&
          [...this.env.links()].includes(link)
        ) {
          await this.env.rebuildLink(link);
        }
      }
    }
  }

  /**
   * Swap the track on every sender, all or nothing.
   *
   * Successful swaps are rolled back when one fails. A sender that refuses
   * even the rollback belongs to a broken connection, which is rebuilt while
   * `active` still points at the old capture.
   */
  private async moveAllLinks(
    track: MediaStreamTrack,
    previous: MicCapture,
    targets: readonly MicrophoneTarget[],
  ): Promise<boolean> {
    const attempts = await Promise.allSettled(
      targets.map(({ sender }) => sender.replaceTrack(track)),
    );
    if (attempts.every((result) => result.status === "fulfilled")) {
      if (this.active === previous) await this.env.applyEncoding();
      return true;
    }

    const moved = targets.filter((_, index) => attempts[index]?.status === "fulfilled");
    await this.moveBackTo(previous, moved);
    return false;
  }

  /**
   * What the capture lets out: nothing while muted, and in push to talk
   * nothing while the key is up.
   *
   * Push to talk works at the gate when there is one: it holds the gate the
   * detector would otherwise open, which keeps the lookahead (the syllable
   * said with the press is not clipped) and the ramps (a press does not
   * click). Without a gate in the path, which a browser without AudioWorklet
   * leaves, the track itself is switched instead, so the key still decides.
   * The mute is the track switch either way, and wins over the key.
   */
  private applyMute(capture: MicCapture, muted: boolean): void {
    if (capture.gateActive) {
      if (this.pushToTalk) {
        capture.holdGate(this.talking);
        this.heldGates.add(capture);
      } else if (this.heldGates.has(capture)) {
        capture.holdGate(null);
        this.heldGates.delete(capture);
      }
    }
    const enabled = !muted && (capture.gateActive || this.transmits);
    capture.rawTrack.enabled = enabled;
    if (capture.track !== capture.rawTrack) capture.track.enabled = enabled;
  }

  /** A capture made for push to talk has its gate held shut from the start. */
  private noteBuiltFor(capture: MicCapture, settings: VoiceSettings): void {
    if (capture.gateActive && settings.inputMode === "pushToTalk") this.heldGates.add(capture);
  }

  private stopTail(): void {
    if (this.tail === null) return;
    clearTimeout(this.tail);
    this.tail = null;
  }

  /** Puts the key's state on the running capture, and says when `talking` or `transmits` changed. */
  private applyTalk(): void {
    if (this.active) this.applyMute(this.active, this.env.store.getSnapshot().muted);
    const { talking, transmits } = this;
    if (talking === this.shownTalking && transmits === this.shownTransmits) return;
    this.shownTalking = talking;
    this.shownTransmits = transmits;
    this.env.talkingChanged();
  }
}
