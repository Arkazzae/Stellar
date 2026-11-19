// The suppressor comparison: one noise-suppression engine as the app builds
// it now against an earlier worklet and binary, sample by sample, in Chrome.
// It builds the page in web/bench/voice/suppressor with Vite, opens it in the
// Chrome already installed on the machine and renders each fixture through
// both in an OfflineAudioContext.
//
//   npm run bench:voice-suppressor -w stellarr-web -- --engine speex --before <git revision>
//   npm run bench:voice-suppressor -w stellarr-web -- --engine rnnoise \
//     --before-worklet old/rnnoise-worklet.js --before-wasm old/rnnoise_simd.wasm
//
// With a revision, the earlier files are the ones committed under artifacts/
// at that revision for the engine's served paths. A file a recipe built there
// is not in git; give it, and a worklet built from source, by path instead.
// The current side needs what the app needs: run `docker buildx bake recipes`
// and `node ../artifacts/assemble.mjs` first, as `npm test` would.
//
// It also checks two things of the current side: that the "destroy" the app
// sends silences the node, and that a 404 for the binary makes
// attachSuppressor resolve to null.
//
// No browser package is downloaded. STELLARR_CHROMIUM (or CHROME) points at
// the browser when it is not in a usual place; see ../../cdp.mjs.
import { spawnSync } from "node:child_process";
import { createServer } from "node:http";
import { existsSync, mkdtempSync, readFileSync } from "node:fs";
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { extname, join, resolve, sep } from "node:path";

import { build } from "vite";

import { workerBuild } from "../../../worklets.ts";
import { launch } from "../../cdp.mjs";

const here = import.meta.dirname;
const webRoot = resolve(here, "..", "..", "..");
const repositoryRoot = resolve(webRoot, "..");
const publicAudio = resolve(webRoot, "public", "audio");

const ENGINES = ["speex", "rnnoise", "gtcrn"];

/** The fixtures of the suppressor record: steady noise alone and under speech, impulses, silence, overload. */
const DEFAULT_FIXTURES = ["speech", "speech-over-fan", "fan-speech", "fan", "keyboard", "buzz", "silence", "clipping"];

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".wasm": "application/wasm",
};

function usage() {
  return [
    "Usage: npm run bench:voice-suppressor -w stellarr-web -- --engine <name> <earlier files> [options]",
    "",
    `  --engine <name>           ${ENGINES.join(", ")}`,
    "  --before <revision>       the worklet and binary committed under artifacts/ at a git revision",
    "  --before-worklet <path>   the earlier worklet, instead of a revision's",
    "  --before-wasm <path>      the earlier binary, instead of a revision's",
    `  --fixtures <a,b,...>      fixtures of web/bench/voice/fixtures.ts (default ${DEFAULT_FIXTURES.join(",")})`,
    "  --require-identical       fail unless every fixture comes out sample for sample the same",
    "  --out <path>              also write the result as JSON",
    "  --timeout <s>             timeout of the whole run in Chrome, in seconds (default 600)",
    "  --headed                  show the browser (debugging only)",
  ].join("\n");
}

function parseArgs(argv) {
  const options = {
    engine: null,
    before: null,
    beforeWorklet: null,
    beforeWasm: null,
    fixtures: DEFAULT_FIXTURES,
    requireIdentical: false,
    out: null,
    timeout: 600,
    headed: false,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    const equals = argument.indexOf("=");
    const flag = equals === -1 ? argument : argument.slice(0, equals);
    const inline = equals === -1 ? undefined : argument.slice(equals + 1);
    const takeValue = () => {
      if (inline !== undefined) return inline;
      index += 1;
      return argv[index];
    };
    switch (flag) {
      case "--engine":
        options.engine = takeValue();
        break;
      case "--before":
        options.before = takeValue();
        break;
      case "--before-worklet":
        options.beforeWorklet = resolve(process.cwd(), takeValue() ?? "");
        break;
      case "--before-wasm":
        options.beforeWasm = resolve(process.cwd(), takeValue() ?? "");
        break;
      case "--fixtures":
        options.fixtures = (takeValue() ?? "").split(",").filter(Boolean);
        break;
      case "--require-identical":
        options.requireIdentical = true;
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
  if (!ENGINES.includes(options.engine)) throw new Error(`--engine must be one of ${ENGINES.join(", ")}\n\n${usage()}`);
  const byPath = options.beforeWorklet !== null || options.beforeWasm !== null;
  if (options.before === null && !(options.beforeWorklet && options.beforeWasm)) {
    throw new Error(`Give --before <revision>, or both --before-worklet and --before-wasm\n\n${usage()}`);
  }
  if (options.before !== null && byPath) throw new Error("Give either --before or the two paths, not both");
  return options;
}

/** A file as it was at a git revision, as bytes. */
function atRevision(revision, path) {
  const result = spawnSync("git", ["show", `${revision}:${path}`], { cwd: repositoryRoot, maxBuffer: 1 << 28 });
  if (result.status !== 0) throw new Error(`git show ${revision}:${path} failed: ${result.stderr.toString().trim()}`);
  return result.stdout;
}

/**
 * The bytes the app served at `servedPath` at a revision: the committed file
 * the manifest pointed it to.
 */
function servedAtRevision(revision, servedPath) {
  const manifest = JSON.parse(atRevision(revision, "artifacts/manifest.json").toString("utf8")).artifacts;
  const entry = manifest.find((artifact) => artifact.path === servedPath);
  if (!entry) throw new Error(`At ${revision} the manifest serves nothing at /${servedPath}; give the file by path.`);
  if (entry.source.file === undefined) {
    throw new Error(`At ${revision} /${servedPath} was built by the recipe "${entry.source.recipe}"; build it there and give it by path.`);
  }
  return atRevision(revision, `artifacts/${entry.source.file}`);
}

/** The page, with the app's worklets built as the app's build makes them. */
async function buildPage(outDir) {
  await build({
    configFile: false,
    root: here,
    base: "/",
    publicDir: false,
    resolve: { alias: { "@": resolve(webRoot, "src/stellarr") } },
    worker: workerBuild("assets"),
    build: { outDir, emptyOutDir: true, target: "es2022", sourcemap: false, reportCompressedSize: false },
    logLevel: "warn",
  });
}

/**
 * The page and its worklets, web/public/audio/ for the files the app takes
 * from artifacts/, the earlier files at /before/, and /__missing, through
 * which the page makes one path answer 404.
 */
function serve(distRoot, earlier) {
  let missing = null;
  const state = { lastBinary: null };
  const server = createServer(async (request, response) => {
    try {
      const { pathname } = new URL(request.url ?? "/", "http://127.0.0.1");
      if (pathname === "/__missing") {
        missing = request.method === "POST" ? await new Promise((done) => {
          let body = "";
          request.on("data", (chunk) => (body += chunk));
          request.on("end", () => done(body));
        }) : null;
        response.writeHead(204).end();
        return;
      }
      if (pathname === missing) throw new Error("made missing");
      let body;
      let type;
      if (pathname === "/before/worklet.js") {
        body = earlier.worklet();
        type = MIME[".js"];
      } else if (pathname === "/before/binary.wasm") {
        body = earlier.wasm(state.lastBinary);
        type = MIME[".wasm"];
      } else {
        const relative = pathname === "/" ? "/index.html" : decodeURIComponent(pathname);
        let file = resolve(distRoot, `.${relative}`);
        if (!file.startsWith(distRoot + sep)) throw new Error("outside the build");
        if (!existsSync(file) && relative.startsWith("/audio/")) file = resolve(publicAudio, `.${relative.slice(6)}`);
        if (relative.startsWith("/audio/") && relative.endsWith(".wasm")) state.lastBinary = relative.slice(1);
        body = await readFile(file);
        type = MIME[extname(file)] ?? "application/octet-stream";
      }
      response.writeHead(200, { "content-type": type, "content-length": body.byteLength, "cache-control": "no-store" });
      response.end(body);
    } catch {
      response.writeHead(404).end();
    }
  });
  return new Promise((resolveListen, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () =>
      resolveListen({ server, state, url: `http://127.0.0.1:${server.address().port}/` }),
    );
  });
}

function withTimeout(promise, seconds, what) {
  let timer;
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${what} took longer than ${seconds} s`)), seconds * 1000);
  });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

// JSON from the page carries -Infinity, the level of silence, as null.
const figure = (value, digits = 1) => (typeof value === "number" && Number.isFinite(value) ? value.toFixed(digits) : "-inf");

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const earlier = {
    worklet: () =>
      options.before === null
        ? readFileSync(options.beforeWorklet)
        : servedAtRevision(options.before, `audio/${options.engine}-worklet.js`),
    // The binary the app fetched for the current side, as it was then.
    wasm: (servedPath) => {
      if (options.before === null) return readFileSync(options.beforeWasm);
      if (servedPath === null) throw new Error("the page asked for the earlier binary before the current one");
      return servedAtRevision(options.before, servedPath);
    },
  };

  const distRoot = mkdtempSync(join(tmpdir(), "stellarr-voice-suppressor-"));
  console.log("Building the comparison page...");
  await buildPage(distRoot);
  const { server, state, url } = await serve(distRoot, earlier);
  const problems = [];
  let result = null;
  let browser = null;
  try {
    browser = await launch({ width: 800, height: 600, headless: !options.headed });
    const page = await browser.newPage();
    await page.goto(url);
    await page.waitFor("window.__suppressor?.ready === true", 60_000);
    const chrome = await page.eval("navigator.userAgent");
    const work = (async () => ({
      comparison: await page.eval(
        `window.__suppressor.compare(${JSON.stringify(options.engine)}, ${JSON.stringify(options.fixtures)})`,
      ),
      destroyed: await page.eval(`window.__suppressor.destroyed(${JSON.stringify(options.engine)})`),
      // The binary the app fetched for this engine, which may be a SIMD build.
      missingBinaryGivesNull: await page.eval(
        `window.__suppressor.missingBinary(${JSON.stringify(options.engine)}, ${JSON.stringify(`/${state.lastBinary}`)})`,
      ),
    }))();
    result = { engine: options.engine, before: options.before ?? { worklet: options.beforeWorklet, wasm: options.beforeWasm }, chrome, ...(await withTimeout(work, options.timeout, "the comparison")) };
    problems.push(...browser.pageErrors.map((error) => `page: ${error}`));
  } finally {
    if (browser) await browser.close().catch(() => {});
    await new Promise((done) => server.close(done));
    await rm(distRoot, { recursive: true, force: true });
  }

  const { latencies, results } = result.comparison;
  console.log(`\n${options.engine} in ${result.chrome}`);
  console.log(`latency: before ${latencies.before ?? "none"} samples, now ${latencies.after ?? "none"} samples`);
  console.log("fixture            differing   largest      difference         output (before / now)   first sound");
  for (const row of results) {
    const identical = row.differing === 0;
    console.log(
      [
        row.fixture.padEnd(18),
        `${row.differing}/${row.samples}`.padEnd(11),
        (identical ? "0" : row.largest.toExponential(2)).padEnd(12),
        (identical ? "identical" : `${figure(row.differenceDbfs)} dBFS, ${figure(row.differenceDb)} dB`).padEnd(18),
        `${figure(row.outputDbfs.before)} / ${figure(row.outputDbfs.after)} dBFS`.padEnd(23),
        `${row.firstSound.before} / ${row.firstSound.after}`,
      ].join(" "),
    );
    if (options.requireIdentical && !identical) problems.push(`${row.fixture}: ${row.differing} samples differ`);
    if (row.firstSound.before > row.firstSound.after) {
      problems.push(`${row.fixture}: the earlier engine sounded only from sample ${row.firstSound.before}; it had not loaded`);
    }
  }
  const { before, after } = result.destroyed;
  console.log(
    `destroy: before ${before.silentAfter ? "falls silent" : "keeps sounding"}, now ${after.silentAfter ? "falls silent" : "keeps sounding"}`,
  );
  if (!after.soundBefore || !after.silentAfter) problems.push("the current node did not fall silent after destroy");
  console.log(`a 404 for /${state.lastBinary}: attachSuppressor ${result.missingBinaryGivesNull ? "gives null" : "does not give null"}`);
  if (!result.missingBinaryGivesNull) problems.push("attachSuppressor did not give null for a missing binary");

  if (options.out) {
    await mkdir(resolve(options.out, ".."), { recursive: true });
    await writeFile(options.out, `${JSON.stringify({ ...result, problems, passed: problems.length === 0 }, null, 2)}\n`);
  }
  for (const problem of problems) console.log(`FAIL ${problem}`);
  console.log(problems.length === 0 ? "PASS" : `FAIL: ${problems.length} problem(s)`);
  if (problems.length > 0) process.exitCode = 1;
}

await main();
