// A small Chromium driver over the DevTools Protocol, used by the transcript
// benchmark. No browser package is installed or downloaded: the benchmark
// drives the Chrome already on the machine. Set STELLARR_CHROMIUM (or CHROME)
// when it is not in a usual place.
import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const CANDIDATES = [
  "/usr/bin/chromium",
  "/usr/bin/chromium-browser",
  "/usr/bin/google-chrome",
  "/usr/bin/google-chrome-stable",
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  "/Applications/Chromium.app/Contents/MacOS/Chromium",
];

/** The browser binary to use, or a message naming how to provide one. */
export function findChromium() {
  for (const candidate of [
    process.env.STELLARR_CHROMIUM,
    process.env.CHROME,
    process.env.CHROME_PATH,
    ...CANDIDATES,
  ]) {
    if (candidate !== undefined && candidate !== "" && existsSync(candidate)) return candidate;
  }
  throw new Error(
    "No Chrome or Chromium found. Install one, or set STELLARR_CHROMIUM to its executable.",
  );
}

/**
 * How long one start of Chrome may take to open its DevTools port, and how
 * many starts are tried. A hosted CI runner can be slow enough to miss a
 * short wait, and a Chrome that fails to come up there one time comes up the
 * next. STELLARR_CHROMIUM_START_SECONDS changes the wait.
 */
const START_SECONDS = Number(process.env.STELLARR_CHROMIUM_START_SECONDS) || 60;
const START_ATTEMPTS = 3;

/**
 * One start of Chrome with a fresh profile, up to the moment it says where
 * its DevTools port is. A start that exits or stays silent is stopped and its
 * profile removed, and the error carries what Chrome printed.
 */
async function start(executable, flags) {
  const profile = mkdtempSync(join(tmpdir(), "stellarr-transcript-bench-"));
  const proc = spawn(executable, [...flags, `--user-data-dir=${profile}`, "about:blank"], {
    stdio: ["ignore", "ignore", "pipe"],
  });
  let log = "";
  proc.stderr.setEncoding("utf8");
  try {
    const wsUrl = await new Promise((resolve, reject) => {
      const timer = setTimeout(
        () => reject(new Error(`Chrome did not open a DevTools port within ${START_SECONDS} s.\n${log}`)),
        START_SECONDS * 1000,
      );
      proc.stderr.on("data", (chunk) => {
        log += chunk;
        const match = /DevTools listening on (ws:\/\/\S+)/.exec(log);
        if (match === null) return;
        clearTimeout(timer);
        resolve(match[1]);
      });
      proc.once("error", (error) => {
        clearTimeout(timer);
        reject(new Error(`Chrome could not be started: ${error.message}\n${log}`));
      });
      proc.once("exit", (code, signal) => {
        clearTimeout(timer);
        reject(new Error(`Chrome exited (${code ?? signal}) before opening a DevTools port.\n${log}`));
      });
    });
    return { proc, profile, wsUrl };
  } catch (error) {
    if (proc.exitCode === null && proc.signalCode === null) {
      const exited = new Promise((resolve) => proc.once("exit", resolve));
      proc.kill("SIGKILL");
      await exited;
    }
    rmSync(profile, { recursive: true, force: true });
    throw error;
  }
}

/**
 * Starts a headless browser and returns a browser handle. Each page gets its
 * own browser context so two runs cannot share React state.
 *
 * Only the start is tried again: once Chrome has opened its port, whatever it
 * does wrong fails the run.
 */
export async function launch({ width = 1280, height = 900, headless = true, flags: extraFlags = [] } = {}) {
  const executable = findChromium();
  const flags = [
    ...(headless ? ["--headless=new"] : []),
    "--remote-debugging-port=0",
    "--remote-debugging-address=127.0.0.1",
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-extensions",
    "--disable-background-timer-throttling",
    "--disable-backgrounding-occluded-windows",
    "--disable-renderer-backgrounding",
    "--disable-dev-shm-usage",
    "--mute-audio",
    `--window-size=${width},${height}`,
    ...extraFlags,
  ];
  const failures = [];
  let started = null;
  for (let attempt = 1; started === null && attempt <= START_ATTEMPTS; attempt += 1) {
    try {
      started = await start(executable, flags);
    } catch (error) {
      failures.push(`Start ${attempt} of ${START_ATTEMPTS}: ${error.message}`);
    }
  }
  if (started === null) throw new Error(`Chrome could not be started.\n\n${failures.join("\n\n")}`);
  const { proc, profile, wsUrl } = started;

  const socket = new WebSocket(wsUrl);
  await new Promise((resolve, reject) => {
    socket.onopen = resolve;
    socket.onerror = () => reject(new Error("Could not open the DevTools socket."));
  });

  let sequence = 0;
  const pending = new Map();
  const listeners = new Set();
  const pageErrors = [];

  socket.onmessage = (event) => {
    const message = JSON.parse(event.data);
    if (message.id !== undefined && pending.has(message.id)) {
      const { resolve, reject } = pending.get(message.id);
      pending.delete(message.id);
      if (message.error !== undefined) {
        reject(new Error(JSON.stringify(message.error)));
      } else {
        resolve(message.result);
      }
      return;
    }
    if (message.method === "Runtime.exceptionThrown") {
      const detail = message.params.exceptionDetails;
      pageErrors.push(detail.exception?.description ?? detail.text);
    }
    if (message.method === "Runtime.consoleAPICalled" && message.params.type === "error") {
      pageErrors.push(
        message.params.args
          .map((arg) => arg.value ?? arg.description ?? arg.type)
          .join(" "),
      );
    }
    for (const listener of listeners) listener(message);
  };

  // A command still waiting when Chrome goes away would otherwise wait for
  // ever, and with it the run that was meant to fail on a timeout.
  socket.onclose = () => {
    for (const { reject } of pending.values()) reject(new Error("The DevTools socket closed."));
    pending.clear();
  };

  const send = (method, params = {}, sessionId) => {
    if (socket.readyState !== WebSocket.OPEN) {
      return Promise.reject(new Error(`The DevTools socket is closed; ${method} was not sent.`));
    }
    const id = ++sequence;
    socket.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
    });
  };

  const browser = {
    executable,
    pageErrors,
    async newPage() {
      const { browserContextId } = await send("Target.createBrowserContext", {
        disposeOnDetach: false,
      });
      const { targetId } = await send("Target.createTarget", {
        url: "about:blank",
        browserContextId,
      });
      const { sessionId } = await send("Target.attachToTarget", { targetId, flatten: true });
      const cmd = (method, params = {}) => send(method, params, sessionId);
      await cmd("Page.enable");
      await cmd("Runtime.enable");
      await cmd("Accessibility.enable").catch(() => {});
      await cmd("Emulation.setDeviceMetricsOverride", {
        width,
        height,
        deviceScaleFactor: 1,
        mobile: false,
      });
      await cmd("Emulation.setLocaleOverride", { locale: "en-US" }).catch(() => {});

      const page = {
        sessionId,
        targetId,
        cmd,
        errors: pageErrors,
        /**
         * Calls `listener(method, params)` for every protocol event of this
         * page, such as `Network.requestWillBeSent` once `Network.enable` has
         * been sent. Returns the function that stops it.
         */
        on(listener) {
          const scoped = (message) => {
            if (message.sessionId === sessionId && message.method !== undefined) {
              listener(message.method, message.params);
            }
          };
          listeners.add(scoped);
          return () => listeners.delete(scoped);
        },
        async goto(url) {
          let timer;
          let listener;
          const loaded = new Promise((resolve, reject) => {
            listener = (message) => {
              if (message.sessionId !== sessionId) return;
              if (message.method !== "Page.loadEventFired") return;
              listeners.delete(listener);
              resolve();
            };
            listeners.add(listener);
            timer = setTimeout(() => reject(new Error(`Navigation timed out: ${url}`)), 60_000);
          });
          try {
            await cmd("Page.navigate", { url });
            await loaded;
          } finally {
            clearTimeout(timer);
            listeners.delete(listener);
          }
        },
        async eval(expression) {
          const result = await cmd("Runtime.evaluate", {
            expression,
            awaitPromise: true,
            returnByValue: true,
            userGesture: true,
          });
          if (result.exceptionDetails !== undefined) {
            const detail =
              result.exceptionDetails.exception?.description ?? result.exceptionDetails.text;
            throw new Error(`page: ${detail}`);
          }
          return result.result.value;
        },
        async waitFor(expression, timeout = 30_000) {
          const deadline = Date.now() + timeout;
          while (Date.now() < deadline) {
            try {
              if (await page.eval(expression)) return;
            } catch {
              // The page may still be loading its module.
            }
            await sleep(100);
          }
          throw new Error(`Timed out waiting for: ${expression}`);
        },
        async resize(nextWidth, nextHeight) {
          await cmd("Emulation.setDeviceMetricsOverride", {
            width: nextWidth,
            height: nextHeight,
            deviceScaleFactor: 1,
            mobile: false,
          });
        },
      };
      return page;
    },
    async close() {
      try {
        await send("Browser.close");
      } catch {
        // The browser may already be gone.
      }
      await sleep(300);
      // Closed from this side too: a socket Chrome never closed kept the run
      // from exiting after it had written its result.
      socket.close();
      try {
        proc.kill("SIGKILL");
      } catch {
        // Already exited.
      }
      // A killed Chrome's helpers may still be writing to the profile for a
      // moment, which fails a single attempt with ENOTEMPTY.
      rmSync(profile, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 });
    },
  };
  return browser;
}

export { sleep };
