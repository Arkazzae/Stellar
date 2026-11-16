// The playback record. It builds the page in web/bench/voice/playback with
// Vite, opens it in the Chrome already installed on the machine, renders the
// app's own playback strips in an OfflineAudioContext, and checks what comes
// out against web/bench/voice/records/playback.txt.
//
//   npm run bench:voice-playback -w stellarr-web -- --check
//   npm run bench:voice-playback -w stellarr-web -- --record --repeat 5
//
// No browser package is downloaded. STELLARR_CHROMIUM (or CHROME) points at
// the browser when it is not in a usual place; see ../../cdp.mjs.
import { createServer } from "node:http";
import { existsSync, mkdtempSync, readdirSync } from "node:fs";
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, extname, join, resolve, sep } from "node:path";

import { build } from "vite";

import { workerBuild } from "../../../worklets.ts";
import { launch } from "../../cdp.mjs";

const here = import.meta.dirname;
const voiceRoot = resolve(here, "..");
const webRoot = resolve(voiceRoot, "..", "..");
const recordFile = resolve(voiceRoot, "records", "playback.txt");
const speechRecord = resolve(voiceRoot, "records", "capture", "speech.txt");
const recordingsDir = resolve(voiceRoot, "recordings");

const PREAMBLE = [
  "Playback record: what each receiver strip does to a signal, measured in Chrome's OfflineAudioContext.",
  "See web/bench/voice/README.md for the notation. Rewritten only by npm run voice:record,",
  "in a commit that names the audible change it intends.",
];

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".mp3": "audio/mpeg",
  ".wav": "audio/wav",
};

function usage() {
  return [
    "Usage: npm run bench:voice-playback -w stellarr-web -- [options]",
    "",
    "  --check            compare with records/playback.txt (default)",
    "  --record           write records/playback.txt from fresh runs",
    "  --repeat <n>       separate browser runs (default 1, or 3 with --record)",
    "  --out <path>       also write a JSON result here",
    "  --timeout <s>      per-run timeout in seconds (default 600)",
    "  --headed           show the browser (debugging only)",
  ].join("\n");
}

function parseArgs(argv) {
  const options = { mode: "check", repeat: null, out: null, timeout: 600, headed: false };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    const equals = argument.indexOf("=");
    const flag = equals === -1 ? argument : argument.slice(0, equals);
    const inline = equals === -1 ? undefined : argument.slice(equals + 1);
    const value = inline ?? argv[index + 1];
    const takeValue = () => {
      if (inline === undefined) index += 1;
      return value;
    };
    switch (flag) {
      case "--check":
        options.mode = "check";
        break;
      case "--record":
        options.mode = "record";
        break;
      case "--repeat":
        options.repeat = Number(takeValue());
        break;
      case "--out":
        options.out = resolve(process.cwd(), takeValue() ?? "");
        break;
      case "--timeout":
        options.timeout = Number(takeValue());
        break;
      case "--headed":
        options.headed = true;
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
  options.repeat ??= options.mode === "record" ? 3 : 1;
  if (!Number.isInteger(options.repeat) || options.repeat < 1) throw new Error("--repeat needs a whole number");
  return options;
}

/**
 * The page, with the detector worklet built as the app's build makes it:
 * the same worker options and the same minifier, so Chrome runs the bundle
 * that ships at /audio/detector-worklet.js.
 */
async function buildPage(outDir) {
  await build({
    configFile: false,
    root: here,
    base: "/",
    publicDir: false,
    resolve: { alias: { "@": resolve(webRoot, "src/stellarr") } },
    worker: workerBuild("assets"),
    build: {
      outDir,
      emptyOutDir: true,
      target: "es2022",
      sourcemap: false,
      reportCompressedSize: false,
    },
    logLevel: "warn",
  });
}

function recordingNames() {
  if (!existsSync(recordingsDir)) return [];
  return readdirSync(recordingsDir)
    .filter((name) => name.toLowerCase().endsWith(".wav"))
    .sort();
}

/** The page with its worklet, and the app's sample, the way nginx serves them. */
function serve(distRoot) {
  const fixed = {
    "/sounds/voice-playback-test.mp3": resolve(webRoot, "public", "sounds", "voice-playback-test.mp3"),
  };
  const server = createServer(async (request, response) => {
    try {
      const { pathname } = new URL(request.url ?? "/", "http://127.0.0.1");
      let file;
      if (fixed[pathname]) file = fixed[pathname];
      else if (pathname.startsWith("/recordings/")) {
        const name = decodeURIComponent(pathname.slice("/recordings/".length));
        if (name !== basename(name) || !name.toLowerCase().endsWith(".wav")) throw new Error("not a recording");
        file = resolve(recordingsDir, name);
      } else {
        const relative = pathname === "/" ? "/index.html" : decodeURIComponent(pathname);
        file = resolve(distRoot, `.${relative}`);
        if (!file.startsWith(distRoot + sep)) throw new Error("outside the build");
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
      resolveListen({ server, url: `http://127.0.0.1:${server.address().port}/` });
    });
  });
}

function withTimeout(promise, seconds, what) {
  let timer;
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${what} took longer than ${seconds} s`)), seconds * 1000);
  });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

async function openPage(url, headed) {
  const browser = await launch({ width: 800, height: 600, headless: !headed });
  const page = await browser.newPage();
  await page.goto(url);
  await page.waitFor("window.__voice?.ready === true", 60_000);
  // The user agent string is reduced to 155.0.0.0; the full version is behind
  // the client hints.
  const chrome = await page.eval(`(async () => {
    const brands = (await navigator.userAgentData?.getHighEntropyValues(["fullVersionList"]))?.fullVersionList ?? [];
    const brand = brands.find((item) => /Chrom/.test(item.brand));
    return brand ? brand.brand + " " + brand.version : navigator.userAgent;
  })()`);
  return { browser, page, chrome };
}

/** cdp.mjs removes the profile right after killing Chrome, which can race its last writes. */
async function closeQuietly(browser) {
  try {
    await browser.close();
  } catch {
    // The profile directory is in the temporary folder; a leftover is harmless.
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const distRoot = mkdtempSync(join(tmpdir(), "stellarr-voice-playback-"));
  console.log("Building the playback page...");
  await buildPage(distRoot);
  const { server, url } = await serve(distRoot);
  const texts = [];
  const problems = [];
  const recordings = [];
  let chrome = "";
  let crossCheck = [];
  let last = null;
  try {
    for (let run = 1; run <= options.repeat; run += 1) {
      if (last) await closeQuietly(last.browser);
      last = await openPage(url, options.headed);
      chrome = last.chrome;
      const started = performance.now();
      const text = await withTimeout(last.page.eval("window.__voice.record()"), options.timeout, `run ${run}`);
      texts.push(text);
      console.log(`Run ${run} of ${options.repeat} in Chrome ${chrome}: ${Math.round(performance.now() - started)} ms`);
      if (last.browser.pageErrors.length > 0) problems.push(...last.browser.pageErrors.map((error) => `page: ${error}`));
    }
    const page = last.page;

    // The Node driver of the capture record against Chrome's own AudioWorklet.
    const speech = await readFile(speechRecord, "utf8");
    const checked = await page.eval(`window.__voice.crossCheck(${JSON.stringify(speech)})`);
    crossCheck = checked.problems;
    console.log(`Capture node in Chrome's AudioWorklet, fixture speech: ${checked.agreement}`);
    for (const problem of crossCheck) problems.push(`capture in Chrome vs the Node record: ${problem}`);

    const spread = await page.eval(`window.__voice.spread(${JSON.stringify(texts)})`);
    console.log(`Spread over ${texts.length} run(s): ${spread}`);

    if (options.mode === "record") {
      const record = await page.eval(
        `window.__voice.finish(${JSON.stringify(texts)}, ${JSON.stringify(chrome)}, ${JSON.stringify(PREAMBLE)})`,
      );
      await mkdir(resolve(recordFile, ".."), { recursive: true });
      await writeFile(recordFile, record);
      console.log(`Recorded ${recordFile}`);
    } else {
      const recorded = await readFile(recordFile, "utf8").catch(() => null);
      if (recorded === null) problems.push("no playback record; run npm run voice:record");
      else {
        const recordedChrome = /^chrome (.*)$/m.exec(recorded)?.[1];
        if (recordedChrome && recordedChrome !== chrome) {
          console.log(`Recorded with Chrome ${recordedChrome}, checking with Chrome ${chrome}.`);
        }
        for (const [index, text] of texts.entries()) {
          const found = await page.eval(
            `window.__voice.compare(${JSON.stringify(recorded)}, ${JSON.stringify(text)})`,
          );
          for (const problem of found) problems.push(`run ${index + 1}: ${problem}`);
        }
      }
    }

    // Recordings in the ignored folder: their playback record stays beside them.
    for (const name of recordingNames()) {
      const text = await withTimeout(
        page.eval(`window.__voice.recording(${JSON.stringify(`/recordings/${encodeURIComponent(name)}`)}, ${JSON.stringify(name)})`),
        options.timeout,
        name,
      );
      const file = resolve(recordingsDir, name.replace(/\.wav$/i, ".playback.txt"));
      const before = await readFile(file, "utf8").catch(() => null);
      if (before === null || options.mode === "record") {
        await writeFile(file, text);
        recordings.push({ name, status: "recorded" });
        console.log(`${name}: recorded beside it, not checked`);
      } else {
        const found = await page.eval(`window.__voice.compare(${JSON.stringify(before)}, ${JSON.stringify(text)})`);
        recordings.push({ name, status: found.length === 0 ? "matches" : "differs", problems: found });
        for (const problem of found) problems.push(`${name}: ${problem}`);
        console.log(`${name}: ${found.length === 0 ? "matches its record" : `${found.length} differences`}`);
      }
    }
  } finally {
    if (last) await closeQuietly(last.browser);
    await new Promise((resolveClose) => server.close(resolveClose));
    await rm(distRoot, { recursive: true, force: true });
  }

  if (options.out) {
    await mkdir(resolve(options.out, ".."), { recursive: true });
    await writeFile(
      options.out,
      `${JSON.stringify({ format: "stellarr-voice-playback/1", mode: options.mode, chrome, runs: texts.length, crossCheck, recordings, problems, passed: problems.length === 0 }, null, 2)}\n`,
    );
  }
  for (const problem of problems.slice(0, 40)) console.log(`FAIL ${problem}`);
  if (problems.length > 40) console.log(`... and ${problems.length - 40} more`);
  console.log(problems.length === 0 ? "PASS" : `FAIL: ${problems.length} problem(s)`);
  if (problems.length > 0) process.exitCode = 1;
}

await main();
