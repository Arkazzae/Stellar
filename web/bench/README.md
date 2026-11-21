# Web client benchmarks and characterization harnesses

Real-browser performance benchmarks and audio characterization suites for the Stellarr web client (`web/`). All browser benchmarks drive a locally installed Chrome or Chromium binary directly over the Chrome DevTools Protocol (`web/bench/cdp.mjs`) without downloading extra browser packages or shipping benchmark code in the production bundle.

| Suite | Directory | Command / Guide |
| --- | --- | --- |
| Long-history transcript benchmark | `web/bench/transcript/` | `npm run bench:transcript -w stellarr-web` ([see below](#transcript-benchmark)) |
| Channel rail drag benchmark | `web/bench/rail/` | `npm run bench:rail-drag -w stellarr-web` ([see below](#channel-rail-drag-benchmark)) |
| Live-call channel rail update probe | `web/bench/rail/updates/` | `node web/bench/rail/updates/probe.mjs` ([see below](#channel-rail-live-update-probe)) |
| Voice capture, playback & suppressor records | `web/bench/voice/` | [`web/bench/voice/README.md`](voice/README.md) |

---

## Browser discovery

All CDP harnesses locate Chrome or Chromium via `STELLARR_CHROMIUM` (or `CHROME` / `CHROME_PATH`), falling back to standard installation paths:

- `/usr/bin/chromium`, `/usr/bin/chromium-browser`
- `/usr/bin/google-chrome`, `/usr/bin/google-chrome-stable`
- `/Applications/Google Chrome.app/Contents/MacOS/Google Chrome`
- `/Applications/Chromium.app/Contents/MacOS/Chromium`

Chrome is given 60 seconds (configurable via `STELLARR_CHROMIUM_START_SECONDS`) and up to three attempts with a fresh temporary profile to open its DevTools port.

---

## Transcript benchmark

Reproducible measurement and correctness verification of the long-history message transcript (`web/bench/transcript/`). It builds a minimal React page mounting the production `Transcript`, `MessageGroup`, incremental `createTranscriptPresenter`, and `globals.css` stylesheet.

### Running

```sh
# From the repository root (Node.js 22+)
npm run bench:transcript -w stellarr-web -- \
  --messages 1000,10000 --check --out bench/results/transcript.json
```

| Option | Description |
| --- | --- |
| `--messages 1000,10000` | Comma-separated message history sizes to measure (default `1000,10000`). |
| `--out <path>` | Output JSON path (default `web/bench/results/transcript.json`). |
| `--label <name>` | Label recorded in the JSON output. |
| `--check` / `--no-check` | Enable (default) or disable invariant assertions. |
| `--baseline <path>` | Compare mount, append, and prepend timings against a previous JSON result. |
| `--max-slowdown 2` | Maximum allowed slowdown multiplier relative to `--baseline` (default `2`). |
| `--timeout 240` | Per-size scenario timeout in seconds. |
| `--headed` | Run Chrome in headed mode for visual debugging. |
| `--keep-dist` | Keep the temporary Vite build under `web/.bench-dist`. |

### What is measured and verified

The fixture generates deterministic multi-day histories (~630 messages/day, mixed lengths, inline formatting, links, replies, reactions, pins, and data-URI images every 19 messages with intrinsic dimensions `1200×800`, `900×1600`, `640×640`, and `1600×600`) and runs a 13-step sequence at each history size:

1. Cold mount of `1,000` or `10,000` messages scrolled to the bottom row.
2. Append `200` messages while anchored at the bottom.
3. Prepend `200` older messages while reading in the middle of the history.
4. Expand a text row and a media row above the viewport.
5. Scroll through `80` animation frames (`180 px` per frame), recording frame intervals and Long Tasks.
6. Hover a visible message, verify hit-testing on the floating action toolbar, and trigger its reply callback.
7. Resize the viewport to `620×1020`, verify scroll anchor stability, and jump to `20%` and `90%` of the history.
8. Restore `1280×900` and jump to `12%`, `50%`, `88%`, and a reply target.
9. Scroll up `700 px` and expand an above-viewport row again.
10. Open the inline message editor, place the caret at offset `4`, append `30` new messages, and verify editor focus and selection survival.
11. Run native `window.find` for a phrase in an offscreen row at `70%` of the history.
12. Query the browser accessibility tree for that offscreen row.
13. Verify total DOM row, group, and message entry counts.

### Representative results (10,000 messages, macOS arm64, headless Chrome 144 at `1280×900`)

| Metric (10,000 messages) | Before optimization | After optimization |
| --- | ---: | ---: |
| Cold mount (producer + render) | 3,347 ms | 3,050 ms |
| Append 200 (producer / React commit / total) | 234 / 497 / 883 ms | 13 / 52 / 149 ms |
| Prepend 200 (producer / React commit / total) | 229 / 434 / 940 ms | 17 / 45 / 168 ms |
| Scroll frame interval (p50 / p95) | 54.2 / 91.4 ms | 29.4 / 48.6 ms |
| Scroll frames > 50 ms | 52 of 80 | 3 of 80 |
| DOM rows / groups / entries | 10,448 / 10,430 / 10,430 | 10,448 / 10,430 / 10,430 |

Key architectural decisions validated by this benchmark:

- **CSS layout and style containment**: `.transcript-inner > [data-row-id]` applies `contain: layout style` (omitting `paint` so the floating message toolbar can overhang its row). Hover, reaction, or edit mutations inside one row no longer invalidate layout across all 10,000 siblings.
- **Structural memoization**: `Transcript` renders a memoized `MessageRow` while `createTranscriptPresenter` reuses unchanged group and body references across appends and prepends, even when parent action callbacks change identity on every render.
- **Full DOM retention without virtualization or `content-visibility: auto`**: Keeping all loaded rows in the DOM preserves native find-in-page, full accessibility tree inspection, and exact scroll anchoring.

---

## Channel rail drag benchmark

Measures main-thread work, React commits, rendered component counts, and forced synchronous style/layout recalculations while dragging a channel row or a voice participant down a 150-channel, 10-section server rail (`web/bench/rail/run.mjs`).

```sh
npm run bench:rail-drag -w stellarr-web -- \
  --throttle 1,4 --drag rows,people --out bench/results/rail-drag.json
```

| Option | Description |
| --- | --- |
| `--throttle 1,4` | Comma-separated CDP CPU throttling multipliers (default `1,4`). |
| `--drag rows,people` | Drag scenarios to run: `rows` (reorder a channel across sections) and/or `people` (drag a voice occupant down the rail). |
| `--step 4` | Pointer movement step size in pixels per move event (default `4`). |
| `--hold 400` | Number of pointer moves held at the bottom edge to auto-scroll through all sections (default `400`). |
| `--out <file>` | Write detailed JSON metrics to `<file>`. |
| `--shot <file.png>` | Capture a PNG screenshot during the drag. |
| `--headed` | Run Chrome in headed mode. |
| `--keep-dist` | Keep the unminified Vite build in `web/.bench-dist/rail`. |

---

## Channel rail live-update probe

Measures React fiber commits, DOM class mutations, CSS animations, and voice activity detector transitions inside `RailContainer` across three live browser sessions (`Ala`, `Bob`, `Cyd`) connected to a running Stellarr instance during a voice call (`web/bench/rail/updates/`):

```sh
# 1. Build an unminified client bundle so React Fiber component names are preserved
node web/bench/rail/updates/build.mjs

# 2. Generate 48 kHz looped WAV microphone fixtures from local speech recordings
node web/bench/rail/updates/mics.mjs /path/to/speech-dir

# 3. Run the 3-browser live probe against a running test container
node web/bench/rail/updates/probe.mjs \
  --origin http://127.0.0.1:3110 \
  --mics /path/to/speech-dir \
  --container stellarr-railprobe-api-1 \
  --out rail-updates.json

# 4. Summarize commits, DOM class flips, and speaking-ring transitions per scenario window
node web/bench/rail/updates/analyze.mjs rail-updates.json
```
