# Transcript benchmark

A reproducible, real-browser measurement of the long-history transcript. It
builds a small React page that mounts the actual `Transcript`, `MessageGroup`
and message renderer with the application stylesheet, then drives a Chrome that
is already installed on the machine through the DevTools protocol. No browser
package or runtime dependency is downloaded, and nothing here ships in the
application bundle.

## Run

```sh
# From the repository root. Node 22 or newer.
npm run bench:transcript -w stellarr-web -- \
  --messages 1000,10000 --check --out bench/results/transcript.json
```

`STELLARR_CHROMIUM` (or `CHROME` / `CHROME_PATH`) points at the browser when it
is not in one of the usual places:

- `/usr/bin/chromium`, `/usr/bin/chromium-browser`
- `/usr/bin/google-chrome`, `/usr/bin/google-chrome-stable`
- `/Applications/Google Chrome.app/Contents/MacOS/Google Chrome`
- `/Applications/Chromium.app/Contents/MacOS/Chromium`

Useful options:

| Option | Meaning |
| --- | --- |
| `--messages 1000,10000` | History sizes to measure. Default `1000,10000`. |
| `--out <path>` | Where the JSON result is written. Default `web/bench/results/transcript.json`. |
| `--label <name>` | A name recorded in the result file. |
| `--check` / `--no-check` | Correctness checks on (default) or measurements only. |
| `--baseline <path>` | Compare mount/append/prepend with an earlier result. |
| `--max-slowdown 2` | Allowed multiplier against `--baseline` (default 2). |
| `--timeout 240` | Per-size scenario timeout in seconds. |
| `--headed` | Show the browser while debugging. |
| `--keep-dist` | Keep the temporary Vite build in `web/.bench-dist`. |

`--messages=1000,10000` and `--messages1000,10000` are accepted as well as
the spaced form.

## What is measured

The fixture is not a row helper and does not fake layout. It imports the real
components and the same producer chain the providers use:

- `buildTranscript` turns generated messages into day/author groups.
- `createTranscriptPresenter` (one instance per page, exactly as a provider
  creates it) turns those groups into the view models; each append and prepend
  is a new call with the complete message list.
- `Transcript` renders every loaded row into the plain scroller. No row is
  windowed, capped or removed.
- `MessageGroup` renders the real author header, body grammar, reactions,
  reply lines, reserved image boxes and media attachments.
- The application's `globals.css` is built through Vite/Tailwind, so the
  component rules, spacing and line heights are the ones the product uses.

The generated history is deterministic: one message per author block, about
630 messages per day, mixed short and long text, inline formatting and URLs,
periodic replies, reactions, pinned rows, and images every 19 messages with
four intrinsic sizes (`1200×800`, `900×1600`, `640×640`, `1600×600`). The
image data URIs have the same intrinsic dimensions as their attachment
metadata, so a loaded picture does not collapse to a placeholder.

The sequence for each size:

1. Cold mount of 1 000 or 10 000 messages, opening at the real last row.
2. Append 200 messages while at the end.
3. Prepend 200 older messages while reading in the middle.
4. Grow a row and a media row above the reader.
5. Scroll 80 animation frames, 180 px each, sampling frame intervals and long
   tasks.
6. Hover a visible message, check the floating toolbar is the hit target, and
   press its reply action to prove the newest callback runs.
7. Resize the viewport to 620×1020 and verify the visible anchor; jump to 20%
   and 90% of the history at that width.
8. Restore 1280×900 and jump to 12%, 50% and 88%, plus a reply jump.
9. Scroll up 700 px after those jumps and grow a row above the reader again.
10. Open the inline editor, put the caret at position 4, append 30 messages and
    check that focus, selection and the editor survive.
11. Ask native find for a phrase in a cold row at 70% of the history.
12. Ask the accessibility tree for that same offscreen row.
13. Count every row, group and message entry in the DOM.

`--check` fails the process on a correctness regression: every expected row must
be in the DOM; the cold mount and append must reach the real bottom; prepend,
growth, viewport resize and upward scrolling must keep the visible row within
1.5 px; jumps must land centred; edit focus and caret must survive; native find
must select the offscreen phrase; the offscreen row must be in the
accessibility tree; and the page must log no errors. The checks are about
correctness and scaling, not tight wall-clock budgets, so they remain useful on
a loaded CI machine. Timings are recorded for review.

## Baseline and result

Measured on macOS arm64 with Google Chrome 144 headless at 1280×900, in one
browser session per size. Numbers are one representative run; a repeat on the
same host measured 30.2 / 48.9 ms p50/p95 and 157 / 162 ms append/prepend at
10 000 messages. See `results/baseline.json` and `results/after.json` for the
raw result files, including every check.

| 10 000 messages | Before | After |
| --- | ---: | ---: |
| Mount, producer + render | 3 347 ms | 3 050 ms |
| Append 200, producer | 234 ms | 13 ms |
| Append 200, React commit | 497 ms | 52 ms |
| Append 200, total | 883 ms | 149 ms |
| Prepend 200, producer | 229 ms | 17 ms |
| Prepend 200, React commit | 434 ms | 45 ms |
| Prepend 200, total | 940 ms | 168 ms |
| Scroll frame p50 / p95 | 54.2 / 91.4 ms | 29.4 / 48.6 ms |
| Scroll frames over 50 ms | 52 of 80 | 3 of 80 |
| DOM rows / groups / entries | 10 448 / 10 430 / 10 430 | same |
| Scroll height | 1 516 104 px | same |

At 1 000 messages the same code keeps a 16.6 ms p50 scroll frame, and the
append and prepend totals fall from 146/154 ms to 56/82 ms. There is less
history to reuse and less work to skip, so the effect is smaller than at 10 000.

The producer timings include the incremental presenter used by both providers;
the React commit timings are what the transcript now saves by not drawing
unchanged rows. The benchmark replaces the action and edit callbacks with new
functions on every update, as `ChatProvider` does, so a stale memo or a
callback identity leak would show up as a slower append rather than being
hidden by stable test doubles.

What changed:

- `.transcript-inner > [data-row-id]` is `contain: layout style`. Each row's
  internal hover, reaction or edit cannot invalidate layout and style across
  the thousands of rows around it. Paint is deliberately not contained because
  the message toolbar hangs above its row; a real pointer hover check confirms
  the toolbar still receives the hit. A containment-only run at 10 000
  messages measured 32.8 / 56.8 ms p50/p95, so the scroll gain is attributable
  to containment rather than to the producer changes.
- `Transcript` renders a memoised `MessageRow`. The row receives the group
  object and the callbacks that can change its drawing; the author callback is
  bound inside the row, and the editor object belongs only to the row being
  edited. When the provider reuses unchanged group and body objects, unaffected
  rows bail out of React work. A row whose group object changed still renders.
- Every row remains in the document. There is no windowing, no cap and no new
  pagination behaviour; find-in-page, accessibility, focus, selectable text and
  the plain scroller are unchanged. No animation was added, so the existing
  reduced-motion behaviour is untouched.

Mount remains the expensive phase: React still creates every row on a cold
10 000-message open. CSS `content-visibility: auto` was measured as an
alternative and rejected. On this fixture it made a cold 10 000-message mount
14.3 s and an append 32.9 s, and jumps to cold rows landed far from their
target; the row containment kept all correctness checks passing with much less
browser work. That is why the shipped change uses containment and structural
memoisation instead of skipping offscreen layout.

## CI notes

The benchmark needs a Chrome/Chromium binary; it does not need a browser
download, a display, a server or the application API. It builds a temporary
Vite page from the repository and serves it on loopback.

Recommended CI shape:

```sh
STELLARR_CHROMIUM=/usr/bin/google-chrome \
npm run bench:transcript -w stellarr-web -- \
  --messages 1000,10000 --check --out bench/results/ci-transcript.json
```

Run it on pull requests that touch the transcript, message rendering, the chat
presenters or the transcript stylesheet, and on a nightly schedule. A cold
10 000-message run takes roughly 20–40 s on an idle arm64 machine, plus about a
second for the Vite build; allow five minutes under a shared runner. Correctness
checks are the gate. If a timing baseline is desired, store one result per
runner class and pass `--baseline` with a generous `--max-slowdown` (2 is the
default); do not gate on tight millisecond budgets.

### Limitations

- The scroll scenario writes `scrollTop` on animation frames rather than
  synthesising wheel events. It measures layout, style and paint work under
  scrolling, not gesture latency.
- Headless Chrome's `window.find` selects the offscreen phrase but does not
  scroll the nested transcript to it. The check proves native find reaches and
  selects the text; scrolling the find result into view is a visual behaviour
  that must be checked by hand in a headed browser if it is changed.
- The accessibility check fetches the full accessibility tree, which costs about
  3 s at 10 000 messages. This is part of the benchmark, not the application.
- The fixture does not copy the large `web/public` asset tree, so the custom
  font files are not served and the benchmark uses fallback font metrics. Row
  heights therefore differ slightly from production; the containment and row
  count behaviour is unaffected.
- Wall-clock numbers are sensitive to host load, CPU frequency and browser
  version. Compare before/after runs from the same machine and load; the
  correctness checks are the stable part.
