# Voice processing record

A record of what the web client's audio processing does today, made before it
is rewritten, so that the rewrite can be held to the same sound:

- **capture**: the speech detector, the input gate and the leveller in
  `web/public/audio/vad-processor.js`, as the app builds them
  (`capture/captureGraph.ts`, `capture/gateNode.ts`, `capture/vad.ts`);
- **playback**: each remote participant's strip of Web Audio nodes
  (`playback/remoteMixer.ts`, `playback/remoteStrip.ts`,
  `playback/playbackProfile.ts`), for every playback profile.

How the microphone is opened and what the browser does to it
(`micAcquire.ts`, `browserProcessing.ts`, `micPipeline.ts`, the microphone
constraints) is left out on purpose: it is not being preserved. So are the
noise suppressors and the fixed input gain.

## Checks

| What | Command | Time |
| --- | --- | --- |
| Capture record | `npm run test:web` (three files in `src/stellarr/lib/voice/characterization/`) | about 5 s of CPU, split over three files that run side by side |
| Playback record | `npm run bench:voice-playback -w stellarr-web -- --check` | about 10 s, most of it starting Chrome |

The playback check needs Chrome, found as the transcript benchmark finds it
(`STELLARR_CHROMIUM`, `CHROME`, or a usual install path). In CI it is one more
step in the web job:

```sh
STELLARR_CHROMIUM="$chrome" npm run bench:voice-playback -w stellarr-web -- \
  --check --out "$RUNNER_TEMP/voice-playback.json"
```

## Re-recording

```sh
npm run voice:record -w stellarr-web
```

This rewrites `records/capture/*.txt` through Vitest and
`records/playback.txt` from five separate Chrome runs. **It is legitimate only
in a commit whose message names the audible change it intends**, such as
"the gate releases 20 ms later" or "the studio profile loses 1 dB at 3 kHz".
A rewrite that is meant to sound the same must pass the checks without it. Read
the diff of `records/` before committing: every line that moved is a change
someone will hear or see.

Two other things also move the records, and neither is a change in sound:

- an edit to `fixtures.ts` changes the test audio. The fixture's own test
  ("is the audio its record was made from") fails first and names the digest;
  re-record in the same commit and say that the fixtures changed;
- a new Chrome may render the playback strips differently. The check prints
  the Chrome the record was made with next to the one it ran in.

## The fixtures

Made by `fixtures.ts` from a seed, at 48 kHz, nothing downloaded. Speech is a
small formant synthesiser (a band-limited sawtooth with a falling pitch line
and accents, five resonators gliding between vowels, consonants in front of
syllables: s, sz and f frication, plosive bursts after a closure, nasals,
with word gaps and phrase pauses). Every length is a multiple of 80 ms, a whole
number of render quanta and of 10 ms analysis frames.

| Fixture | Length | What it exercises |
| --- | --- | --- |
| `speech` | 6.72 s | A talker at -26 dBFS, pauses of 300 and 800 ms: onset, lookahead, hangover across words and the short pause (but not at the strict end of the dial), release in the long pause, the leveller lifting the voice. |
| `talker-quiet` | 6.40 s | A soft high voice at -44 dBFS: the absolute floor the dial moves, the leveller climbing at 6 dB/s to its +18 dB ceiling. |
| `talker-loud-after-quiet` | 4.48 s | A quiet phrase, then a shout at -9 dBFS peaking near full scale: the peak guard, the 40 dB/s fall, the 6 dB cut limit, and whether the cut lands while the loud samples are still in the gate's delay line. |
| `clipping` | 4.48 s | Speech driven 20 dB into hard clipping, then samples past full scale (to 1.6): the clamps in the analysis and what the gate passes of samples over 1. |
| `cold-start` | 3.84 s | Speech from the first sample: the 120 ms warm-up learns the voice as if it were the room. |
| `silence` | 1.92 s | Digital zero, then -85 dBFS self-noise: zeros teach the floor nothing; the meter rests at zero. |
| `whisper` | 3.84 s | Whispered phrases at -40 dBFS, no pitch: only the top of the dial lets them count. |
| `keyboard` | 2.56 s | Typing at 4-8 keys a second, peaks near -10 dBFS: the transient and voicing tests. |
| `speech-clicks` | 5.12 s | Clicks 30 ms after a word, 150 ms into a pause (must close the held gate), and typing through a long pause. |
| `breath-chewing` | 3.84 s | An inhale, chewing at speech level and an exhale, no voice: only the voicing requirement keeps the gate shut; the leveller learns nothing. |
| `speech-crunching` | 3.84 s | A phrase running straight into two seconds of crunching: only the unvoiced limit (600 ms) closes the gate. |
| `buzz` | 3.84 s | A 100 Hz transformer buzz after 800 ms of quiet: pitched and steady, so only the stationary-noise rule closes the gate. |
| `mains-hum` | 3.84 s | 50 Hz mains hum with a strong 100 Hz after 800 ms of quiet: pitched, but its energy alternates between 10 ms frames. |
| `fan-speech` | 5.12 s | A fan switching on during a phrase and staying, a second phrase over it: the unvoiced limit, the stationary rule at the top of the dial, speech 14 dB over a fan. |

The playback record uses its own signals, generated in the page: a 31-tone
multitone (third octaves from 20 Hz to 20 kHz), 1 kHz tones at fixed levels
and a 1 kHz tone stepping from -40 to -6 dBFS and back. It also plays the app's
own `public/sounds/voice-playback-test.mp3`, decoded by Chrome, which the
detector treats as speech (it opens eight times and the leveller learns from
it).

## The capture record

`capture/worklet.ts` evaluates a copy of `vad-processor.js` in a scope that
has `AudioWorkletProcessor`, `registerProcessor` and `sampleRate`, keeps the
class it registers, and drives it as the browser would: render quanta of 128
frames, one mono input, one output, messages from the main thread delivered
between quanta, `postMessage` calls collected with the quantum they were made
in. `capture/app.ts` takes each node's options from the app's own code:
`buildCaptureGraph` for the microphone (gate, leveller or both, as the
settings choose), `SpeechDetector` for the local meter and for a remote
participant, `buildGateNode` at `REMOTE_GATE_SENSITIVITY` for a gated remote
strip. Messages the app posts later (`config` when the dial moves, `stop` on
teardown) are delivered as the app sends them.

Configurations, for every fixture: the meter, the gate, the leveller and both,
at sensitivity 0, 60 (the default) and 100; the meter at 85 (the one place the
voicing requirement is partial); a remote participant's meter; the receiver's
gate at 40. On `speech`, the meter and gate+leveller also run with the dial
moved to 100 at 1.4 s and 0 at 3.0 s, and the call ended at 5.6 s.

Each run is written as (all measured from outside the processor):

| Line | What |
| --- | --- |
| `posts` | every message posted to the main thread, at the end of its render quantum in ms; `+` speaking, `-` not, then the meter level in eighths |
| `speech` | the stretches between a `+` and the next `-` |
| `gain` | per 10 ms of output, RMS(out) / RMS(input delayed by the configuration's latency) in dB; `x` closed (below -60 dB), `-` nothing to measure against, `!` output with no input; `v*n` is n blocks of v, `a>b*n` a straight ramp |
| `peak` | the largest output sample in dBFS, and how many went past 0.95 and past 1 |
| `out none` | the configuration writes no output (the meter) |
| `stopped` | when `process()` first returned false |

`records/capture/index.txt` holds each configuration's latency, measured by
cross-correlation on a probe (white noise while the gate is held open). The
check compares posts, speech and stops exactly, gains within 0.1 dB (the
encoding rounds to at most 0.06 dB), peaks within 0.05 dB and the counts past
0.95 and 1 within 2 samples or 1%. Node runs the same JavaScript on any
machine, so the capture record has no measured spread; the playback check
also renders `speech` through Chrome's own AudioWorklet and requires the
same gains and the same messages in the same order, which is what shows that
this driver runs the processor as the browser does.

## The playback record

`playback/render.ts` builds each strip with `RemoteVoiceMixer` itself
(`attachElement`, which mounts a `RemoteStrip` and applies the profile and the
volume immediately, and `setGate`) on an `OfflineAudioContext` at 48 kHz. An
offline context has no `createMediaStreamDestination` and no media element, so
for the length of the build the context object gets two methods: the
destination becomes a gain node feeding the rendered output, and the element
source becomes a buffer source with the test signal. `window.AudioContext`
returns that context while the mixer is made. Nothing else is replaced.

Strips: every profile at 100%; custom tunings flat, bright and extreme at
100%; studio at 50, 150 and 200%; off at 50 and 200%. For each:

| Line | What |
| --- | --- |
| `latency` | samples from input to output, from cross-correlation on -50 dBFS noise |
| `response-low` | gain at 31 third-octave frequencies, multitone at -60 dBFS RMS (under every threshold) |
| `response-hot` | the same at -20 dBFS RMS, where the compressor and limiter work |
| `curve` | output against input level for a 1 kHz tone, dBFS of the tone's peak, steady state |
| `attack`, `release` | output envelope per ms around a -40 to -6 dBFS step and back, aligned by the latency |
| `settle`, `overshoot` | ms to settle within 1 dB after each step; dB the attack overshoots |

Then: the studio strip with the receiver's gate over `speech`; the app's
sample through each profile (`level`: RMS, peak and the 50th, 90th and 99th
percentile of 50 ms blocks); the sample through the capture node in Chrome.

Chrome's compressor is its own implementation, so the playback record is held
to tolerances, written in its header. Each is four times the largest spread
seen across the separate Chrome runs the record was made from, and never less
than a floor: 0.05 dB for the responses, 0.1 dB for the curve and levels,
0.25 dB for the envelopes, 2 ms for settling times, 0 samples for latency. On
the machine that made the record, five runs agreed to the last digit, so the
floors are what is in force. They sit below the smallest deliberate change
the check was tried against (see the commit that added this harness). The
spread between machines, or between Chrome builds, was not measured.

## Real recordings

Put WAV files in `recordings/`. That folder is ignored by git apart from its
`.gitignore`, and so is everything the harness writes there: a voice in a
public repository is not something its owner can take back. Any WAV at 48 kHz
works (16, 24 or 32-bit integer or float, several channels are averaged); to
convert:

```sh
afconvert -f WAVE -d LEI16@48000 -c 1 in.m4a recordings/me.wav   # macOS
ffmpeg -i in.m4a -ac 1 -ar 48000 recordings/me.wav
```

When files are there, both checks run over them too:

- `npm run test:web` runs each one, and four versions made from it (24 dB
  quieter over a quiet room, 10 dB hotter into clipping, over a fan, with
  typing and breaths), through the default configurations. The first run
  writes `<name>.capture.txt` beside it and reports the test as skipped, not
  passed; later runs check against it. `<name>.summary.txt` says what the
  detector made of it: how much of the speech (taken from the recording's own
  envelope) it caught, how much the gate passed, and what happened to the
  first 100 ms after each pause. Delete a `.capture.txt` to record it again.
- the playback check plays each through every profile and keeps
  `<name>.playback.txt` beside it the same way.

A minute of ordinary talking with a few pauses, in the room and on the
microphone you use for calls, is what the record most lacks.

`node bench/voice/make-tts.mjs` makes five synthetic voices with macOS's `say`
and `afconvert` (Polish and English, a fast talker, short words, a whisper)
into the same folder. The system voices' licence does not allow their output
to be redistributed: neither the files nor anything made from their samples
may be committed.

## Where the harness touches the implementation

A rewrite changes these files of the harness, not the records:

- `capture/worklet.ts`: that `vad-processor.js` registers one processor
  class, reads `sampleRate` from its global scope, takes `processorOptions`,
  and talks through `this.port`;
- `capture/app.ts`: the functions the app builds its nodes with
  (`buildCaptureGraph`, `SpeechDetector.add`, `buildGateNode`) and
  `REMOTE_GATE_SENSITIVITY`;
- `playback/render.ts`: `RemoteVoiceMixer` with `attachElement` and
  `setGate`, and the two context methods it is given.

Nothing reads a field of a processor or a node. The detector's frame-level
reasoning (the noise floor it has learned, the SNR, the voicing of a frame,
the gate's and the leveller's internal gains) is therefore not in the record:
only what reaches the output and the main thread is.
