# Web Client Benchmarks (`web/bench/`)

Real-browser performance and audio characterization harnesses driving local Chrome/Chromium directly over the Chrome DevTools Protocol (`web/bench/cdp.mjs`). Zero benchmark code ships in production bundles.

| Suite | Directory | Command / Reference |
| --- | --- | --- |
| **10k-message transcript** | `web/bench/transcript/` | `npm run bench:transcript -w stellarr-web` ([see below](#transcript-benchmark)) |
| **150-channel rail drag** | `web/bench/rail/` | `npm run bench:rail-drag -w stellarr-web` ([see below](#channel-rail-drag-benchmark)) |
| **Live-call rail updates** | `web/bench/rail/updates/` | `node web/bench/rail/updates/probe.mjs` ([see below](#channel-rail-live-update-probe)) |
| **Voice DSP & suppressors** | `web/bench/voice/` | [`web/bench/voice/README.md`](voice/README.md) |

All CDP suites resolve Chrome/Chromium from `STELLARR_CHROMIUM` (`CHROME` / `CHROME_PATH`) or standard Linux/macOS binary paths (`60 s` startup deadline via `STELLARR_CHROMIUM_START_SECONDS`, up to 3 retries).

---

## Transcript benchmark

Mounts the production `Transcript`, `MessageGroup`, `createTranscriptPresenter`, and `globals.css` against deterministic `1,000`- and `10,000`-message multi-day histories (formatted text, links, replies, reactions, pins, and sized data-URI images):

```sh
npm run bench:transcript -w stellarr-web -- \
  --messages 1000,10000 --check --out bench/results/transcript.json
```

| Option | Description |
| --- | --- |
| `--messages 1000,10000` | History sizes to test (`1000,10000` default). |
| `--out <path>` / `--label <name>` | Output JSON path (`web/bench/results/transcript.json`) and run label. |
| `--check` / `--no-check` | Toggle DOM, keyboard, toolbar, and scroll-anchor assertions (`--check` default). |
| `--baseline <path>` | Compare timings (`--max-slowdown 2` default) and verify identical keyboard/toolbar behavior against a baseline JSON. |
| `--timeout 600` / `--headed` / `--keep-dist` | Per-size timeout (s), headed browser window, or keep `web/.bench-dist`. |

### Verified invariants across the 17-step run

- **Layout & style containment:** `.transcript-inner > [data-row-id]` sets `contain: layout style` (omitting `paint` so hover toolbars overhang rows cleanly), isolating single-row mutations from sibling layout recalculation.
- **On-demand hover toolbars:** Floating action toolbars mount **only** on the hovered, focused, or active-picker message—verified across cold mount, 80-frame scroll bursts, 6× CPU-throttled pointer sweeps and wheel notches, touch taps (`620×1020` mobile + desktop), and 40-stop `Tab` / `Shift+Tab` keyboard traversals.
- **Structural memoization & editor stability:** Appending/prepending 200 messages reuses unchanged row/group props and preserves caret offset inside an open inline message editor.
- **Full DOM retention:** Avoids virtualization and `content-visibility: auto` so native `window.find` (`Ctrl+F`), accessibility tree queries at `70%` scroll depth, and image-aspect scroll anchoring remain exact.

---

## Channel rail drag benchmark

Measures main-thread cost, React commits, and layout recalculations while dragging channels or voice occupants across a 150-channel, 10-section rail (`web/bench/rail/run.mjs`):

```sh
npm run bench:rail-drag -w stellarr-web -- \
  --throttle 1,4 --drag rows,people --out bench/results/rail-drag.json
```

Supports `--throttle <list>`, `--drag rows,people`, `--step <px>` (`4`), `--hold <moves>` (`400`), `--out <json>`, `--shot <png>`, `--headed`, and `--keep-dist`.

---

## Channel rail live-update probe

Profiles React fiber commits, DOM class flips, and speaking-ring transitions in `RailContainer` across three live browser sessions (`Ala`, `Bob`, `Cyd`) during an active call (`web/bench/rail/updates/`):

```sh
node web/bench/rail/updates/build.mjs                          # 1. Unminified client bundle
node web/bench/rail/updates/mics.mjs /path/to/speech-dir       # 2. 48 kHz looped WAV fixtures
node web/bench/rail/updates/probe.mjs \
  --origin http://127.0.0.1:3110 --mics /path/to/speech-dir \
  --container stellarr-railprobe-api-1 --out rail-updates.json # 3. Run 3-client live probe
node web/bench/rail/updates/analyze.mjs rail-updates.json      # 4. Print commit & transition summary
```
