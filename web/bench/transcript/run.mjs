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
    "  --timeout <seconds>     per-size scenario timeout (default 240)",
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
    timeout: 240,
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
        options.timeout = Number(value ?? "240");
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
    return {
      found,
      nodeCount: nodes.length,
      elapsedMs: Math.round(elapsedMs),
      names: names.slice(0, 3).map((name) => name.slice(0, 80)),
    };
  } catch (error) {
    return { found: false, reason: error.message };
  }
}

async function runSize(browser, pageUrl, size) {
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

    const append = await page.eval("window.__bench.append(200)");
    const appendExpected = await page.eval("window.__bench.expected()");
    checks.push(...expectedMatches(appendExpected, append, "append"));
    checks.push(
      check("append.follows-end", append.bottomGap <= 2 && append.lastRowVisible === true, {
        bottomGap: append.bottomGap,
        lastRowVisible: append.lastRowVisible,
      }),
    );

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

    const scroll = await page.eval("window.__bench.scrollRun(80, 180)");

    const toolbarTarget = await page.eval("window.__bench.toolbarTarget()");
    await page.cmd("Input.dispatchMouseEvent", {
      type: "mouseMoved",
      x: toolbarTarget.x,
      y: toolbarTarget.y,
    });
    await new Promise((resolveWait) => setTimeout(resolveWait, 300));
    const toolbar = await page.eval("window.__bench.toolbarCheck()");
    checks.push(
      check("toolbar.hit-test", toolbar.opacity === "1" && toolbar.hitToolbar === true, {
        row: toolbarTarget.row,
        toolbar,
      }),
    );
    const pressed = await page.eval('window.__bench.pressToolbarAction("Reply")');
    const callbackInfo = await page.eval("window.__bench.callbackInfo()");
    const messageCount = await page.eval("window.__bench.messageCount()");
    checks.push(
      check(
        "toolbar.latest-action",
        callbackInfo.lastAction?.action === "reply" &&
          callbackInfo.lastAction.at === messageCount,
        { pressed, callbackInfo, messageCount },
      ),
    );
    // Leave no message hovered while the next scenarios scroll: a pointer
    // resting over the list keeps changing which toolbar is open.
    await page.cmd("Input.dispatchMouseEvent", { type: "mouseMoved", x: 2, y: 2 });
    await new Promise((resolveWait) => setTimeout(resolveWait, 200));

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
    const markerRow = await page.eval("window.__bench.markerRowId()");
    const markerText = await page.eval("window.__bench.marker.text");
    const accessibility = await accessibilityFor(page, markerRow, markerText);
    checks.push(
      check("accessibility.offscreen-row", accessibility.found === true, accessibility),
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
      jumps,
      reply,
      upwardGrowth,
      editor: { before: editor, after: editorAfter },
      accessibility,
      find,
      final,
      style,
      pageErrors,
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
      const timedOut = new Promise((_, reject) => {
        timer = setTimeout(
          () => reject(new Error(`${size} messages exceeded ${options.timeout} s`)),
          options.timeout * 1000,
        );
      });
      try {
        const run = await Promise.race([runSize(browser, url, size), timedOut]);
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
