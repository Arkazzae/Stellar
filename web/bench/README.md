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
| `--baseline <path>` | Compare mount, append, and prepend timings against a previous JSON result, require the same keyboard stops and toolbar placement, and print the pointer sweep and wheel figures beside it. |
| `--max-slowdown 2` | Maximum allowed slowdown multiplier relative to `--baseline` (default `2`). |
| `--timeout 600` | Per-size scenario timeout in seconds. The sweep and wheel at 6× throttling take most of a 10,000-message run. |
| `--headed` | Run Chrome in headed mode for visual debugging. |
| `--keep-dist` | Keep the temporary Vite build under `web/.bench-dist`. |

### What is measured and verified

The fixture generates deterministic multi-day histories (~630 messages/day, mixed lengths, inline formatting, links, replies, reactions, pins, and data-URI images every 19 messages with intrinsic dimensions `1200×800`, `900×1600`, `640×640`, and `1600×600`) and runs a 17-step sequence at each history size:

1. Cold mount of `1,000` or `10,000` messages scrolled to the bottom row, counting every node in the document and how many belong to message toolbars.
2. Append `200` messages while anchored at the bottom.
3. Prepend `200` older messages while reading in the middle of the history.
4. Expand a text row and a media row above the viewport.
5. Scroll through `80` animation frames (`180 px` per frame), recording frame intervals and Long Tasks.
6. Tap a message with touch emulation at the desktop width, which leaves it hovered as touch browsers do, and tap its reply action.
7. Sweep a real pointer down, up, and down the visible messages (`10 px` a frame), then wheel `40` notches of `120 px` with the pointer resting over the list, each unthrottled and at 6× CPU throttling. Each reports main-thread time, style and layout counts and times (`Performance.getMetrics`), and frames over `33 ms`; the window closes `700 ms` after the last event, so work left for the end of a fade is counted.
8. Hover a visible message, verify hit-testing on the floating action toolbar and where it hangs, move the pointer from the message up onto its reply action, and press it.
9. Open that message's reaction picker, rest the pointer on the picker, and pick; then tap a message again with the mouse resting elsewhere. That tap is recorded, not checked: Chrome gives the hover back to a resting mouse after the next layout, whatever causes it.
10. Tab from one message's first action to the next message's first and Shift+Tab back, then Tab `40` stops from a message at `62%` of the history and Shift+Tab `40` back.
11. Resize the viewport to `620×1020`, verify scroll anchor stability, jump to `20%` and `90%` of the history, and tap a message and then its disclosure at that phone width.
12. Restore `1280×900` and jump to `12%`, `50%`, `88%`, and a reply target.
13. Scroll up `700 px` and expand an above-viewport row again.
14. Open the inline message editor, place the caret at offset `4`, append `30` new messages, and verify editor focus and selection survival.
15. Run native `window.find` for a phrase in an offscreen row at `70%` of the history.
16. Query the browser accessibility tree for that offscreen row, and count the messages that name a way to their actions.
17. Verify total DOM row, group, and message entry counts.

The toolbar checks fail when the toolbar is not where the pointer stopped, leaves while the pointer moves onto it or while its picker is open, or sends an action or reaction to another message. The keyboard checks fail when Tab after a message's last action misses the next message's first, when Shift+Tab does not retrace every stop, or when a stop shows no focus ring. The phone check fails when a tap hides the disclosure or the disclosure does not open and close the sheet.

### Architectural invariants verified by the benchmark

- **CSS layout and style containment**: `.transcript-inner > [data-row-id]` applies `contain: layout style` (omitting `paint` so the floating message toolbar can overhang its row). Hover, reaction, or edit mutations inside one row do not invalidate layout across sibling rows.
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
