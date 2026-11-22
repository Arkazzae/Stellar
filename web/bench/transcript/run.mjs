// Real-browser benchmark for the transcript. It builds the React fixture in
// web/bench/transcript with the application's own stylesheet, opens it in the
// Chrome already installed on the machine, and drives the actual Transcript,
// MessageGroup and producer chain through a long history.
//
//   npm run bench:transcript -- --messages 1000,10000 --out results/baseline.json
//
// No browser package is downloaded or bundled. STELLARR_CHROMIUM (or CHROME)
// points at the browser when it is not in a usual place.
import { createServer } from "node:http";
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { extname, resolve, sep } from "node:path";

import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/postcss";
import { build } from "vite";

import { stellarrArtifactSizes } from "../../artifactSizes.ts";
import { launch } from "../cdp.mjs";

const here = import.meta.dirname;
const webRoot = resolve(here, "..", "..");
const fixtureRoot = resolve(here);
const distRoot = resolve(webRoot, ".bench-dist");

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".woff2": "font/woff2",
};

function usage() {
  return [
    "Usage: npm run bench:transcript -- [options]",
    "",
    "  --messages 1000,10000   history sizes (default: 1000,10000)",
    "  --out <path>            write the machine-readable result here",
    "  --label <name>          name the run in the result",
    "  --check                 fail on a correctness regression (default)",
    "  --no-check              measurements only",
    "  --baseline <path>       compare timings with an earlier result",
    "  --max-slowdown <n>      allowed multiplier against --baseline (default 2)",
    "  --timeout <seconds>     per-size scenario timeout (default 600)",
    "  --headed                show the browser (debugging only)",
    "  --keep-dist             keep .bench-dist after the run",
  ].join("\n");
}

function parseArgs(argv) {
  const options = {
    messages: [1000, 10000],
    out: resolve(webRoot, "bench", "results", "transcript.json"),
    label: "transcript",
    check: true,
    baseline: null,
    maxSlowdown: 2,
    timeout: 600,
    headed: false,
    keepDist: false,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    // `--messages1000,10000` is easy to write by accident; accept it as the
    // same option as the spaced and `--messages=` forms.
    if (
      argument.startsWith("--messages") &&
      argument !== "--messages" &&
      !argument.startsWith("--messages=")
    ) {
      options.messages = argument
        .slice("--messages".length)
        .split(",")
        .map((item) => Number(item.trim()))
        .filter((item) => Number.isFinite(item) && item > 0);
      continue;
    }
    const equals = argument.indexOf("=");
    const flag = equals === -1 ? argument : argument.slice(0, equals);
    const inline = equals === -1 ? undefined : argument.slice(equals + 1);
    const value = inline ?? argv[index + 1];
    switch (flag) {
      case "--messages":
        options.messages = (value ?? "")
          .split(",")
          .map((item) => Number(item.trim()))
          .filter((item) => Number.isFinite(item) && item > 0);
        if (inline === undefined) index += 1;
        break;
      case "--out":
        options.out = resolve(process.cwd(), value ?? options.out);
        if (inline === undefined) index += 1;
        break;
      case "--label":
        options.label = value ?? options.label;
        if (inline === undefined) index += 1;
        break;
      case "--check":
        options.check = true;
        break;
      case "--no-check":
        options.check = false;
        break;
      case "--baseline":
        options.baseline = resolve(process.cwd(), value ?? "");
        if (inline === undefined) index += 1;
        break;
      case "--max-slowdown":
        options.maxSlowdown = Number(value ?? "2");
        if (inline === undefined) index += 1;
        break;
      case "--timeout":
        options.timeout = Number(value ?? "600");
        if (inline === undefined) index += 1;
        break;
      case "--headed":
        options.headed = true;
        break;
      case "--keep-dist":
        options.keepDist = true;
        break;
      case "--help":
      case "-h":
        console.log(usage());
        process.exit(0);
        break;
      default:
        throw new Error(`Unknown option ${argument}\n\n${usage()}`);
    }
  }
  if (options.messages.length === 0) throw new Error("--messages needs at least one size");
  return options;
}

async function buildFixture() {
  const started = performance.now();
  await build({
    configFile: false,
    root: fixtureRoot,
    base: "/",
    publicDir: false,
    // The page imports parts of the app, and the app's voice code reads the
    // sizes of the prebuilt files from the manifest. `virtual:artifact-sizes`
    // is the app's own plugin: without it the build cannot resolve the import.
    plugins: [react(), stellarrArtifactSizes()],
    resolve: {
      alias: { "@": resolve(webRoot, "src/stellarr") },
    },
    css: {
      postcss: { plugins: [tailwindcss()] },
    },
    build: {
      outDir: distRoot,
      emptyOutDir: true,
      target: "es2022",
      minify: false,
      sourcemap: false,
      cssCodeSplit: false,
      reportCompressedSize: false,
    },
    logLevel: "warn",
  });
  return performance.now() - started;
}

function serve() {
  const server = createServer(async (request, response) => {
    try {
      const { pathname } = new URL(request.url ?? "/", "http://127.0.0.1");
      const relative = pathname === "/" ? "/index.html" : decodeURIComponent(pathname);
      const file = resolve(distRoot, `.${relative}`);
      if (!file.startsWith(distRoot + sep)) {
        response.writeHead(404).end();
        return;
      }
      const body = await readFile(file);
      response.writeHead(200, {
        "content-type": MIME[extname(file)] ?? "application/octet-stream",
        "content-length": body.byteLength,
        "cache-control": "no-store",
      });
      response.end(body);
    } catch {
      response.writeHead(404).end();
    }
  });
  return new Promise((resolveListen, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      resolveListen({ server, url: `http://127.0.0.1:${port}/` });
    });
  });
}

function check(name, ok, detail) {
  return { name, ok, detail };
}

const sleep = (ms) => new Promise((resolveWait) => setTimeout(resolveWait, ms));

/** Where the pointer rests when no message should be under it: the scroller's top padding. */
const PARK = { x: 2, y: 2 };

/** Long enough for a toolbar's fade and anything that waits for it. */
const SETTLE_MS = 700;

function move(page, x, y) {
  return page.cmd("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
}

/** A pointer travelling in even steps, as a hand does, rather than jumping. */
async function travel(page, from, to, steps = 8) {
  for (let step = 1; step <= steps; step += 1) {
    await move(page, from.x + ((to.x - from.x) * step) / steps, from.y + ((to.y - from.y) * step) / steps);
    await sleep(16);
  }
}

async function click(page, { x, y }) {
  await page.cmd("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "left", buttons: 1, clickCount: 1 });
  await page.cmd("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "left", buttons: 0, clickCount: 1 });
}

/** A finger rests on the glass for a moment, as a tap does, rather than lifting in the same frame. */
async function tap(page, { x, y }) {
  await page.cmd("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ x, y }] });
  await sleep(80);
  await page.cmd("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
}

/** One Tab, or Shift+Tab, and where focus is a frame later. */
async function pressTab(page, backward = false) {
  const shift = { key: "Shift", code: "ShiftLeft", windowsVirtualKeyCode: 16, nativeVirtualKeyCode: 16 };
  const tab = { key: "Tab", code: "Tab", windowsVirtualKeyCode: 9, nativeVirtualKeyCode: 9, modifiers: backward ? 8 : 0 };
  if (backward) await page.cmd("Input.dispatchKeyEvent", { type: "rawKeyDown", ...shift, modifiers: 8 });
  await page.cmd("Input.dispatchKeyEvent", { type: "rawKeyDown", ...tab });
  await page.cmd("Input.dispatchKeyEvent", { type: "keyUp", ...tab });
  if (backward) await page.cmd("Input.dispatchKeyEvent", { type: "keyUp", ...shift, modifiers: 0 });
  return page.eval(
    "new Promise((done) => requestAnimationFrame(() => done(window.__bench.describeFocus())))",
  );
}

async function metricsOf(page) {
  const { metrics } = await page.cmd("Performance.getMetrics");
  return Object.fromEntries(metrics.map(({ name, value }) => [name, value]));
}

/** Main-thread work between two readings, in milliseconds of the throttled clock. */
function workBetween(before, after) {
  const ms = (name) => Math.round((after[name] - before[name]) * 10_000) / 10;
  return {
    taskMs: ms("TaskDuration"),
    scriptMs: ms("ScriptDuration"),
    styleMs: ms("RecalcStyleDuration"),
    styles: after.RecalcStyleCount - before.RecalcStyleCount,
    layoutMs: ms("LayoutDuration"),
    layouts: after.LayoutCount - before.LayoutCount,
  };
}

/**
 * Runs `drive` with the CPU slowed by `rate` and reports the main-thread
 * work it caused. The window opens with the pointer already where `drive`
 * starts and closes only after the page has settled, so a design that defers
 * work until a fade ends is charged for it.
 */
async function measured(page, rate, drive) {
  await sleep(SETTLE_MS);
  await page.cmd("Emulation.setCPUThrottlingRate", { rate });
  try {
    await page.eval("window.__bench.startFrames()");
    const before = await metricsOf(page);
    const started = performance.now();
    const driven = await drive();
    const driveMs = Math.round(performance.now() - started);
    await sleep(SETTLE_MS);
    const after = await metricsOf(page);
    const frames = await page.eval("window.__bench.stopFrames()");
    return { rate, driveMs, ...driven, ...workBetween(before, after), frames };
  } finally {
    await page.cmd("Emulation.setCPUThrottlingRate", { rate: 1 });
  }
}

/** Sends events at most one per frame, each acknowledged before the next. */
async function paced(count, send) {
  for (let index = 0; index < count; index += 1) {
    const sent = performance.now();
    await send(index);
    const left = 16 - (performance.now() - sent);
    if (left > 0) await sleep(left);
  }
}

/**
 * Toolbars left on messages nothing is using, once a fade still running on a
 * slow page has had up to three seconds to end.
 */
async function idleToolbarsSettled(page) {
  const started = performance.now();
  let idle = await page.eval("window.__bench.idleToolbars()");
  while (idle.count > 0 && performance.now() - started < 3000) {
    await sleep(100);
    idle = await page.eval("window.__bench.idleToolbars()");
  }
  return { ...idle, waitedMs: Math.round(performance.now() - started) };
}

/**
 * A pointer moving down, up and down again through the visible messages, in
 * 10 px steps along one column: every message it crosses is hovered and left.
 */
async function pointerSweep(page, rate) {
  const path = await page.eval("window.__bench.sweepPath()");
  const span = path.bottom - path.top;
  const steps = Math.floor(span / 10) + 1;
  const passes = 3;
  await move(page, path.x, path.top);
  const result = await measured(page, rate, async () => {
    await paced(steps * passes, (index) => {
      const pass = Math.floor(index / steps);
      const offset = (index % steps) * 10;
      return move(page, path.x, pass % 2 === 0 ? path.top + offset : path.bottom - offset);
    });
    // Where the sweep stops, the message under the pointer shows its toolbar.
    const missing = await page.eval("window.__bench.missingToolbars()");
    await move(page, PARK.x, PARK.y);
    return { moves: steps * passes, visibleGroups: path.visibleGroups, missingToolbars: missing.count };
  });
  const idle = await idleToolbarsSettled(page);
  return { ...result, idleToolbarsAfter: idle.count };
}

/**
 * Wheel scrolling with the pointer resting over the list, as a reader
 * scrolls: the message under the pointer changes as the rows move beneath it.
 */
async function wheelScroll(page, rate) {
  const start = await page.eval("window.__bench.wheelStart(0.3)");
  await move(page, start.x, start.y);
  const events = 40;
  const deltaY = 120;
  const result = await measured(page, rate, async () => {
    await paced(events, () =>
      page.cmd("Input.dispatchMouseEvent", { type: "mouseWheel", x: start.x, y: start.y, deltaX: 0, deltaY }),
    );
    return { events, deltaY };
  });
  const end = await page.eval("window.__bench.snapshot()");
  // The rows moved under a resting pointer: the one it now rests on shows its toolbar.
  const missing = await page.eval("window.__bench.missingToolbars()");
  const idle = await idleToolbarsSettled(page);
  await move(page, PARK.x, PARK.y);
  return {
    ...result,
    scrolled: Math.round(end.scrollTop - start.scrollTop),
    missingToolbars: missing.count,
    idleToolbarsAfter: idle.count,
  };
}

function expectedMatches(expected, snapshot, label) {
  const checks = [];
  if (expected === null || snapshot === null) return checks;
  checks.push(
    check(`${label}.all-rows-in-dom`, snapshot.rows === expected.rows, {
      expected: expected.rows,
      actual: snapshot.rows,
    }),
  );
  checks.push(
    check(`${label}.all-groups-in-dom`, snapshot.groups === expected.groups, {
      expected: expected.groups,
      actual: snapshot.groups,
    }),
  );
  checks.push(
    check(`${label}.all-bodies-in-dom`, snapshot.entries === expected.bodies, {
      expected: expected.bodies,
      actual: snapshot.entries,
    }),
  );
  return checks;
}

/**
 * Makes the screen a touch screen. Its coarse pointer brings in the larger
 * touch targets a moment later, which moves the rows, so the tap is aimed
 * only once they have settled.
 */
async function touchScreen(page) {
  await page.cmd("Emulation.setTouchEmulationEnabled", { enabled: true, maxTouchPoints: 1 });
  await sleep(SETTLE_MS);
  await page.eval("window.__bench.waitForLayout()");
}

/**
 * A finger on a wide touch screen, which has the desktop layout: a tap leaves
 * the message hovered, as touch browsers do, and a second tap on its reply
 * action reaches that message.
 */
async function touchTap(page) {
  await touchScreen(page);
  try {
    const target = await page.eval("window.__bench.toolbarTarget()");
    await tap(page, target);
    await sleep(SETTLE_MS);
    const tapped = await page.eval("window.__bench.toolbarState()");
    let replied = null;
    if (tapped.toolbar === true && tapped.opacity === "1") {
      await tap(page, await page.eval('window.__bench.toolbarButtonPoint("Reply")'));
      await sleep(100);
      replied = (await page.eval("window.__bench.callbackInfo()")).lastAction;
    }
    const works =
      tapped.hovered === true &&
      tapped.opacity === "1" &&
      replied?.action === "reply" &&
      replied.messageId === target.row;
    return { row: target.row, tapped, replied, works };
  } finally {
    await page.cmd("Emulation.setTouchEmulationEnabled", { enabled: false });
  }
}

/**
 * A finger at the phone width, where every message shows one disclosure: a
 * tap on a message leaves the disclosure shown, a tap on the disclosure opens
 * the sheet, and Escape closes it and gives focus back to the disclosure.
 */
async function phoneTap(page) {
  await touchScreen(page);
  try {
    const target = await page.eval("window.__bench.toolbarTarget()");
    await tap(page, target);
    await sleep(SETTLE_MS);
    const tapped = await page.eval(`window.__bench.phoneActions(${JSON.stringify(target.row)})`);
    let opened = null;
    let closed = null;
    if (tapped.shown === true) {
      await tap(page, tapped);
      await sleep(SETTLE_MS);
      opened = await page.eval(`window.__bench.phoneActions(${JSON.stringify(target.row)})`);
      const escape = { key: "Escape", code: "Escape", windowsVirtualKeyCode: 27, nativeVirtualKeyCode: 27 };
      await page.cmd("Input.dispatchKeyEvent", { type: "rawKeyDown", ...escape });
      await page.cmd("Input.dispatchKeyEvent", { type: "keyUp", ...escape });
      await sleep(SETTLE_MS);
      closed = await page.eval(`window.__bench.phoneActions(${JSON.stringify(target.row)})`);
    }
    const works =
      tapped.shown === true &&
      tapped.hitDisclosure === true &&
      opened?.sheet === true &&
      closed?.sheet === false &&
      closed.focusOnDisclosure === true;
    return { row: target.row, tapped, opened, closed, works };
  } finally {
    await page.cmd("Emulation.setTouchEmulationEnabled", { enabled: false });
  }
}

async function accessibilityFor(page, rowId, markerText) {
  try {
    await page.cmd("DOM.enable");
    const { root } = await page.cmd("DOM.getDocument", { depth: 0 });
    const { nodeId } = await page.cmd("DOM.querySelector", {
      nodeId: root.nodeId,
      selector: `[data-row-id="${rowId}"] .msg-text-content`,
    });
    if (nodeId === 0) return { found: false, reason: "text node not found" };
    const started = performance.now();
    const { nodes } = await page.cmd("Accessibility.getFullAXTree");
    const elapsedMs = performance.now() - started;
    const names = nodes
      .filter((node) => node.ignored !== true)
      .map((node) => node.name?.value ?? "")
      .filter((name) => name !== "");
    const found = names.some((name) => name.includes(markerText));
    // Each message's way to its actions, by the name it is read out with:
    // its toolbar, or the one control that stands for it.
    const actionWays = nodes.filter(
      (node) =>
        node.ignored !== true &&
        ["toolbar", "button"].includes(node.role?.value) &&
        node.name?.value === "Message actions",
    ).length;
    return {
      found,
      actionWays,
      nodeCount: nodes.length,
      elapsedMs: Math.round(elapsedMs),
      names: names.slice(0, 3).map((name) => name.slice(0, 80)),
    };
  } catch (error) {
    return { found: false, reason: error.message };
  }
}

async function runSize(browser, pageUrl, size, progress) {
  const began = performance.now();
  // Where the run has got to, so a timeout can say which scenario it was in.
  const mark = (name) => progress.marks.push({ name, atMs: Math.round(performance.now() - began) });
  const page = await browser.newPage();
  browser.pageErrors.length = 0;
  const checks = [];
  try {
    await page.goto(pageUrl);
    await page.waitFor("window.__bench?.ready === true", 60_000);

    const mount = await page.eval(`window.__bench.mount(${size})`);
    const expected = await page.eval("window.__bench.expected()");
    checks.push(...expectedMatches(expected, mount, "mount"));
    checks.push(
      check("mount.at-end", mount.bottomGap <= 2 && mount.lastRowVisible === true, {
        bottomGap: mount.bottomGap,
        lastRowVisible: mount.lastRowVisible,
        lastRowBottom: mount.lastRowBottom,
      }),
    );
    checks.push(
      check("mount.work-scaling", mount.rows >= size && mount.groups >= size, {
        rows: mount.rows,
        groups: mount.groups,
      }),
    );
    // Nothing is under the pointer or focused yet, so no message has a toolbar.
    checks.push(check("mount.no-toolbars", mount.document.toolbars === 0, mount.document));

    mark("append");
    const append = await page.eval("window.__bench.append(200)");
    const appendExpected = await page.eval("window.__bench.expected()");
    checks.push(...expectedMatches(appendExpected, append, "append"));
    checks.push(
      check("append.follows-end", append.bottomGap <= 2 && append.lastRowVisible === true, {
        bottomGap: append.bottomGap,
        lastRowVisible: append.lastRowVisible,
      }),
    );

    mark("prepend");
    const prepend = await page.eval("window.__bench.prepend(200)");
    const prependExpected = await page.eval("window.__bench.expected()");
    checks.push(...expectedMatches(prependExpected, prepend.after, "prepend"));
    checks.push(
      check("prepend.keeps-anchor", Math.abs(prepend.delta) <= 1.5, {
        anchor: prepend.id,
        delta: prepend.delta,
        topBefore: prepend.topBefore,
        topAfter: prepend.topAfter,
      }),
    );

    const growth = await page.eval("window.__bench.growAbove(160)");
    checks.push(
      check("grow-above.keeps-anchor", growth.targetHeightAfter > growth.targetHeightBefore && Math.abs(growth.delta) <= 1.5, {
        anchor: growth.id,
        target: growth.target,
        delta: growth.delta,
        contentVisibility: growth.targetContentVisibility,
        heightBefore: growth.targetHeightBefore,
        heightAfter: growth.targetHeightAfter,
      }),
    );

    const mediaGrowth = await page.eval("window.__bench.growMediaAbove(120)");
    checks.push(
      check(
        "grow-media-above.keeps-anchor",
        mediaGrowth.skipped !== true &&
          mediaGrowth.targetHeightAfter > mediaGrowth.targetHeightBefore &&
          Math.abs(mediaGrowth.delta) <= 1.5,
        mediaGrowth.skipped === true
          ? { skipped: true, reason: mediaGrowth.reason }
          : {
              anchor: mediaGrowth.id,
              target: mediaGrowth.target,
              delta: mediaGrowth.delta,
              contentVisibility: mediaGrowth.targetContentVisibility,
              heightBefore: mediaGrowth.targetHeightBefore,
              heightAfter: mediaGrowth.targetHeightAfter,
            },
      ),
    );

    mark("scroll");
    const scroll = await page.eval("window.__bench.scrollRun(80, 180)");

    mark("touch");
    // Before any mouse event, as on a screen that is only touched.
    const touch = await touchTap(page);
    checks.push(check("touch.tap-opens-actions", touch.works, touch));

    // Hovering and wheeling, each unthrottled and with the CPU slowed six
    // times, which is where per-message work the eye never sees shows up.
    await page.cmd("Performance.enable");
    mark("pointer sweep");
    const sweep = { x1: await pointerSweep(page, 1), x6: await pointerSweep(page, 6) };
    mark("wheel");
    const wheel = { x1: await wheelScroll(page, 1), x6: await wheelScroll(page, 6) };
    // The message the pointer stopped on shows its toolbar, and every message
    // it crossed has let its toolbar go again.
    for (const [name, runs] of [["sweep", sweep], ["wheel", wheel]]) {
      for (const [rate, item] of Object.entries(runs)) {
        checks.push(
          check(`${name}.${rate}.pointed-message-has-toolbar`, item.missingToolbars === 0, {
            missingToolbars: item.missingToolbars,
          }),
        );
        checks.push(
          check(`${name}.${rate}.no-idle-toolbars`, item.idleToolbarsAfter === 0, {
            idleToolbarsAfter: item.idleToolbarsAfter,
          }),
        );
      }
    }
    mark("toolbar");

    const toolbarTarget = await page.eval("window.__bench.toolbarTarget()");
    await travel(page, PARK, toolbarTarget);
    await sleep(300);
    const toolbar = await page.eval("window.__bench.toolbarCheck()");
    checks.push(
      check("toolbar.hit-test", toolbar.opacity === "1" && toolbar.hitToolbar === true, {
        row: toolbarTarget.row,
        toolbar,
      }),
    );
    checks.push(
      check(
        "toolbar.hangs-at-the-corner",
        Math.abs(toolbar.place.top + 17) <= 0.5 && Math.abs(toolbar.place.right) <= 0.5,
        toolbar.place,
      ),
    );
    // From the message up onto the button, which hangs over the row above:
    // the toolbar must stay while the pointer crosses onto it.
    const replyPoint = await page.eval('window.__bench.toolbarButtonPoint("Reply")');
    await travel(page, toolbarTarget, replyPoint);
    const onReply = await page.eval("window.__bench.toolbarCheck()");
    await click(page, replyPoint);
    const callbackInfo = await page.eval("window.__bench.callbackInfo()");
    const messageCount = await page.eval("window.__bench.messageCount()");
    checks.push(
      check(
        "toolbar.latest-action",
        onReply.opacity === "1" &&
          callbackInfo.lastAction?.action === "reply" &&
          callbackInfo.lastAction.messageId === toolbarTarget.row &&
          callbackInfo.lastAction.at === messageCount,
        { onReply, callbackInfo, messageCount },
      ),
    );

    // The reaction picker belongs to its message while the pointer is on the
    // picker, and the pick reaches that message.
    const smiley = await page.eval('window.__bench.toolbarButtonPoint("Choose a reaction")');
    await travel(page, replyPoint, smiley, 4);
    await click(page, smiley);
    await sleep(300);
    const pickerAt = await page.eval("window.__bench.pickerPoints()");
    await travel(page, smiley, pickerAt.panel);
    await sleep(300);
    const pickerOpen = await page.eval("window.__bench.toolbarState()");
    checks.push(
      check("picker.keeps-its-message", pickerOpen.picker === true && pickerOpen.toolbar === true, pickerOpen),
    );
    await travel(page, pickerAt.panel, pickerAt.option, 4);
    await click(page, pickerAt.option);
    await sleep(SETTLE_MS);
    const picked = await page.eval("window.__bench.toolbarState()");
    checks.push(
      check(
        "picker.reacts-for-its-message",
        picked.picker === false && picked.lastReact?.messageId === toolbarTarget.row,
        picked,
      ),
    );
    // With the picker gone, its message keeps a toolbar only if the pointer
    // or focus is on it.
    const afterPick = await idleToolbarsSettled(page);
    checks.push(check("picker.lets-go-after-pick", afterPick.count === 0, afterPick));
    await travel(page, pickerAt.option, PARK);
    await sleep(SETTLE_MS);

    // The same tap on a touch screen that also has a mouse, whose pointer
    // rests elsewhere: recorded, not checked. Chrome gives the hover back to
    // that pointer after the next layout, whatever caused it.
    const touchAfterMouse = await touchTap(page);
    // Leave no message hovered while the next scenarios scroll: a pointer
    // resting over the list keeps changing which toolbar is open.
    await travel(page, { x: 640, y: 450 }, PARK, 2);
    await sleep(SETTLE_MS);

    // The keyboard reaches every message's actions in document order, and
    // Shift+Tab walks the same stops back.
    await page.cmd("Emulation.setFocusEmulationEnabled", { enabled: true }).catch(() => {});
    mark("keyboard");
    const pair = await page.eval("window.__bench.keyboardPair(0.5)");
    const pairStops = [pair.focus];
    while (pairStops.length < 16 && pairStops.at(-1).row === pair.first) {
      pairStops.push(await pressTab(page));
    }
    const lastOfFirst = pairStops.at(-2);
    const firstOfSecond = pairStops.at(-1);
    const backToFirst = await pressTab(page, true);
    checks.push(
      check(
        "keyboard.tab-reaches-next-message",
        pair.focus.action === true &&
          lastOfFirst.row === pair.first &&
          lastOfFirst.action === true &&
          firstOfSecond.row === pair.second &&
          firstOfSecond.action === true &&
          firstOfSecond.label === pair.focus.label,
        { pair, stops: pairStops },
      ),
    );
    checks.push(
      check(
        "keyboard.shift-tab-returns",
        backToFirst.row === pair.first && backToFirst.action === true && backToFirst.label === lastOfFirst.label,
        { lastOfFirst, backToFirst },
      ),
    );
    const span = await page.eval("window.__bench.keyboardSpan(0.62)");
    const forward = [span.focus];
    for (let index = 0; index < 40; index += 1) forward.push(await pressTab(page));
    const backward = [forward.at(-1)];
    for (let index = 0; index < 40; index += 1) backward.push(await pressTab(page, true));
    // The same stop, whichever way it was reached; the scroll position is
    // where the walk has got to, so it is left out.
    const stop = ({ row, label, action, ring }) => JSON.stringify({ row, label, action, ring });
    checks.push(
      check(
        "keyboard.shift-tab-retraces",
        [...backward].reverse().map(stop).join() === forward.map(stop).join(),
        { forward, backward },
      ),
    );
    // A stop reached by the keyboard shows the focus ring.
    const ringless = [...pairStops.slice(1), backToFirst, ...forward.slice(1), ...backward.slice(1)].filter(
      (item) => item.ring !== true,
    );
    checks.push(check("keyboard.every-stop-shows-its-ring", ringless.length === 0, ringless.slice(0, 5)));
    const keyboard = { pair: pairStops, back: backToFirst, span: forward, spanBack: backward };
    await page.eval("document.activeElement?.blur()");
    await sleep(SETTLE_MS);
    const afterKeyboard = await idleToolbarsSettled(page);
    checks.push(check("keyboard.lets-go-after-blur", afterKeyboard.count === 0, afterKeyboard));

    mark("resize and jumps");
    await page.eval("window.__bench.capture(0)");
    await page.resize(620, 1020);
    await page.eval("window.__bench.waitForLayout()");
    const resized = await page.eval("window.__bench.measureSaved()");
    const viewport = await page.eval("({width:innerWidth,height:innerHeight})");
    checks.push(
      check("viewport-resize.keeps-anchor", viewport.width === 620 && viewport.height === 1020 && Math.abs(resized.delta) <= 1.5, {
        anchor: resized.id,
        delta: resized.delta,
        viewport,
      }),
    );
    for (const fraction of [0.2, 0.9]) {
      const jump = await page.eval(`window.__bench.jump(${fraction})`);
      checks.push(
        check(`narrow-jump.${fraction}.lands-centred`, Math.abs(jump.centreOffset) <= 4 && jump.waited.stable, {
          id: jump.id,
          centreOffset: jump.centreOffset,
          stable: jump.waited.stable,
        }),
      );
    }
    mark("phone");
    const phone = await phoneTap(page);
    checks.push(check("phone.tap-keeps-the-disclosure-and-opens-the-sheet", phone.works, phone));
    await page.resize(1280, 900);
    await page.eval("window.__bench.waitForLayout()");

    const jumps = [];
    for (const fraction of [0.12, 0.5, 0.88]) {
      const jump = await page.eval(`window.__bench.jump(${fraction})`);
      jumps.push(jump);
      checks.push(
        check(`jump.${fraction}.lands-centred`, Math.abs(jump.centreOffset) <= 4, {
          id: jump.id,
          centreOffset: jump.centreOffset,
          stable: jump.waited.stable,
        }),
      );
    }
    const reply = await page.eval("window.__bench.replyJump()");
    if (reply.skipped !== true) {
      checks.push(
        check("reply-jump.lands-centred", Math.abs(reply.centreOffset) <= 4, {
          id: reply.id,
          centreOffset: reply.centreOffset,
        }),
      );
    }

    const up = await page.eval("window.__bench.scrollUp(700)");
    const upwardGrowth = await page.eval("window.__bench.growAbove(120)");
    checks.push(
      check("upward-after-jumps.keeps-anchor", Math.abs(upwardGrowth.delta) <= 1.5, {
        anchor: upwardGrowth.id,
        delta: upwardGrowth.delta,
        scrollTop: up.scrollTop,
      }),
    );

    mark("edit");
    const editRow = await page.eval("window.__bench.visibleGroup()");
    const editor = await page.eval(`window.__bench.beginEdit(${JSON.stringify(editRow)})`);
    await page.eval("window.__bench.append(30)");
    const editorAfter = await page.eval("window.__bench.checkEdit()");
    checks.push(
      check(
        "edit-focus-survives-append",
        editorAfter.present === true &&
          editorAfter.focused === true &&
          editorAfter.selectionStart === 4 &&
          editorAfter.selectionEnd === 4,
        { editor, after: editorAfter },
      ),
    );

    await page.eval("window.__bench.scrollTo(0.95)");
    mark("find and accessibility");
    const markerRow = await page.eval("window.__bench.markerRowId()");
    const markerText = await page.eval("window.__bench.marker.text");
    const accessibility = await accessibilityFor(page, markerRow, markerText);
    checks.push(
      check("accessibility.offscreen-row", accessibility.found === true, accessibility),
    );
    const { entries } = await page.eval("window.__bench.snapshot()");
    checks.push(
      check("accessibility.every-message-names-its-actions", accessibility.actionWays >= entries, {
        actionWays: accessibility.actionWays,
        entries,
      }),
    );

    await page.cmd("Page.bringToFront").catch(() => {});
    const find = await page.eval("window.__bench.findMarker()");
    checks.push(
      check(
        "native-find.reaches-offscreen",
        find.found === true &&
          find.selection === find.marker &&
          find.before.bottom < 0,
        {
          found: find.found,
          selection: find.selection,
          offscreenBefore: find.before.bottom < 0,
          beforeTop: find.before.top,
          afterTop: find.after.top,
        },
      ),
    );

    const finalExpected = await page.eval("window.__bench.expected()");
    const final = await page.eval("window.__bench.snapshot()");
    checks.push(...expectedMatches(finalExpected, final, "final"));

    const style = await page.eval("window.__bench.styleInfo()");
    const pageErrors = [...browser.pageErrors];
    checks.push(check("no-page-errors", pageErrors.length === 0, pageErrors.slice(0, 8)));

    return {
      messages: size,
      expected,
      mount,
      append,
      prepend,
      growth,
      mediaGrowth,
      resize: resized,
      scroll,
      sweep,
      wheel,
      picker: { open: pickerOpen, picked },
      touch: { first: touch, afterMouse: touchAfterMouse },
      phone,
      toolbar,
      keyboard,
      jumps,
      reply,
      upwardGrowth,
      editor: { before: editor, after: editorAfter },
      accessibility,
      find,
      final,
      style,
      pageErrors,
      scenarios: progress.marks,
      checks,
    };
  } finally {
    try {
      await page.cmd("Page.close");
    } catch {
      // The browser is closed after every size anyway.
    }
  }
}

function summarize(run) {
  const ms = (value) => `${Math.round(value)} ms`;
  const row = (label, value) => `  ${label.padEnd(28)} ${value}`;
  console.log(`\n${run.messages.toLocaleString("en-US")} messages`);
  console.log(row("mount (producer + render)", `${ms(run.mount.totalMs)} (${ms(run.mount.producerMs)} producer, ${ms(run.mount.commitMs)} commit)`));
  console.log(row("append 200", `${ms(run.append.totalMs)} (${ms(run.append.producerMs)} producer, ${ms(run.append.commitMs)} commit)`));
  console.log(row("prepend 200", `${ms(run.prepend.totalMs)} (${ms(run.prepend.producerMs)} producer, ${ms(run.prepend.commitMs)} commit)`));
  console.log(row("prepend anchor delta", `${run.prepend.delta.toFixed(2)} px`));
  console.log(row("grow above anchor delta", `${run.growth.delta.toFixed(2)} px`));
  console.log(row("media grow anchor delta", run.mediaGrowth.skipped === true ? "skipped" : `${run.mediaGrowth.delta.toFixed(2)} px`));
  console.log(row("scroll p50 / p95 / max", `${run.scroll.p50Ms.toFixed(1)} / ${run.scroll.p95Ms.toFixed(1)} / ${run.scroll.maxMs.toFixed(1)} ms`));
  console.log(row("scroll frames over 50 ms", `${run.scroll.over50ms} of ${run.scroll.steps}`));
  const counts = run.mount.document;
  console.log(row("document after mount", `${counts.nodes.toLocaleString("en-US")} nodes, ${counts.elements.toLocaleString("en-US")} elements, ${counts.toolbars.toLocaleString("en-US")} toolbars (${counts.toolbarElements.toLocaleString("en-US")} elements)`));
  const work = (item) =>
    `${ms(item.taskMs)} main thread, ${item.layouts} layouts ${ms(item.layoutMs)}, ${item.styles} styles ${ms(item.styleMs)}, ${item.frames.over33ms} frames > 33 ms`;
  for (const [name, item] of Object.entries(run.sweep)) {
    console.log(row(`pointer sweep ${name}`, `${work(item)} (${item.moves} moves)`));
  }
  for (const [name, item] of Object.entries(run.wheel)) {
    console.log(row(`wheel ${name}`, `${work(item)} (${item.scrolled} px)`));
  }
  const opens = (item) => (item.works ? "toolbar shown, reply reached" : "toolbar not shown");
  console.log(row("tap, touch only", opens(run.touch.first)));
  console.log(row("tap, mouse resting elsewhere", opens(run.touch.afterMouse)));
  console.log(row("tap at the phone width", run.phone.works ? "disclosure shown, sheet opened and closed" : "disclosure or sheet failed"));
  console.log(row("DOM rows / groups / entries", `${run.final.rows.toLocaleString("en-US")} / ${run.final.groups.toLocaleString("en-US")} / ${run.final.entries.toLocaleString("en-US")}`));
  console.log(row("scroll height", `${Math.round(run.final.scrollHeight).toLocaleString("en-US")} px`));
  console.log(row("row containment", `${run.style.first?.contain ?? "?"} (far ${run.style.far?.contain ?? "?"})`));
  const failed = run.checks.filter((item) => !item.ok);
  console.log(row("checks", failed.length === 0 ? `${run.checks.length} passed` : `${failed.length} failed of ${run.checks.length}`));
  for (const failure of failed) {
    console.log(`    FAIL ${failure.name}: ${JSON.stringify(failure.detail)}`);
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  console.log("Building the transcript fixture...");
  const buildMs = await buildFixture();
  console.log(`Fixture built in ${Math.round(buildMs)} ms`);

  const { server, url } = await serve();
  const browser = await launch({ width: 1280, height: 900, headless: !options.headed });
  console.log(`Browser ${browser.executable}`);
  const started = new Date();
  const runs = [];
  try {
    for (const size of options.messages) {
      let timer;
      const progress = { marks: [] };
      const timedOut = new Promise((_, reject) => {
        timer = setTimeout(() => {
          const last = progress.marks.at(-1);
          const where = last === undefined ? "mount" : `${last.name}, entered at ${Math.round(last.atMs / 1000)} s`;
          reject(new Error(`${size} messages exceeded ${options.timeout} s, in ${where}`));
        }, options.timeout * 1000);
      });
      try {
        const run = await Promise.race([runSize(browser, url, size, progress), timedOut]);
        runs.push(run);
        summarize(run);
      } finally {
        clearTimeout(timer);
      }
    }
  } finally {
    await browser.close();
    await new Promise((resolveClose) => server.close(resolveClose));
    if (!options.keepDist) await rm(distRoot, { recursive: true, force: true });
  }

  const scaling = [];
  const small = runs.find((run) => run.messages === Math.min(...runs.map((item) => item.messages)));
  const large = runs.find((run) => run.messages === Math.max(...runs.map((item) => item.messages)));
  if (small !== undefined && large !== undefined && small !== large) {
    const groupsScale = large.expected.groups === small.expected.groups * (large.messages / small.messages);
    scaling.push(
      check("scaling.groups-linear", groupsScale, {
        small: small.expected.groups,
        large: large.expected.groups,
        ratio: large.expected.groups / small.expected.groups,
        messagesRatio: large.messages / small.messages,
      }),
    );
    const rowsScale = large.final.rows > small.final.rows;
    scaling.push(check("scaling.rows-grow", rowsScale, { small: small.final.rows, large: large.final.rows }));
  }

  let baselineChecks = [];
  if (options.baseline !== null) {
    try {
      const baseline = JSON.parse(await readFile(options.baseline, "utf8"));
      for (const run of runs) {
        const before = baseline.runs?.find((item) => item.messages === run.messages);
        if (before === undefined) continue;
        for (const metric of ["mount", "append", "prepend"]) {
          const ratio = run[metric].totalMs / Math.max(1, before[metric].totalMs);
          baselineChecks.push(
            check(`baseline.${run.messages}.${metric}`, ratio <= options.maxSlowdown, {
              ratio: Number(ratio.toFixed(2)),
              beforeMs: Math.round(before[metric].totalMs),
              afterMs: Math.round(run[metric].totalMs),
              maxSlowdown: options.maxSlowdown,
            }),
          );
        }
        // What a keyboard user meets is not a timing: it has to be the same
        // stops in the same order as the run it is compared with, each with
        // its ring and the transcript scrolled as far, both ways.
        for (const way of ["span", "spanBack"]) {
          if (before.keyboard?.[way] === undefined) continue;
          baselineChecks.push(
            check(
              `baseline.${run.messages}.keyboard-sequence.${way === "span" ? "tab" : "shift-tab"}`,
              JSON.stringify(run.keyboard[way]) === JSON.stringify(before.keyboard[way]),
              { before: before.keyboard[way], after: run.keyboard[way] },
            ),
          );
        }
        // And the toolbar hangs where it did, at the same size.
        if (before.toolbar?.place !== undefined) {
          baselineChecks.push(
            check(
              `baseline.${run.messages}.toolbar-place`,
              JSON.stringify(run.toolbar.place) === JSON.stringify(before.toolbar.place),
              { before: before.toolbar.place, after: run.toolbar.place },
            ),
          );
        }
        for (const [name, item] of [
          ...Object.entries(run.sweep ?? {}).map(([rate, value]) => [`sweep ${rate}`, [value, before.sweep?.[rate]]]),
          ...Object.entries(run.wheel ?? {}).map(([rate, value]) => [`wheel ${rate}`, [value, before.wheel?.[rate]]]),
        ]) {
          const [after, earlier] = item;
          if (earlier === undefined) continue;
          console.log(
            `  ${`${run.messages} ${name} vs baseline`.padEnd(28)} ${Math.round(earlier.taskMs)} -> ${Math.round(after.taskMs)} ms main thread, ${earlier.layouts} -> ${after.layouts} layouts`,
          );
        }
      }
    } catch (error) {
      console.error(`Could not read --baseline: ${error.message}`);
      baselineChecks.push(check("baseline.readable", false, error.message));
    }
  }

  const checks = [...runs.flatMap((run) => run.checks), ...scaling, ...baselineChecks];
  const failed = checks.filter((item) => !item.ok);
  const result = {
    format: "stellarr-transcript-benchmark/1",
    label: options.label,
    generatedAt: started.toISOString(),
    environment: {
      browser: browser.executable,
      viewport: { width: 1280, height: 900 },
      buildMs,
      node: process.version,
      platform: process.platform,
    },
    options: {
      messages: options.messages,
      check: options.check,
      baseline: options.baseline,
      maxSlowdown: options.maxSlowdown,
    },
    runs,
    scaling,
    checks,
    passed: failed.length === 0,
  };
  await mkdir(resolve(options.out, ".."), { recursive: true });
  await writeFile(options.out, `${JSON.stringify(result, null, 2)}\n`);
  console.log(`\n${failed.length === 0 ? "PASS" : "FAIL"}: ${checks.length} checks; result written to ${options.out}`);
  if (options.check && failed.length > 0) process.exitCode = 1;
}

await main();
