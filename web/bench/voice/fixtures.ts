/**
 * Seeded test audio at 48 kHz, made here from arithmetic alone.
 *
 * Nothing is downloaded and nothing is recorded: every fixture is rebuilt
 * from its seed each time, identically in Node and in Chrome, and its digest
 * is kept in the capture record so that a change to this file shows up as a
 * changed fixture rather than as changed behaviour.
 *
 * Speech is a small formant synthesiser in the manner of Klatt's cascade: a
 * band-limited sawtooth for the vocal folds with a falling pitch line and
 * accents, five resonators whose first three glide between vowel targets, a
 * syllable rhythm with word gaps and phrase pauses, and consonants in front
 * of syllables (s, sz and f frication, plosive bursts with a closure, nasals).
 * It is not meant to sound like a person; it is meant to have what the
 * detector looks at: a pitch in the voice range, energy that comes and goes
 * at a syllable rate, unvoiced sounds before vowels and gaps between words.
 *
 * Every length is a multiple of 80 ms, so a fixture is a whole number of
 * render quanta and of 10 ms analysis frames.
 */
import {
  OnePole,
  Resonator,
  SAMPLE_RATE,
  Tilt,
  between,
  dbToGain,
  fadeIn,
  gauss,
  mixInto,
  mulberry32,
  peakAbs,
  setActiveLevel,
  setRmsLevel,
  type Rng,
} from "./dsp";

const MS = SAMPLE_RATE / 1000;

export interface Fixture {
  name: string;
  /** What the fixture is there to exercise, one or two sentences. */
  purpose: string;
  seconds: number;
  build(): Float32Array;
}

// ---- speech ----

interface Voice {
  f0Start: number;
  f0End: number;
  formantScale: number;
  /** Aspiration noise mixed into voiced sound, relative to the voicing. */
  breath: number;
}

const LOW_VOICE: Voice = { f0Start: 132, f0End: 96, formantScale: 1, breath: 0.03 };
const HIGH_VOICE: Voice = { f0Start: 228, f0End: 172, formantScale: 1.14, breath: 0.05 };

/** F1, F2, F3 of a, i, u, e, o, a schwa and an open e. */
const VOWELS: ReadonlyArray<readonly [number, number, number]> = [
  [730, 1090, 2440],
  [270, 2290, 3010],
  [300, 870, 2240],
  [530, 1840, 2480],
  [570, 840, 2410],
  [500, 1500, 2500],
  [660, 1720, 2410],
];

type Onset = "none" | "s" | "sz" | "f" | "plosive" | "nasal";

const ONSETS: ReadonlyArray<readonly [Onset, number]> = [
  ["none", 0.33],
  ["s", 0.14],
  ["sz", 0.08],
  ["f", 0.08],
  ["plosive", 0.22],
  ["nasal", 0.15],
];

function pickOnset(rng: Rng): Onset {
  let roll = rng();
  for (const [onset, weight] of ONSETS) {
    roll -= weight;
    if (roll < 0) return onset;
  }
  return "none";
}

/** Control tracks at one value per millisecond. */
class Tracks {
  readonly voiced: Float64Array;
  readonly whispered: Float64Array;
  readonly frication: Float64Array;
  readonly fricationHz: Float64Array;
  readonly fricationBw: Float64Array;
  readonly f1: Float64Array;
  readonly f2: Float64Array;
  readonly f3: Float64Array;
  readonly f0: Float64Array;

  constructor(readonly length: number) {
    this.voiced = new Float64Array(length);
    this.whispered = new Float64Array(length);
    this.frication = new Float64Array(length);
    this.fricationHz = new Float64Array(length).fill(5000);
    this.fricationBw = new Float64Array(length).fill(3000);
    this.f1 = new Float64Array(length).fill(500);
    this.f2 = new Float64Array(length).fill(1500);
    this.f3 = new Float64Array(length).fill(2500);
    this.f0 = new Float64Array(length).fill(120);
  }

  /** Raised-cosine attack and release around a hold, merged by maximum. */
  envelope(
    track: Float64Array,
    start: number,
    attack: number,
    hold: number,
    release: number,
    peak: number,
  ): void {
    const total = attack + hold + release;
    for (let i = 0; i < total; i += 1) {
      const at = Math.round(start) + i;
      if (at < 0 || at >= this.length) continue;
      let value = peak;
      if (i < attack) value = peak * 0.5 * (1 - Math.cos((Math.PI * i) / attack));
      else if (i >= attack + hold) {
        value = peak * 0.5 * (1 + Math.cos((Math.PI * (i - attack - hold)) / release));
      }
      track[at] = Math.max(track[at] ?? 0, value);
    }
  }

  /** Holds `value` over `[from, to)`, gliding in from the value before over `glide` ms. */
  target(track: Float64Array, from: number, to: number, value: number, glide: number): void {
    const start = Math.max(0, Math.round(from));
    const before = track[Math.max(0, start - 1)] ?? value;
    for (let at = start; at < Math.min(this.length, Math.round(to)); at += 1) {
      const progress = Math.min(1, (at - start) / glide);
      track[at] = before + (value - before) * progress;
    }
  }

  fill(track: Float64Array, from: number, to: number, value: number): void {
    for (let at = Math.max(0, Math.round(from)); at < Math.min(this.length, Math.round(to)); at += 1) {
      track[at] = value;
    }
  }
}

export interface Span {
  startMs: number;
  endMs: number;
}

export interface Phrase {
  /**
   * Quiet before the phrase, in ms: from the start of the fixture for the
   * first phrase, from the end of the one before for the others.
   */
  gap: number;
  words: number;
}

interface Spoken {
  tracks: Tracks;
  words: Span[];
  phrases: Span[];
}

/**
 * Lays out phrases of words of syllables on the control tracks.
 * `whisper` sends the excitation through noise instead of the vocal folds.
 */
function speak(rng: Rng, voice: Voice, phrases: Phrase[], lengthMs: number, whisper = false): Spoken {
  const tracks = new Tracks(lengthMs);
  const words: Span[] = [];
  const spans: Span[] = [];
  const amplitude = whisper ? tracks.whispered : tracks.voiced;
  const scale = voice.formantScale;

  let previousEnd = 0;
  for (const phrase of phrases) {
    const startAt = previousEnd + phrase.gap;
    let cursor = startAt;
    const pitchScale = between(rng, 0.95, 1.05);
    const syllables: Array<{ start: number; end: number; accent: boolean }> = [];
    for (let word = 0; word < phrase.words; word += 1) {
      const count = 1 + Math.floor(rng() * 3);
      const wordStart = cursor;
      for (let syllable = 0; syllable < count; syllable += 1) {
        const last = word === phrase.words - 1 && syllable === count - 1;
        const onset = pickOnset(rng);
        let vowelStart = cursor;
        if (onset === "s" || onset === "sz" || onset === "f") {
          const length = between(rng, 60, 100);
          const [hz, bw] = onset === "s" ? [6200, 2600] : onset === "sz" ? [3600, 1800] : [5000, 7000];
          tracks.envelope(tracks.frication, cursor, 15, length - 30, 15, onset === "f" ? 0.35 : 0.5);
          tracks.fill(tracks.fricationHz, cursor, cursor + length, hz);
          tracks.fill(tracks.fricationBw, cursor, cursor + length, bw);
          vowelStart = cursor + length - 10;
        } else if (onset === "plosive") {
          // Closure, then a burst and a short aspiration before the vowel.
          const burst = cursor + 30;
          tracks.envelope(tracks.frication, burst, 1, 4, 8, 1.1);
          tracks.fill(tracks.fricationHz, burst, burst + 13, 3200);
          tracks.fill(tracks.fricationBw, burst, burst + 13, 5000);
          tracks.envelope(tracks.frication, burst + 10, 5, 15, 10, 0.25);
          tracks.fill(tracks.fricationHz, burst + 13, burst + 40, 1900);
          tracks.fill(tracks.fricationBw, burst + 13, burst + 40, 2400);
          vowelStart = burst + 30;
        } else if (onset === "nasal") {
          const length = between(rng, 50, 80);
          tracks.envelope(amplitude, cursor, 10, length - 10, 10, 0.25);
          tracks.target(tracks.f1, cursor, cursor + length, 280 * scale, 10);
          tracks.target(tracks.f2, cursor, cursor + length, 1300 * scale, 10);
          tracks.target(tracks.f3, cursor, cursor + length, 2500 * scale, 10);
          vowelStart = cursor + length - 10;
        }
        const vowel = VOWELS[Math.floor(rng() * VOWELS.length)] ?? VOWELS[0]!;
        const length = between(rng, 110, 210) * (last ? 1.45 : 1);
        const accent = rng() < 0.4;
        const peak = (accent ? 1 : 0.62) * dbToGain(between(rng, -2, 2));
        tracks.envelope(amplitude, vowelStart, 22, length - 70, 48, peak);
        tracks.target(tracks.f1, vowelStart, vowelStart + length, vowel[0] * scale, 40);
        tracks.target(tracks.f2, vowelStart, vowelStart + length, vowel[1] * scale, 40);
        tracks.target(tracks.f3, vowelStart, vowelStart + length, vowel[2] * scale, 40);
        syllables.push({ start: vowelStart, end: vowelStart + length, accent });
        cursor = vowelStart + length - 10;
      }
      words.push({ startMs: wordStart, endMs: cursor + 10 });
      if (word < phrase.words - 1) cursor += between(rng, 40, 140);
    }
    spans.push({ startMs: startAt, endMs: cursor + 10 });
    previousEnd = cursor + 10;

    // A falling pitch line over the phrase, an accent hat on accented
    // syllables and a final fall.
    const from = startAt;
    const to = cursor + 10;
    for (let at = Math.max(0, Math.round(from)); at < Math.min(lengthMs, Math.round(to)); at += 1) {
      const progress = (at - from) / Math.max(1, to - from);
      tracks.f0[at] = pitchScale * (voice.f0Start + (voice.f0End - voice.f0Start) * progress);
    }
    for (const { start, end, accent } of syllables) {
      if (!accent) continue;
      for (let at = Math.round(start); at < Math.min(lengthMs, Math.round(end)); at += 1) {
        const shape = Math.sin((Math.PI * (at - start)) / (end - start));
        tracks.f0[at] = (tracks.f0[at] ?? 120) * (1 + 0.12 * shape);
      }
    }
    const final = syllables[syllables.length - 1];
    if (final) {
      for (let at = Math.round(final.start); at < Math.min(lengthMs, Math.round(final.end)); at += 1) {
        const progress = (at - final.start) / (final.end - final.start);
        tracks.f0[at] = (tracks.f0[at] ?? 120) * (1 - 0.1 * progress);
      }
    }
  }
  return { tracks, words, phrases: spans };
}

function polyBlep(phase: number, step: number): number {
  if (phase < step) {
    const t = phase / step;
    return t + t - t * t - 1;
  }
  if (phase > 1 - step) {
    const t = (phase - 1) / step;
    return t * t + t + t + 1;
  }
  return 0;
}

/** Turns control tracks into samples. */
function render(rng: Rng, voice: Voice, spoken: Spoken, samples: number): Float32Array {
  const { tracks } = spoken;
  const out = new Float32Array(samples);
  const scale = voice.formantScale;
  const formants = [
    new Resonator(500, 80),
    new Resonator(1500, 90),
    new Resonator(2500, 150),
    new Resonator(3300 * scale, 250),
    new Resonator(3750 * scale, 200),
  ];
  const fricative = new Resonator(5000, 3000);
  const fricativeTilt = new Tilt(0.9);
  const radiation = new Tilt(0.96);
  const aspirationTilt = new Tilt(0.7);
  let phase = 0;
  let jitter = 1;
  for (let i = 0; i < samples; i += 1) {
    const ms = Math.min(tracks.length - 1, Math.floor(i / MS));
    if (i % MS === 0) {
      const whisper = (tracks.whispered[ms] ?? 0) > 0 ? 1.6 : 1;
      formants[0]!.tune(tracks.f1[ms] ?? 500, 80 * whisper);
      formants[1]!.tune(tracks.f2[ms] ?? 1500, 90 * whisper);
      formants[2]!.tune(tracks.f3[ms] ?? 2500, 150 * whisper);
      fricative.tune(tracks.fricationHz[ms] ?? 5000, tracks.fricationBw[ms] ?? 3000);
    }
    const f0 = (tracks.f0[ms] ?? 120) * jitter;
    const step = f0 / SAMPLE_RATE;
    phase += step;
    if (phase >= 1) {
      phase -= 1;
      jitter = 1 + 0.006 * gauss(rng);
    }
    const saw = 2 * phase - 1 - polyBlep(phase, step);
    const voiced = tracks.voiced[ms] ?? 0;
    const whispered = tracks.whispered[ms] ?? 0;
    const noise = gauss(rng);
    let x = -saw * voiced + aspirationTilt.process(noise) * (voiced * voice.breath + whispered * 0.5);
    for (const formant of formants) x = formant.process(x);
    const frication = fricativeTilt.process(fricative.process(gauss(rng))) * (tracks.frication[ms] ?? 0);
    out[i] = radiation.process(x) + frication * 0.35;
  }
  return out;
}

function speech(seed: number, voice: Voice, phrases: Phrase[], seconds: number, levelDb: number, whisper = false) {
  const rng = mulberry32(seed);
  const lengthMs = Math.round(seconds * 1000);
  const spoken = speak(rng, voice, phrases, lengthMs, whisper);
  const samples = setActiveLevel(render(rng, voice, spoken, Math.round(seconds * SAMPLE_RATE)), levelDb);
  return { samples, words: spoken.words, phrases: spoken.phrases };
}

// ---- the rest of the room ----

/** Microphone self-noise and a quiet room: slightly darkened white noise. */
function roomTone(seed: number, samples: number, levelDb: number): Float32Array {
  const rng = mulberry32(seed);
  const out = new Float32Array(samples);
  const filter = new OnePole(6000);
  for (let i = 0; i < samples; i += 1) out[i] = filter.process(gauss(rng));
  return setRmsLevel(out, levelDb);
}

/** A desk fan: low-passed noise with a slow swell, steady over seconds. */
function fan(seed: number, samples: number, levelDb: number): Float32Array {
  const rng = mulberry32(seed);
  const out = new Float32Array(samples);
  const body = new OnePole(900);
  const rumble = new OnePole(220);
  for (let i = 0; i < samples; i += 1) {
    const noise = gauss(rng);
    const swell = 1 + 0.06 * Math.sin((2 * Math.PI * 0.45 * i) / SAMPLE_RATE);
    out[i] = (body.process(noise) + 1.5 * rumble.process(noise)) * swell;
  }
  return setRmsLevel(out, levelDb);
}

/** Mains hum from a ground loop: 50 Hz and its harmonics, 100 Hz strongest. */
const MAINS_HUM = { fundamental: 50, harmonics: [0.3, 1, 0.25, 0.45, 0.12, 0.18, 0.06, 0.08] };
/** A transformer's buzz: 100 Hz and its harmonics, nothing at 50 Hz. */
const BUZZ = { fundamental: 100, harmonics: [1, 0.5, 0.35, 0.2, 0.15, 0.1] };

function hum(kind: typeof MAINS_HUM, samples: number, levelDb: number): Float32Array {
  const out = new Float32Array(samples);
  const { fundamental, harmonics } = kind;
  for (let i = 0; i < samples; i += 1) {
    let value = 0;
    for (let k = 0; k < harmonics.length; k += 1) {
      value += (harmonics[k] ?? 0) * Math.sin((2 * Math.PI * fundamental * (k + 1) * i) / SAMPLE_RATE + k * 0.7);
    }
    out[i] = value;
  }
  return setRmsLevel(out, levelDb);
}

/** One key: a press and, 70-130 ms later, a softer release. */
function keystroke(rng: Rng, peakDb: number): Float32Array {
  const out = new Float32Array(Math.round(0.2 * SAMPLE_RATE));
  const click = (at: number, amplitude: number) => {
    const high = new Resonator(between(rng, 2400, 4800), between(rng, 90, 220));
    const mid = new Resonator(between(rng, 900, 1600), between(rng, 150, 300));
    const low = new Resonator(between(rng, 180, 360), 70);
    for (let i = 0; i < 1500 && at + i < out.length; i += 1) {
      const excitation = i < 24 ? gauss(rng) * (1 - i / 24) : 0;
      const value = high.process(excitation) * 3 + mid.process(excitation) * 1.5 + low.process(excitation) * 0.8;
      out[at + i] = (out[at + i] ?? 0) + value * amplitude;
    }
  };
  click(0, 1);
  click(Math.round(between(rng, 70, 130) * MS), between(rng, 0.45, 0.7));
  let peak = 0;
  for (const value of out) peak = Math.max(peak, Math.abs(value));
  const scale = peak > 0 ? dbToGain(peakDb) / peak : 0;
  for (let i = 0; i < out.length; i += 1) out[i] = (out[i] ?? 0) * scale;
  return out;
}

/** Typing from `fromMs` to `toMs`, 4 to 8 keys a second. */
function typing(seed: number, target: Float32Array, fromMs: number, toMs: number, peakDb: number): void {
  const rng = mulberry32(seed);
  let at = fromMs;
  while (at < toMs) {
    mixInto(target, keystroke(rng, peakDb + between(rng, -4, 0)), Math.round(at * MS));
    at += between(rng, 120, 260);
  }
}

/** One chew: a cloud of crackles and a wet low squelch, 180-320 ms. */
function chew(rng: Rng, levelDb: number): Float32Array {
  const length = Math.round(between(rng, 180, 320) * MS);
  const out = new Float32Array(length);
  const crackle = new Resonator(between(rng, 1600, 4200), 500);
  const body = new Resonator(between(rng, 350, 700), 250);
  const squelch = new OnePole(600);
  const rate = between(rng, 150, 400) / SAMPLE_RATE;
  for (let i = 0; i < length; i += 1) {
    const envelope = Math.sin((Math.PI * i) / length) ** 0.6;
    const impulse = rng() < rate ? gauss(rng) * (0.3 + 3 * rng() ** 3) : 0;
    const value =
      crackle.process(impulse) * 2.5 + body.process(impulse) * 1.2 + squelch.process(gauss(rng)) * 0.4;
    out[i] = value * envelope;
  }
  setRmsLevel(out, levelDb);
  // A crackle can have a crest factor no microphone delivers; keep the
  // loudest one at -3 dBFS.
  const peak = peakAbs(out);
  const ceiling = dbToGain(-3);
  if (peak > ceiling) for (let i = 0; i < length; i += 1) out[i] = (out[i] ?? 0) * (ceiling / peak);
  return out;
}

/** Chewing from `fromMs` to `toMs`; `gapless` packs the chews end to end. */
function chewing(seed: number, target: Float32Array, fromMs: number, toMs: number, levelDb: number, gapless = false): void {
  const rng = mulberry32(seed);
  let at = fromMs;
  while (at < toMs) {
    const one = chew(rng, levelDb + between(rng, -3, 2));
    mixInto(target, one, Math.round(at * MS));
    at += gapless ? (one.length / MS) * 0.8 : between(rng, 600, 800);
  }
}

/** An inhale or exhale: band-limited noise under a slow swell. */
function breath(seed: number, durationMs: number, centreHz: number, levelDb: number): Float32Array {
  const rng = mulberry32(seed);
  const length = Math.round(durationMs * MS);
  const out = new Float32Array(length);
  const band = new Resonator(centreHz, 1600);
  const tilt = new Tilt(0.8);
  for (let i = 0; i < length; i += 1) {
    const envelope = Math.sin((Math.PI * i) / length) ** 2;
    out[i] = band.process(tilt.process(gauss(rng))) * envelope;
  }
  return setActiveLevel(out, levelDb);
}

function blank(seconds: number): Float32Array {
  return new Float32Array(Math.round(seconds * SAMPLE_RATE));
}

function sum(...parts: Float32Array[]): Float32Array {
  const out = new Float32Array(parts[0]?.length ?? 0);
  for (const part of parts) mixInto(out, part);
  return out;
}

// ---- the catalogue ----

export const FIXTURES: readonly Fixture[] = [
  {
    name: "speech",
    purpose:
      "A talker at -26 dBFS in a quiet room, three phrases with a 300 ms and an 800 ms pause: onset on the first vowel, the lookahead keeping the consonant before it, the hangover across word gaps and across the short pause (but not at the strict end of the dial), the release in the long pause, and the leveller lifting the voice towards its target.",
    seconds: 6.72,
    build() {
      const seconds = this.seconds;
      const voice = speech(
        101,
        LOW_VOICE,
        [{ gap: 600, words: 3 }, { gap: 300, words: 3 }, { gap: 800, words: 3 }],
        seconds,
        -26,
      );
      return sum(voice.samples, roomTone(102, voice.samples.length, -70));
    },
  },
  {
    name: "talker-quiet",
    purpose:
      "A soft, high voice at -44 dBFS over a -76 dBFS room: the absolute floor that the dial moves (it shuts out this talker at the strict end), and the leveller climbing at 6 dB/s until it meets its +18 dB ceiling.",
    seconds: 6.4,
    build() {
      const seconds = this.seconds;
      const voice = speech(
        201,
        HIGH_VOICE,
        [{ gap: 500, words: 4 }, { gap: 500, words: 3 }, { gap: 350, words: 3 }],
        seconds,
        -44,
      );
      return sum(voice.samples, roomTone(202, voice.samples.length, -76));
    },
  },
  {
    name: "talker-loud-after-quiet",
    purpose:
      "A quiet phrase at -40 dBFS, long enough for the leveller to start lifting it, then a shout whose loudest sample is -1 dBFS (about -17 dBFS active): the leveller's peak guard, its 40 dB/s fall and its 6 dB cut limit, and whether the gain lands before the loud samples leave the gate's delay line.",
    seconds: 4.48,
    build() {
      const seconds = this.seconds;
      const quiet = speech(301, LOW_VOICE, [{ gap: 400, words: 5 }], seconds, -40);
      const loud = speech(302, LOW_VOICE, [{ gap: 2400, words: 3 }], seconds, -9);
      // A microphone stops at full scale; the shout stops at -1 dBFS.
      const ceiling = dbToGain(-1) / peakAbs(loud.samples);
      if (ceiling < 1) for (let i = 0; i < loud.samples.length; i += 1) loud.samples[i] = (loud.samples[i] ?? 0) * ceiling;
      return sum(quiet.samples, loud.samples, roomTone(303, quiet.samples.length, -70));
    },
  },
  {
    name: "whisper",
    purpose:
      "Whispered phrases at -40 dBFS: formants and consonants with no pitch. Only the top of the sensitivity dial, where voicing is no longer required, should let them count.",
    seconds: 3.84,
    build() {
      const seconds = this.seconds;
      const voice = speech(401, LOW_VOICE, [{ gap: 400, words: 2 }, { gap: 600, words: 2 }], seconds, -40, true);
      return sum(voice.samples, roomTone(402, voice.samples.length, -72));
    },
  },
  {
    name: "keyboard",
    purpose:
      "Typing at 4-8 keys a second, peaks near -10 dBFS, over a quiet room: the transient test (a sudden rise with a high crest factor) and the voicing test should both keep it out.",
    seconds: 2.56,
    build() {
      const out = roomTone(501, Math.round(this.seconds * SAMPLE_RATE), -70);
      typing(502, out, 300, 2400, -10);
      return out;
    },
  },
  {
    name: "speech-clicks",
    purpose:
      "Speech with key clicks in its pauses: one 30 ms after a word (inside the gap the hangover bridges), one 150 ms into the pause after a phrase (late enough that it must close the gate the hangover still holds), and typing through a long pause.",
    seconds: 5.12,
    build() {
      const seconds = this.seconds;
      const voice = speech(601, LOW_VOICE, [{ gap: 500, words: 4 }, { gap: 1100, words: 3 }], seconds, -26);
      const out = sum(voice.samples, roomTone(602, voice.samples.length, -70));
      const rng = mulberry32(603);
      const firstWord = voice.words[0];
      const firstPhrase = voice.phrases[0];
      if (firstWord) mixInto(out, keystroke(rng, -12), Math.round((firstWord.endMs + 30) * MS));
      if (firstPhrase) {
        mixInto(out, keystroke(rng, -12), Math.round((firstPhrase.endMs + 150) * MS));
        const secondPhrase = voice.phrases[1];
        typing(604, out, firstPhrase.endMs + 700, (secondPhrase?.startMs ?? 2700) - 250, -12);
      }
      return out;
    },
  },
  {
    name: "breath-chewing",
    purpose:
      "An inhale, chewing at speech level (-18 dBFS in the bursts) and an exhale, with no voice: loud, long and unvoiced, so only the voicing requirement keeps the gate shut, and the leveller must learn nothing from it.",
    seconds: 3.84,
    build() {
      const out = roomTone(701, Math.round(this.seconds * SAMPLE_RATE), -70);
      mixInto(out, breath(702, 650, 1300, -38), Math.round(250 * MS));
      chewing(703, out, 1050, 3000, -18);
      mixInto(out, breath(704, 700, 800, -40), Math.round(3080 * MS));
      return out;
    },
  },
  {
    name: "speech-crunching",
    purpose:
      "A phrase that runs straight into two seconds of continuous crunching at the same level: the hangover would hold the gate open for ever, and only the limit on unvoiced frames (600 ms) closes it.",
    seconds: 3.84,
    build() {
      const seconds = this.seconds;
      const voice = speech(801, LOW_VOICE, [{ gap: 400, words: 3 }], seconds, -24);
      const out = sum(voice.samples, roomTone(802, voice.samples.length, -70));
      const end = voice.phrases[0]?.endMs ?? 1500;
      chewing(803, out, end, 3800, -22, true);
      return out;
    },
  },
  {
    name: "buzz",
    purpose:
      "A transformer buzz (100 Hz and harmonics, -36 dBFS) that starts after 800 ms of quiet: a steady sound with a pitch, so it opens the detector at any sensitivity, the unvoiced limit cannot close it, and only the stationary-noise rule (90 steady frames, then the floor learns fast) lets it go.",
    seconds: 3.84,
    build() {
      const samples = Math.round(this.seconds * SAMPLE_RATE);
      const buzz = hum(BUZZ, samples, -36);
      fadeIn(buzz, Math.round(800 * MS), Math.round(60 * MS));
      return sum(buzz, roomTone(901, samples, -72));
    },
  },
  {
    name: "mains-hum",
    purpose:
      "Mains hum (50 Hz with a strong 100 Hz, -36 dBFS) that starts after 800 ms of quiet. Like the buzz it has a pitch, but its energy alternates between 10 ms frames, which the stationary-noise rule reads as modulation.",
    seconds: 3.84,
    build() {
      const samples = Math.round(this.seconds * SAMPLE_RATE);
      const buzz = hum(MAINS_HUM, samples, -36);
      fadeIn(buzz, Math.round(800 * MS), Math.round(60 * MS));
      return sum(buzz, roomTone(902, samples, -72));
    },
  },
  {
    name: "fan-speech",
    purpose:
      "A phrase in a quiet room, a fan (-40 dBFS) that switches on during it and stays, and a second phrase over the fan: the open detector meeting a new steady noise (closed by the unvoiced limit, or at the top of the dial by the stationary rule), the floor learning the fan, and speech about 14 dB over it.",
    seconds: 5.12,
    build() {
      const seconds = this.seconds;
      const voice = speech(1001, LOW_VOICE, [{ gap: 500, words: 4 }, { gap: 1300, words: 3 }], seconds, -26);
      const noise = fan(1002, voice.samples.length, -40);
      fadeIn(noise, Math.round(1500 * MS), Math.round(200 * MS));
      return sum(voice.samples, noise, roomTone(1003, voice.samples.length, -72));
    },
  },
  {
    name: "speech-over-fan",
    purpose:
      "A voice only 8 dB over a fan (-40 dBFS) that was running from the first sample, so the warm-up learns it: onsets sit on the on-threshold, and which frames count as speech depends on it at every point of the dial, the remote meter's default included.",
    seconds: 4.48,
    build() {
      const seconds = this.seconds;
      const voice = speech(1501, LOW_VOICE, [{ gap: 600, words: 3 }, { gap: 500, words: 2 }], seconds, -32);
      return sum(voice.samples, fan(1502, voice.samples.length, -40), roomTone(1503, voice.samples.length, -72));
    },
  },
  {
    name: "fan",
    purpose:
      "A fan (-40 dBFS) running from the first sample and nothing else: steady noise alone, which a noise suppressor should learn and turn down. The detector's capture record does not use it.",
    seconds: 4.48,
    build() {
      const samples = Math.round(this.seconds * SAMPLE_RATE);
      return sum(fan(1601, samples, -40), roomTone(1602, samples, -72));
    },
  },
  {
    name: "silence",
    purpose:
      "Digital silence, then microphone self-noise at -85 dBFS: zeros must teach the floor nothing, the meter must rest at zero, and the gain where there is nothing to measure is written as '-'.",
    seconds: 1.92,
    build() {
      const out = blank(this.seconds);
      mixInto(out, roomTone(1101, Math.round(0.96 * SAMPLE_RATE), -85), Math.round(0.96 * SAMPLE_RATE));
      return out;
    },
  },
  {
    name: "clipping",
    purpose:
      "A phrase driven 20 dB into hard clipping at full scale, then one whose samples run past full scale (to about 1.6) without clipping, as a +24 dB input gain can make them: the clamps in the analysis, the peak guard at a peak of 1, and what the gate passes of samples over 1.",
    seconds: 4.48,
    build() {
      const seconds = this.seconds;
      const first = speech(1201, LOW_VOICE, [{ gap: 300, words: 3 }], seconds, -26);
      const second = speech(1202, HIGH_VOICE, [{ gap: 2400, words: 2 }], seconds, -8);
      const out = sum(roomTone(1203, first.samples.length, -60));
      const drive = dbToGain(20);
      const overRange = 1.6 / Math.max(1e-9, peakAbs(second.samples));
      for (let i = 0; i < out.length; i += 1) {
        const driven = (first.samples[i] ?? 0) * drive;
        out[i] = (out[i] ?? 0) + Math.max(-1, Math.min(1, driven)) + (second.samples[i] ?? 0) * overRange;
      }
      return out;
    },
  },
  {
    name: "cold-start",
    purpose:
      "Speech from the very first sample, with no room to learn first: the 120 ms warm-up learns the voice as if it were the room, which is what happens when someone talks the moment the microphone opens.",
    seconds: 3.84,
    build() {
      const seconds = this.seconds;
      const voice = speech(1301, LOW_VOICE, [{ gap: 0, words: 3 }, { gap: 500, words: 2 }], seconds, -24);
      return sum(voice.samples, roomTone(1302, voice.samples.length, -70));
    },
  },
];

/**
 * Not a fixture of the record: the signal the delay of each configuration is
 * measured on. A vowel opens the gate, then white noise, which correlates
 * with itself at one lag only, passes while it is held.
 */
export const LATENCY_PROBE = {
  seconds: 1.28,
  /** The noise is at 0.9-1.2 s; this output window holds it for any delay up to 250 ms. */
  windowStart: Math.round(1.15 * SAMPLE_RATE),
  windowLength: Math.round(0.02 * SAMPLE_RATE),
  maxDelay: Math.round(0.25 * SAMPLE_RATE),
  build(): Float32Array {
    const samples = Math.round(this.seconds * SAMPLE_RATE);
    const out = roomTone(1401, samples, -70);
    const vowel = new Float32Array(samples);
    const vowelStart = Math.round(0.4 * SAMPLE_RATE);
    const vowelEnd = Math.round(0.9 * SAMPLE_RATE);
    let phase = 0;
    const formants = [new Resonator(700, 80), new Resonator(1200, 90), new Resonator(2500, 150)];
    for (let i = vowelStart; i < vowelEnd; i += 1) {
      phase = (phase + 120 / SAMPLE_RATE) % 1;
      let x = 1 - 2 * phase;
      for (const formant of formants) x = formant.process(x);
      vowel[i] = x;
    }
    setRmsLevel(vowel, -20, vowelStart, vowelEnd);
    const rng = mulberry32(1402);
    const noise = new Float32Array(samples);
    for (let i = Math.round(0.9 * SAMPLE_RATE); i < Math.round(1.2 * SAMPLE_RATE); i += 1) {
      noise[i] = gauss(rng) * dbToGain(-20);
    }
    return sum(out, vowel, noise);
  },
};

export function fixtureByName(name: string): Fixture {
  const fixture = FIXTURES.find((item) => item.name === name);
  if (!fixture) throw new Error(`no fixture called ${name}`);
  return fixture;
}

/** Builds a fixture and checks that it is a whole number of render quanta. */
export function buildFixture(fixture: Fixture): Float32Array {
  const samples = fixture.build();
  if (samples.length !== Math.round(fixture.seconds * SAMPLE_RATE) || samples.length % 3840 !== 0) {
    throw new Error(`${fixture.name} is ${samples.length} samples, not a multiple of 80 ms`);
  }
  return samples;
}

// ---- clean talkers, for measures that need the speech without the room ----

/**
 * Two talkers alone, at -26 dBFS, with digital silence between phrases: the
 * synthetic counterpart of a recording for the suppressor comparison, which
 * mixes them with the rooms below and measures what a suppressor leaves of
 * the speech against these. Not fixtures of any record.
 */
export const TALKERS: ReadonlyArray<{ name: string; build(): Float32Array }> = [
  {
    name: "synthetic-low",
    build: () =>
      speech(1701, LOW_VOICE, [{ gap: 300, words: 4 }, { gap: 700, words: 3 }, { gap: 1200, words: 4 }, { gap: 500, words: 2 }], 9.6, -26)
        .samples,
  },
  {
    name: "synthetic-high",
    build: () =>
      speech(1801, HIGH_VOICE, [{ gap: 300, words: 3 }, { gap: 900, words: 4 }, { gap: 600, words: 3 }, { gap: 1500, words: 3 }], 9.6, -26)
        .samples,
  },
];

// ---- material for mixing into recordings ----

/** The same rooms the fixtures use, offered to the recordings harness for its mixtures. */
export const ROOM = {
  fan: (samples: number, levelDb: number) => fan(9001, samples, levelDb),
  room: (samples: number, levelDb: number) => roomTone(9002, samples, levelDb),
  /** Typing and breaths, spread over the whole length. */
  clicksAndBreath(samples: number): Float32Array {
    const out = new Float32Array(samples);
    const totalMs = (samples / SAMPLE_RATE) * 1000;
    const rng = mulberry32(9003);
    for (let at = 400; at < totalMs - 300; at += between(rng, 900, 2200)) {
      if (rng() < 0.35) mixInto(out, breath(9004 + Math.round(at), 600, 1100, -40), Math.round(at * MS));
      else typing(9005 + Math.round(at), out, at, at + between(rng, 150, 600), -12);
    }
    return out;
  },
};
