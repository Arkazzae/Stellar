# Voice processing characterization and benchmark suite

Characterization records and real-browser verification harnesses for the web client's audio pipeline (`web/bench/voice/`):

- **`capture`**: The speech detector, input gate, and automatic leveler inside the detector worklet (`web/src/stellarr/lib/voice/capture/detector/`, compiled to `/audio/detector-worklet.js`), driven with the exact node options used in production (`capture/captureGraph.ts`, `capture/gateNode.ts`, `capture/vad.ts`).
- **`playback`**: Per-participant remote Web Audio processing strips (`playback/remoteMixer.ts`, `playback/remoteStrip.ts`, `playback/playbackProfile.ts`) across all playback profiles and custom EQ/compressor tunings.
- **`suppressor`**: Source-built WebAssembly noise suppressors—Speex and RNNoise (`capture/speex/` and `capture/rnnoise/`, compiled to `/audio/speex-worklet.js` and `/audio/rnnoise-worklet.js` with WASM binaries from `artifacts/recipes/speex/` and `artifacts/recipes/rnnoise/`).

Microphone device acquisition and browser-native constraints (`micAcquire.ts`, `browserProcessing.ts`, `micPipeline.ts`), fixed input gain, and the prebuilt GTCRN artifact are excluded from the characterization records.

## Running checks

| Check | Command | Typical duration |
| --- | --- | --- |
| Capture record | `npm run test:web` (`src/stellarr/lib/voice/characterization/`) | ~1.2 s wall / ~3.7 s CPU on a 10-core arm64 Mac |
| Playback record | `npm run bench:voice-playback -w stellarr-web -- --check` | ~5 s (~2 s offline rendering in headless Chrome) |
| Suppressor record | `npm run test:web` (`src/stellarr/lib/voice/characterization/suppressorRecord.test.ts`) | ~2 s |

If local WAV recordings are present under `web/bench/voice/recordings/`, `npm run test:web` and `bench:voice-playback` automatically evaluate them as well (~3.4 s for 97 s of 48 kHz audio).

In CI, the `web` job runs the playback check against headless Chrome:

```sh
STELLARR_CHROMIUM="$chrome" npm run bench:voice-playback -w stellarr-web -- \
  --check --out "$RUNNER_TEMP/voice-playback.json" --summary "$RUNNER_TEMP/voice-playback.md"
```

The command reports the Chrome version recorded in `records/playback.txt` alongside the active browser version, the maximum deviation and tolerance for every metric category, and writes a Markdown summary when `--summary` is provided. Passing `--measured <file>` writes the current run's raw measurements in the exact `records/playback.txt` format for line-by-line diffing.

## Re-recording golden files

```sh
npm run voice:record -w stellarr-web
```

This regenerates `records/capture/*.txt` and `records/suppressor/*.txt` via Vitest and `records/playback.txt` across five headless Chrome runs.

> [!IMPORTANT]
> Only re-record golden files in a commit that intentionally changes audible behavior (and state the exact acoustic change in the commit message), when `fixtures.ts` itself is modified, or when a major Chrome update shifts `OfflineAudioContext` compressor output beyond existing platform baselines. Refactorings meant to preserve sound must pass existing records without re-recording.

## Synthetic 48 kHz fixtures (`fixtures.ts`)

`fixtures.ts` deterministically synthesizes 16 mono 48 kHz test signals from a fixed seed (using a 5-resonator formant synthesizer with pitched sawtooth excitation, sibilant/fricative noise bursts, plosive closures, nasals, word gaps, and room/fan noise models). Every fixture length is an exact multiple of `80 ms` (both `128`-sample render quanta and `10 ms` analysis frames).

| Fixture | Duration | Exercised behavior |
| --- | --- | --- |
| `speech` | 6.72 s | Talker at `-26 dBFS` with `300 ms` and `800 ms` pauses: onset detection, lookahead, hangover across words and short pauses, gate release during long pauses, and leveler gain riding. |
| `talker-quiet` | 6.40 s | Soft high-pitched voice at `-44 dBFS`: sensitivity dial absolute floor and leveler climbing at `6 dB/s` up to its `+18 dB` boost cap. |
| `talker-loud-after-quiet` | 4.48 s | Quiet phrase followed by a `-1 dBFS` shout: peak guard, `40 dB/s` attenuation rate, `6 dB` cut limit, and lookahead delay-line protection. |
| `clipping` | 4.48 s | Speech driven `20 dB` into hard clipping plus out-of-range samples up to `1.6`: analysis clamping and sample handling above `1.0`. |
| `cold-start` | 3.84 s | Speech starting at sample `0`: `120 ms` noise-floor warm-up behavior when speech is present immediately. |
| `silence` | 1.92 s | Digital zero followed by `-85 dBFS` thermal noise: floor protection against digital silence and idle meter resting at `0`. |
| `whisper` | 3.84 s | Unvoiced whispered phrases at `-40 dBFS`: accepted only at high sensitivity settings where voicing thresholds relax. |
| `keyboard` | 2.56 s | Keystrokes at `4–8 keys/s` peaking near `-10 dBFS`: transient and voicing rejection. |
| `speech-clicks` | 5.12 s | Clicks `30 ms` after a word, `150 ms` into a pause (forcing held gate closure), and continuous typing across a long pause. |
| `breath-chewing` | 3.84 s | Inhale, speech-level chewing, and exhale without voicing: gate remains closed and leveler freezes its gain estimate. |
| `speech-crunching` | 3.84 s | Voiced phrase running directly into 2 seconds of unvoiced crunching: `600 ms` unvoiced timeout closes the gate. |
| `buzz` | 3.84 s | `100 Hz` transformer buzz after `800 ms` of quiet: pitched but stationary, closed by the stationary-noise rule. |
| `mains-hum` | 3.84 s | `50 Hz` mains hum with `100 Hz` harmonic after `800 ms` of quiet: pitched energy alternating across `10 ms` frames. |
| `fan-speech` | 5.12 s | Fan noise switching on mid-phrase and continuing under a second phrase (`+14 dB` SNR): unvoiced limit and stationary noise adaptation. |
| `speech-over-fan` | 4.48 s | Voice at `+8 dB` SNR over continuous fan noise from sample `0`: exercises onset SNR thresholds across all sensitivity dial positions, including the remote meter default. |
| `fan` | 4.48 s | Continuous fan noise alone from sample `0`: stationary noise suppression baseline for suppressor tests (unused by `capture`). |

The playback harness additionally generates a 31-band third-octave multitone (`20 Hz`–`20 kHz`), steady `1 kHz` tones across levels, a `-40` to `-6 dBFS` step tone, and decodes `public/sounds/voice-playback-test.mp3` in Chrome.

## Capture record (`capture/`)

`capture/worklet.ts` executes `capture/detector/processor.ts` inside a deterministic `AudioWorkletProcessor` harness in `128`-sample render quanta, delivering main-thread control messages between quanta and recording every `postMessage` event alongside its quantum index. `capture/app.ts` constructs configurations using production helpers (`buildCaptureGraph`, `SpeechDetector`, `buildGateNode` at `REMOTE_GATE_SENSITIVITY`).

Each fixture is tested across meter-only, gate-only, leveler-only, and gate+leveler modes at sensitivities `0`, `60` (default), and `100`, plus meter at `85`, remote participant meter, and receiver gate at `40`. On `speech`, dynamic mid-call sensitivity changes (`100` at `1.4 s`, `0` at `3.0 s`) and teardown (`stop` at `5.6 s`) are also verified.

Recorded fields per run:

| Field | Description |
| --- | --- |
| `posts` | Main-thread messages emitted at the end of a quantum (timestamp in `ms`, `+` speaking / `-` silent, and meter level in eighths) |
| `speech` | Time intervals between `+` and `-` speaking transitions |
| `gain` | Per-10 ms output-to-delayed-input RMS ratio in dB (`x` = closed below `-60 dB`, `-` = silent reference, `!` = output without input, `v*n` = run-length encoding, `a>b*n` = linear ramp) |
| `peak` | Maximum output sample in dBFS plus sample counts exceeding `0.95` and `1.0` |
| `out none` | Indicates a meter-only configuration that writes no audio output |
| `stopped` | Timestamp when `process()` first returned `false` after teardown |

`records/capture/index.txt` records each configuration's sample latency (measured via white-noise cross-correlation while the gate is open). Assertions require exact matches on `posts`, `speech`, and `stopped`, `0.1 dB` tolerance on `gain`, `0.05 dB` on `peak`, and `2 samples` (or `1%`) on threshold counts. The playback check also cross-checks `speech` inside Chrome's native `AudioWorklet` to verify that the Node driver matches browser execution.

## Playback record (`playback/`)

`playback/render.ts` instantiates `RemoteVoiceMixer` (`attachElement` and `setGate`) against a `48 kHz` `OfflineAudioContext` in headless Chrome across every playback profile (`100%`), custom EQ tunings (`flat`, `bright`, `extreme`), `studio` at `50%`, `150%`, and `200%`, and `off` at `50%` and `200%`.

Recorded metrics per strip:

| Field | Description |
| --- | --- |
| `latency` | Input-to-output delay in samples via `-50 dBFS` noise cross-correlation |
| `response-low` | Gain across 31 third-octave bands at `-60 dBFS` RMS (linear region below compressor thresholds) |
| `response-hot` | Gain across 31 third-octave bands at `-20 dBFS` RMS (active compression and limiting) |
| `curve` | Steady-state `1 kHz` input-to-output transfer curve in dBFS |
| `attack`, `release` | 1 ms output envelope around a `-40 → -6 → -40 dBFS` tone step, latency-aligned |
| `settle`, `overshoot` | Settling time within `1 dB` after each step (`ms`) and attack overshoot (`dB`) |

Cross-platform comparison floors (`playback/compare.ts`) verified across macOS `arm64`, Linux `x64`, and Linux `arm64`:

| Metric | Max observed macOS vs. Linux diff | Enforced floor |
| --- | ---: | ---: |
| Latency | `0 samples` | `0 samples` |
| Frequency response | `0.01 dB` | `0.05 dB` |
| Transfer curve | `0.01 dB` | `0.1 dB` |
| Envelope (per 1 ms block) | `0.26 dB` | `0.6 dB` |
| Post-step envelope area | `4.2 dB·ms` | `10 dB·ms` |
| Settling time | `2 ms` | `5 ms` |
| Attack overshoot & sample levels | `0 dB` | `0.1 dB` |
| Gated strip & capture node gains | `0 dB` | `0.25 dB` |

## Suppressor record (`suppressor/`)

`suppressor/record.ts` drives the assembled production WebAssembly binaries (`web/public/audio/speex.wasm` and `web/public/audio/rnnoise.wasm`) through their TypeScript wrappers (`capture/speex/preprocessor.ts` and `capture/rnnoise/denoiser.ts`) one `128`-sample render quantum at a time across `speech`, `speech-over-fan`, `fan-speech`, `fan`, `keyboard`, `buzz`, `silence`, and `clipping`.

Because WebAssembly arithmetic is deterministic across architectures, `records/suppressor/speex.txt` (int16 input/output digests, per-10 ms gains, peak, and preprocessor `ctl` state) and `records/suppressor/rnnoise.txt` (float32 output digest and `1,408`-sample latency) are verified for bit-exact output equality.

## Comparing suppressor builds (`bench:voice-suppressor`)

```sh
npm run bench:voice-suppressor -w stellarr-web -- --engine speex --before <git-revision>
npm run bench:voice-suppressor -w stellarr-web -- --engine rnnoise \
  --before-worklet old/rnnoise-worklet.js --before-wasm old/rnnoise_simd.wasm \
  --quality --mixtures en-daniel-speech.fan10,en-whisper.quiet --write-outputs <dir>
```

`suppressor/run.mjs` renders fixtures through the current engine (`attachSuppressor` with `/audio/*`) and an earlier revision or explicit worklet/WASM pair in Chrome's `OfflineAudioContext`, latency-aligns both outputs, and reports sample diffs, peak/RMS deltas, and first active sample index:

- `--fixtures <list>` selects specific fixtures; `--require-identical` exits non-zero on any sample difference.
- `--write-outputs <dir>` saves input and both outputs as 32-bit float WAV files for listening comparison.
- `--quality` mixes clean speech recordings with five room/noise profiles at `-26 dBFS` (`suppressor/quality.ts`) and measures noise attenuation during pauses, SI-SDR, speech level, high-band energy (`>4 kHz` and `>8 kHz`), initial `50 ms` phrase onset preservation, and correlation latency.

In addition to offline rendering, `suppressor/run.mjs` verifies three real-time lifecycle invariants in Chrome (started with `--disable-audio-output`):

1. Audio streams through `attachSuppressor` in a real-time `AudioContext` without render underruns.
2. Sending the `"destroy"` port message when tearing down a suppressor immediately silences the worklet node.
3. A `404` response for the WASM binary causes `attachSuppressor` to resolve to `null`.

## Local voice recordings (`recordings/`)

Place 48 kHz WAV files in `web/bench/voice/recordings/` (gitignored so personal voice recordings are never committed). Convert recordings with:

```sh
afconvert -f WAVE -d LEI16@48000 -c 1 in.m4a recordings/me.wav   # macOS
ffmpeg -i in.m4a -ac 1 -ar 48000 recordings/me.wav               # ffmpeg
```

When WAV files are present in `recordings/`, `npm run test:web` and `bench:voice-playback` automatically generate and verify local baseline `.capture.txt`, `.summary.txt`, and `.playback.txt` files alongside them. On macOS, `node bench/voice/make-tts.mjs` can also synthesize five local 48 kHz TTS voices into `recordings/` (do not commit TTS outputs).

## End-to-end two-browser call verification (`call/`)

`web/bench/voice/call/` verifies that two headless Chrome instances connected to a live Stellarr container exchange audio and trigger remote speaking indicators accurately over WebRTC:

```sh
npm run build:web
web/bench/voice/call/start-container.sh <image> web/dist     # prints https://localhost:18743
node web/bench/voice/call/call.mjs --origin https://localhost:18743 --out result.json
docker rm -f stellarr-voice-call && docker volume rm stellarr-voice-call-data
```

## Harness integration boundaries

The characterization suite touches only public module boundaries—never private fields of processors or nodes:

- `capture/worklet.ts`: `capture/detector/processor.ts` registration, `sampleRate`, `processorOptions`, and `this.port`.
- `playback/run.mjs`: `workerBuild` from `web/worklets.ts` (testing the minified production worklet bundle).
- `capture/app.ts`: `buildCaptureGraph`, `SpeechDetector.add`, `buildGateNode`, and `REMOTE_GATE_SENSITIVITY`.
- `playback/render.ts`: `RemoteVoiceMixer` (`attachElement` and `setGate`).
- `suppressor/record.ts`: `SpeexPreprocessor` / `instantiateSpeex` (`capture/speex/preprocessor.ts`) and `RnnoiseDenoiser` / `instantiateRnnoise` (`capture/rnnoise/denoiser.ts`).
- `suppressor/main.ts`: `attachSuppressor` and `resetSuppressorCacheForTests` (`capture/noiseSuppression.ts`).
