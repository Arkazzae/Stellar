// Real-browser benchmark for dragging in the channel rail. It builds the web
// client, serves it on loopback, answers its API from fixture.mjs and drags a
// channel down a rail of about 150 channels in ten sections, in the Chrome
// already installed on the machine.
//
//   npm run bench:rail-drag -- [--throttle 1,4] [--out result.json] [--shot drag.png]
//
// For each CPU throttling rate it reports, over the pointer moves of one drag:
// React commits per move (split by whether the move changed the target slot),
// the components those commits rendered, forced style and layout work (style
// or layout run from inside script, which is a read after a write), and the
// longest task on the page's main thread.
//
// STELLARR_CHROMIUM (or CHROME) points at the browser when it is not in a
// usual place; see web/bench/cdp.mjs.
import { createServer } from "node:http";
import { readFile, rm, writeFile } from "node:fs/promises";
import { extname, resolve, sep } from "node:path";

import { build } from "vite";

import { launch } from "../cdp.mjs";
import { buildServer, createApi } from "./fixture.mjs";

const here = import.meta.dirname;
const webRoot = resolve(here, "..", "..");
const distRoot = resolve(webRoot, ".bench-dist", "rail");

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".woff2": "font/woff2",
  ".ogg": "audio/ogg",
};

function parseArgs(argv) {
  // Long enough at the bottom edge for the rail to scroll through every section.
  const options = { throttle: [1, 4], out: null, shot: null, headed: false, keepDist: false, step: 4, hold: 400 };
  for (let index = 0; index < argv.length; index += 1) {
    const [flag, inline] = argv[index].split("=", 2);
    const value = () => inline ?? argv[++index];
    switch (flag) {
      case "--throttle":
        options.throttle = value().split(",").map(Number).filter((rate) => rate >= 1);
        break;
      case "--out":
        options.out = resolve(process.cwd(), value());
        break;
      case "--shot":
        options.shot = resolve(process.cwd(), value());
        break;
      case "--step":
        options.step = Number(value());
        break;
      case "--hold":
        options.hold = Number(value());
        break;
      case "--headed":
        options.headed = true;
        break;
      case "--keep-dist":
        options.keepDist = true;
        break;
      default:
        throw new Error(
          `Unknown option ${argv[index]}. Options: --throttle 1,4  --out <file>  --shot <file.png>  ` +
            "--step <px per move>  --hold <moves at the edge>  --headed  --keep-dist",
        );
    }
  }
  return options;
}

async function buildClient() {
  const started = performance.now();
  await build({
    root: webRoot,
    configFile: resolve(webRoot, "vite.config.ts"),
    configLoader: "runner",
    mode: "production",
    logLevel: "warn",
    // Unminified, so the rendered components can be named; React is still its
    // production build.
    build: { outDir: distRoot, emptyOutDir: true, minify: false, sourcemap: false, reportCompressedSize: false },
  });
  return performance.now() - started;
}

/** The built documents, with every address that is not a file answered by the web client's page. */
function serve() {
  const server = createServer(async (request, response) => {
    const { pathname } = new URL(request.url ?? "/", "http://127.0.0.1");
    const file = resolve(distRoot, `.${decodeURIComponent(pathname)}`);
    const page = resolve(distRoot, "index.html");
    let body;
    let path = file;
    try {
      if (!file.startsWith(distRoot + sep)) throw new Error("outside");
      body = await readFile(file);
    } catch {
      if (extname(pathname) !== "") {
        response.writeHead(404).end();
        return;
      }
      path = page;
      body = await readFile(page);
    }
    response.writeHead(200, {
      "content-type": MIME[extname(path)] ?? "application/octet-stream",
      "cache-control": "no-store",
    });
    response.end(body);
  });
  return new Promise((resolveListen, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => resolveListen({ server, origin: `http://127.0.0.1:${server.address().port}` }));
  });
}

/*
 * Runs in the page before React does. React reports every commit to the
 * DevTools hook when one is installed, production build included; this one
 * counts them and, as DevTools does, the components each commit rendered:
 * a fiber rendered when it is new or carries the PerformedWork flag, and a
 * subtree whose child list was not rebuilt was skipped whole.
 */
const PROBE = `(() => {
  const PERFORMED_WORK = 1;
  const COMPONENTS = new Set([0, 1, 11, 14, 15]);
  const nameOf = (fiber) => {
    const type = fiber.type;
    if (type == null) return "?";
    return type.displayName || type.name || type.type?.displayName || type.type?.name || type.render?.name || "?";
  };
  const walk = (fiber, names) => {
    for (let child = fiber.child; child !== null; child = child.sibling) {
      const previous = child.alternate;
      if (COMPONENTS.has(child.tag) && (previous === null || (child.flags & PERFORMED_WORK) !== 0)) names.push(nameOf(child));
      if (previous === null || child.child !== previous.child) walk(child, names);
    }
  };
  const bench = (window.__railBench = { moves: [], drop: null, lift: null, phase: "idle", commits: 0 });
  window.__REACT_DEVTOOLS_GLOBAL_HOOK__ = {
    isDisabled: false,
    supportsFiber: true,
    renderers: new Map(),
    inject: () => 1,
    checkDCE: () => {},
    onCommitFiberUnmount: () => {},
    onPostCommitFiberRoot: () => {},
    onCommitFiberRoot: (_id, root) => {
      bench.commits += 1;
      if (bench.phase === "idle") return;
      const names = [];
      walk(root.current, names);
      const record = bench.phase === "moving" ? bench.moves.at(-1) : bench[bench.phase];
      if (record === undefined || record === null) return;
      record.commits += 1;
      record.components.push(...names);
    },
  };
  const blank = () => ({ commits: 0, components: [], slotChanged: false });
  bench.start = () => { bench.lift = blank(); bench.phase = "lift"; };
  bench.moved = () => { bench.moves.push(blank()); bench.phase = "moving"; };
  bench.released = () => { bench.drop = blank(); bench.phase = "drop"; };
  // The live region speaks once per new target, which is what a slot change is.
  new MutationObserver((records) => {
    if (bench.phase !== "moving") return;
    if (records.some((record) => record.target.parentElement?.closest?.('[aria-live="assertive"]') || record.target.closest?.('[aria-live="assertive"]'))) {
      const last = bench.moves.at(-1);
      if (last) last.slotChanged = true;
    }
  }).observe(document, { subtree: true, childList: true, characterData: true });
})();`;

const JS_EVENTS = new Set([
  "FunctionCall",
  "EventDispatch",
  "FireAnimationFrame",
  "TimerFire",
  "RunMicrotasks",
  "v8.callFunction",
  "EvaluateScript",
  "FireIdleCallback",
]);

/** Forced style and layout, and task lengths, on the page's main thread between two marks. */
function readTrace(events, fromMark, toMark) {
  const at = (label) =>
    events.find((event) => event.name === "TimeStamp" && event.args?.data?.message === label)?.ts ?? NaN;
  const from = at(fromMark);
  const to = at(toMark);
  if (Number.isNaN(from) || Number.isNaN(to)) throw new Error(`The trace lacks the marks ${fromMark} and ${toMark}.`);
  const threads = new Map();
  for (const event of events) {
    if (event.ph === "M" && event.name === "thread_name" && event.args?.name === "CrRendererMain") {
      threads.set(`${event.pid}:${event.tid}`, []);
    }
  }
  const open = new Map();
  for (const event of events) {
    const key = `${event.pid}:${event.tid}`;
    const list = threads.get(key);
    if (list === undefined) continue;
    const frame = event.args?.beginData?.stackTrace?.[0];
    const caller = frame === undefined ? undefined : `${frame.functionName || "(anonymous)"} ${frame.url.split("/").at(-1)}:${frame.lineNumber}`;
    if (event.ph === "X") list.push({ name: event.name, start: event.ts, end: event.ts + (event.dur ?? 0), caller });
    else if (event.ph === "B") (open.get(key) ?? open.set(key, []).get(key)).push({ ...event, caller });
    else if (event.ph === "E") {
      const begin = open.get(key)?.pop();
      if (begin !== undefined) list.push({ name: begin.name, start: begin.ts, end: event.ts, caller: begin.caller });
    }
  }
  const result = { forcedLayouts: 0, forcedStyle: 0, longestTask: 0, tasksOver50: 0, forcedBy: {} };
  const blame = (event) => {
    const caller = event.caller ?? "(no stack)";
    result.forcedBy[caller] = (result.forcedBy[caller] ?? 0) + 1;
  };
  for (const list of threads.values()) {
    list.sort((a, b) => a.start - b.start || b.end - a.end);
    const stack = [];
    for (const event of list) {
      if (event.start < from || event.start > to) continue;
      while (stack.length > 0 && stack.at(-1).end <= event.start) stack.pop();
      const inScript = stack.some((parent) => JS_EVENTS.has(parent.name));
      if (event.name === "Layout" && inScript) {
        result.forcedLayouts += 1;
        blame(event);
      }
      if (event.name === "UpdateLayoutTree" && inScript) {
        result.forcedStyle += 1;
        blame(event);
      }
      if (event.name === "RunTask") {
        const length = (event.end - event.start) / 1000;
        result.longestTask = Math.max(result.longestTask, length);
        if (length > 50) result.tasksOver50 += 1;
      }
      stack.push(event);
    }
  }
  result.longestTask = Math.round(result.longestTask * 10) / 10;
  return result;
}

const frame = (page) => page.eval("new Promise((done) => requestAnimationFrame(() => setTimeout(done, 0)))");

async function scenario(browser, origin, rate, options) {
  const page = await browser.newPage();
  const api = createApi(buildServer());
  page.on((method, params) => {
    if (method !== "Fetch.requestPaused") return;
    const { requestId, request } = params;
    // The hub is left waiting rather than refused: a refusal makes the client
    // retry, and every retry is a commit that has nothing to do with the drag.
    if (new URL(request.url).pathname.startsWith("/hubs/")) return;
    const { status, body } = api.answer(request.method, request.url, request.postData);
    void page.cmd("Fetch.fulfillRequest", {
      requestId,
      responseCode: status,
      responseHeaders: [{ name: "content-type", value: "application/json; charset=utf-8" }],
      ...(body === undefined ? {} : { body: Buffer.from(JSON.stringify(body)).toString("base64") }),
    });
  });
  await page.cmd("Fetch.enable", { patterns: [{ urlPattern: "*/api/*" }, { urlPattern: "*/hubs/*" }] });
  await page.cmd("Page.addScriptToEvaluateOnNewDocument", { source: PROBE });
  await page.goto(`${origin}/`);
  await page.waitFor("document.querySelectorAll('.rail .channel-main').length >= 150", 60_000);
  // Fonts and the first paint settle before anything is measured.
  await page.eval("document.fonts.ready.then(() => new Promise((done) => setTimeout(done, 300)))");

  const start = await page.eval(`(() => {
    const scroll = document.querySelector('.rail-scroll').getBoundingClientRect();
    const row = document.querySelector('.rail .text-channel-node .channel-main').getBoundingClientRect();
    return { x: Math.round(row.left + 40), y: Math.round(row.top + row.height / 2), bottom: Math.floor(scroll.bottom) };
  })()`);

  await page.cmd("Emulation.setCPUThrottlingRate", { rate });
  const trace = [];
  const stop = page.on((method, params) => {
    if (method === "Tracing.dataCollected") trace.push(...params.value);
  });
  const traced = new Promise((done) => {
    const off = page.on((method) => {
      if (method !== "Tracing.tracingComplete") return;
      off();
      done();
    });
  });
  await page.cmd("Tracing.start", {
    transferMode: "ReportEvents",
    traceConfig: {
      includedCategories: [
        "devtools.timeline",
        "disabled-by-default-devtools.timeline",
        "disabled-by-default-devtools.timeline.stack",
        "toplevel",
        "v8.execute",
      ],
    },
  });
  await frame(page);
  // Marks in the trace, which keeps a clock of its own.
  const mark = (label) => page.eval(`console.timeStamp(${JSON.stringify(label)})`);

  const mouse = (type, y, extra = {}) =>
    page.cmd("Input.dispatchMouseEvent", { type, x: start.x, y, button: "left", buttons: 1, clickCount: 1, ...extra });

  await page.eval("window.__railBench.start()");
  await mark("lift");
  await mouse("mousePressed", start.y);
  await mouse("mouseMoved", start.y + 8);
  await frame(page);
  await page.eval("window.__railBench.moved()");
  await mark("moves");
  let y = start.y + 8;
  const edge = start.bottom - 6;
  let moves = 0;
  let shotTaken = false;
  while (y < edge) {
    y = Math.min(edge, y + options.step);
    await page.eval("window.__railBench.moved()");
    await mouse("mouseMoved", y);
    await frame(page);
    moves += 1;
    if (options.shot !== null && !shotTaken && rate === options.throttle[0] && y > start.y + 260) {
      shotTaken = true;
      const { data } = await page.cmd("Page.captureScreenshot", { format: "png" });
      await writeFile(options.shot, Buffer.from(data, "base64"));
    }
  }
  // Held at the bottom edge, the rail scrolls under the carried channel.
  for (let index = 0; index < options.hold; index += 1) {
    await page.eval("window.__railBench.moved()");
    await mouse("mouseMoved", edge - (index % 2));
    await frame(page);
    moves += 1;
  }
  await mark("moved");
  await page.eval("window.__railBench.released()");
  await mouse("mouseReleased", edge, { buttons: 0 });
  for (let index = 0; index < 5; index += 1) await frame(page);
  await mark("dropped");
  const scrolled = await page.eval(`(() => {
    const scroller = document.querySelector('.rail-scroll');
    return { top: Math.round(scroller.scrollTop), max: scroller.scrollHeight - scroller.clientHeight };
  })()`);

  await page.cmd("Tracing.end");
  await traced;
  stop();
  await page.cmd("Emulation.setCPUThrottlingRate", { rate: 1 });

  const bench = await page.eval(`(() => {
    const b = window.__railBench;
    return { lift: b.lift, drop: b.drop, moves: b.moves };
  })()`);
  await page.cmd("Page.close").catch(() => {});

  const moveRecords = bench.moves.slice(1);
  const steady = moveRecords.filter((move) => !move.slotChanged);
  const changed = moveRecords.filter((move) => move.slotChanged);
  const stat = (list, pick) => {
    if (list.length === 0) return { mean: 0, max: 0 };
    const values = list.map(pick);
    return {
      mean: Math.round((values.reduce((sum, value) => sum + value, 0) / values.length) * 100) / 100,
      max: Math.max(...values),
    };
  };
  const histogram = (list) => {
    const counts = {};
    for (const name of list.flatMap((move) => move.components)) counts[name] = (counts[name] ?? 0) + 1;
    return Object.fromEntries(Object.entries(counts).sort((a, b) => b[1] - a[1]).slice(0, 8));
  };

  return {
    throttle: rate,
    moves,
    slotChanges: changed.length,
    scrolled,
    commitsPerMove: {
      slotUnchanged: stat(steady, (move) => move.commits),
      slotChanged: stat(changed, (move) => move.commits),
    },
    componentsPerMove: {
      slotUnchanged: stat(steady, (move) => move.components.length),
      slotChanged: stat(changed, (move) => move.components.length),
    },
    renderedWhileMoving: histogram(moveRecords),
    lift: { commits: bench.lift.commits, components: bench.lift.components.length, ...readTrace(trace, "lift", "moves") },
    moving: readTrace(trace, "moves", "moved"),
    drop: { commits: bench.drop.commits, components: bench.drop.components.length, ...readTrace(trace, "moved", "dropped") },
    sent: api.requests.filter((request) => request.path.startsWith("/api/admin/channels")),
  };
}

function print(result) {
  const lines = [
    `CPU ${result.throttle}x  ${result.moves} moves, ${result.slotChanges} slot changes, ` +
      `rail scrolled ${result.scrolled.top} of ${result.scrolled.max} px`,
    `  commits per move      slot unchanged ${result.commitsPerMove.slotUnchanged.mean} (max ${result.commitsPerMove.slotUnchanged.max})` +
      `   slot changed ${result.commitsPerMove.slotChanged.mean} (max ${result.commitsPerMove.slotChanged.max})`,
    `  components per move   slot unchanged ${result.componentsPerMove.slotUnchanged.mean} (max ${result.componentsPerMove.slotUnchanged.max})` +
      `   slot changed ${result.componentsPerMove.slotChanged.mean} (max ${result.componentsPerMove.slotChanged.max})`,
    `  forced while moving   ${result.moving.forcedLayouts} layouts, ${result.moving.forcedStyle} style recalcs` +
      `${Object.keys(result.moving.forcedBy).length === 0 ? "" : `, from ${JSON.stringify(result.moving.forcedBy)}`}`,
    `  longest task          ${result.moving.longestTask} ms while moving (${result.moving.tasksOver50} over 50 ms),` +
      ` ${result.lift.longestTask} ms lift, ${result.drop.longestTask} ms drop`,
    `  lift                  ${result.lift.commits} commits, ${result.lift.components} components, ` +
      `${result.lift.forcedLayouts} forced layouts, ${result.lift.forcedStyle} forced style`,
    `  drop                  ${result.drop.commits} commits, ${result.drop.components} components, ` +
      `${result.drop.forcedLayouts} forced layouts, ${result.drop.forcedStyle} forced style`,
    `  rendered while moving ${JSON.stringify(result.renderedWhileMoving)}`,
    `  sent                  ${result.sent.map((request) => `${request.method} ${request.path}`).join(", ") || "nothing"}`,
  ];
  console.log(lines.join("\n"));
}

const options = parseArgs(process.argv.slice(2));
console.log("Building the web client…");
console.log(`Built in ${Math.round(await buildClient())} ms.`);
const { server, origin } = await serve();
const browser = await launch({ width: 1280, height: 900, headless: !options.headed });
const results = [];
try {
  for (const rate of options.throttle) {
    const result = await scenario(browser, origin, rate, options);
    print(result);
    results.push(result);
  }
} finally {
  await browser.close();
  server.close();
  if (!options.keepDist) await rm(distRoot, { recursive: true, force: true });
}
// The refused hub is expected to complain; anything else is worth reading.
const errors = browser.pageErrors.filter((error) => !/negotiat|start the connection/.test(error));
if (errors.length > 0) console.log(`Page errors:\n  ${errors.slice(0, 10).join("\n  ")}`);
if (options.out !== null) {
  await writeFile(options.out, `${JSON.stringify({ chrome: browser.executable, results }, null, 2)}\n`);
  console.log(`Wrote ${options.out}`);
}
