import { describe, expect, it } from "vitest";

import { frameMeanSquare, framePeakAbs, meanSquareToDbfs } from "./frame";
import { clickFrame, continuousTone, FRAME, noise, SAMPLE_RATE, tone } from "./testSignals";
import { AdaptiveVad, sensitivityToThresholds, VAD_DEFAULTS, type VadConfig } from "./vad";

function vad(overrides: Partial<VadConfig> = {}) {
  return new AdaptiveVad({ sampleRate: SAMPLE_RATE, ...overrides });
}

/** A repeatable speech-like envelope: strongly modulated rather than a lab tone. */
function speechFrame(index: number): Float32Array {
  const envelope = [0.08, 0.13, 0.24, 0.4, 0.31, 0.17, 0.1, 0.05][index % 8] ?? 0.1;
  return tone(envelope, FRAME, 185 + (index % 5) * 17);
}

/** Feed `frames` frames of the given signal, returning the last result. */
function feed(detector: AdaptiveVad, make: (index: number) => Float32Array, frames: number) {
  let last = detector.processFrame(new Float32Array(FRAME));
  for (let i = 0; i < frames; i += 1) last = detector.processFrame(make(i));
  return last;
}

describe("a frame's measurements", () => {
  it("never trust a clipped, torn or silent frame", () => {
    // Samples past full scale count as full scale, not as more energy.
    expect(frameMeanSquare(new Float32Array([4, -9, 2.5, -3]))).toBe(1);
    expect(meanSquareToDbfs(frameMeanSquare(new Float32Array([1, -1, 1, -1])))).toBe(0);
    expect(framePeakAbs(new Float32Array([0.25, -4, Number.NaN]))).toBe(1);
    // A torn buffer's NaN and Infinity count as 0, not as full scale: treating
    // corruption as a loud frame would fire the indicator.
    expect(frameMeanSquare(new Float32Array([0.5, Number.NaN, 0.5, Number.POSITIVE_INFINITY]))).toBeCloseTo(0.125, 6);
    // Digital silence has a level, far below anything real.
    expect(meanSquareToDbfs(0)).toBeLessThan(-100);
    expect(Number.isFinite(meanSquareToDbfs(0))).toBe(true);
  });
});

describe("frame accumulation", () => {
  it("emits one result per completed 10 ms frame, carrying the remainder in the caller's array", () => {
    const detector = vad();
    expect(detector.frameSize).toBe(480);
    const out: never[] = [];
    // Three 128-sample render quanta are less than a frame; the fourth completes one.
    for (let quantum = 0; quantum < 3; quantum += 1) expect(detector.push(new Float32Array(128), out)).toHaveLength(0);
    expect(detector.push(new Float32Array(128), out)).toBe(out);
    expect(out).toHaveLength(1);

    const fresh = vad();
    expect(fresh.push(new Float32Array(FRAME * 3 + 7))).toHaveLength(3);
    // 7 samples are still buffered; 473 more complete the fourth frame.
    expect(fresh.push(new Float32Array(FRAME - 7))).toHaveLength(1);
  });

  it("sizes the frame for the context's rate", () => {
    expect(vad({ sampleRate: 44100 }).frameSize).toBe(441);
    expect(vad({ sampleRate: 16000 }).frameSize).toBe(160);
  });
});

describe("adaptive detection", () => {
  it("learns the room for 120 ms before judging it", () => {
    const detector = vad();
    // Without a warm-up the initial floor sits ~25 dB below any real room, so
    // the very first frame looks like a huge SNR event and latches on forever.
    let frames = 0;
    while (detector.warmingUp && frames < 100) {
      detector.processFrame(tone(0.4, FRAME));
      expect(detector.speaking).toBe(false);
      frames += 1;
    }
    expect(frames).toBe(VAD_DEFAULTS.warmupFrames);
  });

  it("does not fire on steady room noise, however long it runs, and learns a quiet floor", () => {
    const detector = vad();
    expect(feed(detector, (i) => noise(0.01, FRAME, i + 1), 500).speaking).toBe(false);

    const quiet = vad();
    feed(quiet, (i) => noise(0.0005, FRAME, i + 1), 100);
    expect(quiet.noiseFloorDb).toBeLessThan(-60);
    expect(quiet.speaking).toBe(false);
  });

  it("ignores digital silence, so an unmute cannot flash a phantom onset", () => {
    const silent = vad();
    const result = feed(silent, () => new Float32Array(FRAME), 200);
    expect(result.speaking).toBe(false);
    expect(result.level).toBeLessThan(0.05);

    const detector = vad();
    feed(detector, (i) => noise(0.01, FRAME, i + 1), 200);
    const floorBefore = detector.noiseFloorDb;
    // A muted peer's track is all zeroes. The floor must not follow it down.
    feed(detector, () => new Float32Array(FRAME), 100);
    expect(detector.noiseFloorDb).toBeCloseTo(floorBefore, 5);
    // Unmuting back into the same room is not a speaking event.
    feed(detector, (i) => noise(0.01, FRAME, i + 4000), 5);
    expect(detector.speaking).toBe(false);
  });

  it("confirms speech on the third loud frame, not on a single spike", () => {
    const detector = vad();
    feed(detector, (i) => noise(0.01, FRAME, i + 1), 200);
    detector.processFrame(tone(0.4, FRAME));
    detector.processFrame(tone(0.4, FRAME));
    expect(detector.speaking).toBe(false);
    detector.processFrame(tone(0.4, FRAME));
    expect(detector.speaking).toBe(true);
  });

  it("rejects repeated keyboard and controller-like impacts", () => {
    const detector = vad();
    feed(detector, (i) => noise(0.01, FRAME, i + 1), 200);
    for (let click = 0; click < 20; click += 1) {
      expect(detector.processFrame(clickFrame(0.45 + (click % 4) * 0.1)).transient).toBe(true);
      feed(detector, (i) => noise(0.01, FRAME, click * 20 + i + 5000), 9);
      expect(detector.speaking).toBe(false);
    }
  });

  it("opens for real speech shortly after a rejected impact", () => {
    const detector = vad();
    feed(detector, (i) => noise(0.01, FRAME, i + 1), 200);
    expect(detector.processFrame(clickFrame()).transient).toBe(true);
    feed(detector, (i) => speechFrame(i), 9);
    expect(detector.speaking).toBe(true);
  });

  it("does not let a click reuse an old speech hangover", () => {
    const detector = vad({ hangoverFrames: 45 });
    feed(detector, (i) => noise(0.01, FRAME, i + 1), 200);
    feed(detector, (i) => speechFrame(i), 10);
    // The ordinary 450 ms hangover is still open after this natural pause.
    feed(detector, (i) => noise(0.01, FRAME, i + 900), 10);
    expect(detector.speaking).toBe(true);

    const click = detector.processFrame(clickFrame());
    expect(click.transient).toBe(true);
    expect(click.speaking).toBe(false);
  });

  it("holds through a gap shorter than the hangover and releases after a longer one", () => {
    const short = vad({ hangoverFrames: 30 });
    feed(short, (i) => noise(0.01, FRAME, i + 1), 200);
    feed(short, () => tone(0.4, FRAME), 10);
    feed(short, (i) => noise(0.01, FRAME, i + 500), 20);
    expect(short.speaking).toBe(true);

    const long = vad({ hangoverFrames: 30 });
    feed(long, (i) => noise(0.01, FRAME, i + 1), 200);
    feed(long, () => tone(0.4, FRAME), 10);
    feed(long, (i) => noise(0.01, FRAME, i + 900), 40);
    expect(long.speaking).toBe(false);
  });

  it("adapts to a loud room a fixed threshold would misjudge, and still hears quiet speech in a silent one", () => {
    const loud = vad();
    expect(feed(loud, (i) => noise(0.05, FRAME, i + 1), 600).speaking).toBe(false);
    expect(loud.noiseFloorDb).toBeGreaterThan(-40);
    feed(loud, () => tone(0.5, FRAME), 5);
    expect(loud.speaking).toBe(true);

    const silent = vad();
    feed(silent, () => new Float32Array(FRAME), 300);
    // ~-46 dBFS: far under any fixed threshold, but well above this floor.
    feed(silent, () => tone(0.008, FRAME), 6);
    expect(silent.speaking).toBe(true);
  });

  it("keeps hearing a monologue: speech cannot raise the floor over itself", () => {
    const detector = vad();
    feed(detector, (i) => noise(0.01, FRAME, i + 1), 200);
    // Thirty seconds of continuous speech, the case that deafens a naive tracker.
    const speech = feed(detector, (i) => speechFrame(i), 3000);
    expect(detector.speaking).toBe(true);
    expect(speech.snrDb).toBeGreaterThan(9.5);
  });

  it("recovers from a new steady noise source within three seconds", () => {
    const detector = vad();
    feed(detector, (i) => noise(0.005, FRAME, i + 1), 200);
    feed(detector, () => tone(0.4, FRAME), 10);
    expect(detector.speaking).toBe(true);
    // Someone starts a fan, loud enough to clear the release threshold against
    // the old floor. The ring must not stay on for minutes.
    feed(detector, (i) => noise(0.12, FRAME, i + 7000), 300);
    expect(detector.speaking).toBe(false);
  });

  it("keeps the floor inside its clamps and the level inside 0..1", () => {
    const silent = vad();
    feed(silent, () => new Float32Array(FRAME), 50);
    expect(silent.noiseFloorDb).toBeGreaterThanOrEqual(VAD_DEFAULTS.noiseFloorMinDb);

    const loud = vad();
    const loudResult = feed(loud, () => tone(1, FRAME), 500);
    expect(loud.noiseFloorDb).toBeLessThanOrEqual(VAD_DEFAULTS.noiseFloorMaxDb);
    expect(loudResult.level).toBeLessThanOrEqual(1);

    const quietResult = feed(vad(), () => tone(0.02, FRAME), 40);
    expect(quietResult.level).toBeGreaterThanOrEqual(0);
    expect(feed(vad(), () => tone(0.6, FRAME), 40).level).toBeGreaterThan(quietResult.level);
  });

  it("returns to its initial state on reset", () => {
    const detector = vad();
    feed(detector, (i) => noise(0.01, FRAME, i + 1), 200);
    feed(detector, () => tone(0.5, FRAME), 10);
    expect(detector.speaking).toBe(true);

    detector.reset();
    expect(detector.speaking).toBe(false);
    expect(detector.level).toBe(0);
    expect(detector.noiseFloorDb).toBe(VAD_DEFAULTS.noiseFloorInitDb);
    expect(detector.warmingUp).toBe(true);
  });
});

describe("pitch, not loudness", () => {
  /** The near-silent floor a neural suppressor leaves between words. */
  function suppressedRoom(detector: AdaptiveVad) {
    feed(detector, (i) => noise(0.0002, FRAME, i + 1), 200);
  }

  it("never opens for loud broadband crunching, however long it lasts", () => {
    const detector = vad();
    suppressedRoom(detector);
    // Bursts of crisps: 200 ms on, 200 ms off, for three seconds, at speech level.
    for (let i = 0; i < 300; i += 1) {
      const chewing = Math.floor(i / 20) % 2 === 0;
      const result = detector.processFrame(chewing ? noise(0.2, FRAME, i + 3) : noise(0.0002, FRAME, i + 900));
      expect(result.speaking).toBe(false);
    }
  });

  it("opens for a vowel at the energy a crunch is refused at, and says why", () => {
    const voiced = vad();
    suppressedRoom(voiced);
    let last = voiced.processFrame(new Float32Array(FRAME));
    for (const frame of continuousTone(120, 0.05, 6)) last = voiced.processFrame(frame);
    expect(voiced.speaking).toBe(true);
    expect(last.voicing).toBeGreaterThan(VAD_DEFAULTS.voicingOn);
    expect(last.voiced).toBe(true);
    expect(last.peak).toBeCloseTo(0.05, 2);

    const unvoiced = vad();
    suppressedRoom(unvoiced);
    feed(unvoiced, (i) => noise(0.05, FRAME, i + 3), 6);
    expect(unvoiced.speaking).toBe(false);
  });

  it("opens on a word that begins with an unvoiced consonant", () => {
    const detector = vad();
    suppressedRoom(detector);
    // 80 ms of "sz", then the vowel.
    feed(detector, (i) => noise(0.05, FRAME, i + 5), 8);
    expect(detector.speaking).toBe(false);
    for (const frame of continuousTone(120, 0.05, 4)) detector.processFrame(frame);
    expect(detector.speaking).toBe(true);
  });

  it("closes once speech has turned into crunching, whatever the hangover says", () => {
    const detector = vad();
    suppressedRoom(detector);
    for (const frame of continuousTone(120, 0.1, 30)) detector.processFrame(frame);
    // The hangover carries the first crunches, as it must carry a trailing "s"...
    feed(detector, (i) => noise(0.2, FRAME, i + 3), 20);
    expect(detector.speaking).toBe(true);
    // ...but not a whole bag of them, and nothing voiceless reopens it.
    feed(detector, (i) => noise(0.2, FRAME, i + 300), VAD_DEFAULTS.maxUnvoicedFrames);
    expect(detector.speaking).toBe(false);
    feed(detector, (i) => noise(0.2, FRAME, i + 600), 50);
    expect(detector.speaking).toBe(false);
  });
});

describe("the sensitivity dial", () => {
  it("moves every threshold the right way and keeps hysteresis at both ends", () => {
    const strict = sensitivityToThresholds(0);
    const loose = sensitivityToThresholds(100);
    expect(strict).toEqual({ snrOnDb: 14, snrOffDb: 11, hangoverFrames: 25, absoluteFloorDb: -45, voicingOn: 0.6 });
    expect(loose).toEqual({ snrOnDb: 3, snrOffDb: 1.5, hangoverFrames: 45, absoluteFloorDb: -68, voicingOn: 0 });
    let previous = sensitivityToThresholds(0);
    for (let value = 5; value <= 100; value += 5) {
      const next = sensitivityToThresholds(value);
      expect(next.snrOnDb).toBeLessThan(previous.snrOnDb);
      expect(next.snrOffDb).toBeLessThan(next.snrOnDb);
      // Never so low that room noise alone holds the gate open.
      expect(next.snrOffDb).toBeGreaterThanOrEqual(1.5);
      expect(next.absoluteFloorDb).toBeLessThan(previous.absoluteFloorDb);
      // Pitch is required at every ordinary setting and let go only near the top.
      expect(next.voicingOn).toBe(value <= 70 ? VAD_DEFAULTS.voicingOn : next.voicingOn);
      expect(next.voicingOn).toBeLessThanOrEqual(previous.voicingOn);
      previous = next;
    }
  });

  it("clamps anything out of range or not a number", () => {
    expect(sensitivityToThresholds(-50)).toEqual(sensitivityToThresholds(0));
    expect(sensitivityToThresholds(900)).toEqual(sensitivityToThresholds(100));
    expect(sensitivityToThresholds(Number.NaN)).toEqual(sensitivityToThresholds(50));
  });

  it("makes a quiet sound count only at the top, and a suppressed residue only there", () => {
    const strict = vad();
    feed(strict, (i) => noise(0.01, FRAME, i + 1), 200);
    strict.configure(sensitivityToThresholds(0));
    feed(strict, () => tone(0.02, FRAME), 6);
    expect(strict.speaking).toBe(false);

    const permissive = vad();
    feed(permissive, (i) => noise(0.01, FRAME, i + 1), 200);
    permissive.configure(sensitivityToThresholds(100));
    feed(permissive, () => tone(0.02, FRAME), 6);
    expect(permissive.speaking).toBe(true);

    // After a neural suppressor the floor sits far below -68 dBFS; a -50 dBFS
    // residue is a huge SNR event but under the strict absolute floor.
    const suppressedStrict = vad({ ...sensitivityToThresholds(0) });
    feed(suppressedStrict, () => noise(0.0002, FRAME), 200);
    feed(suppressedStrict, () => tone(0.004, FRAME), 12);
    expect(suppressedStrict.speaking).toBe(false);

    const suppressedLoose = vad({ ...sensitivityToThresholds(100) });
    feed(suppressedLoose, () => noise(0.0002, FRAME), 200);
    feed(suppressedLoose, () => tone(0.004, FRAME), 12);
    expect(suppressedLoose.speaking).toBe(true);
  });

  it("lets a whisper through at the top of the dial, where pitch is not required", () => {
    const whisper = vad({ ...sensitivityToThresholds(100) });
    feed(whisper, (i) => noise(0.0002, FRAME, i + 1), 200);
    feed(whisper, (i) => noise(0.05, FRAME, i + 3), 6);
    expect(whisper.speaking).toBe(true);
  });
});

describe("runtime reconfiguration", () => {
  it("changes thresholds without losing the learned floor or restarting the warm-up", () => {
    const detector = vad();
    feed(detector, (i) => noise(0.01, FRAME, i + 1), 200);
    const floor = detector.noiseFloorDb;
    detector.configure(sensitivityToThresholds(100));
    expect(detector.noiseFloorDb).toBe(floor);
    expect(detector.warmingUp).toBe(false);
  });

  it("validates every value it is sent instead of disabling detection", () => {
    const detector = vad();
    const before = { ...detector.config };
    detector.configure({ snrOnDb: Number.NaN, hangoverFrames: "lots", voicedHoldFrames: Number.NaN });
    expect(detector.config.snrOnDb).toBe(before.snrOnDb);
    expect(detector.config.hangoverFrames).toBe(before.hangoverFrames);
    expect(detector.config.voicedHoldFrames).toBe(before.voicedHoldFrames);

    detector.configure({ snrOnDb: 5, snrOffDb: 9, attackFrames: 0, hangoverFrames: -5, voicingOn: 5, maxUnvoicedFrames: 0.2 });
    // An inverted hysteresis is repaired, counts are at least one frame, voicing at most 1.
    expect(detector.config.snrOffDb).toBeLessThan(detector.config.snrOnDb);
    expect(detector.config.attackFrames).toBe(1);
    expect(detector.config.hangoverFrames).toBe(1);
    expect(detector.config.voicingOn).toBe(1);
    expect(detector.config.maxUnvoicedFrames).toBe(1);
  });

  it("does not leave a longer countdown running after a tightened hangover", () => {
    const detector = vad({ hangoverFrames: 60 });
    feed(detector, (i) => noise(0.01, FRAME, i + 1), 200);
    feed(detector, () => tone(0.4, FRAME), 10);
    expect(detector.speaking).toBe(true);
    detector.configure({ hangoverFrames: 3 });
    feed(detector, (i) => noise(0.01, FRAME, i + 900), 8);
    expect(detector.speaking).toBe(false);
  });
});
