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
 * Starts a headless browser and returns a browser handle. Each page gets its
 * own browser context so two runs cannot share React state.
 */
export async function launch({ width = 1280, height = 900, headless = true } = {}) {
  const executable = findChromium();
  const profile = mkdtempSync(join(tmpdir(), "stellarr-transcript-bench-"));
  const proc = spawn(
    executable,
    [
      ...(headless ? ["--headless=new"] : []),
      "--remote-debugging-port=0",
      "--remote-debugging-address=127.0.0.1",
      `--user-data-dir=${profile}`,
      "--no-first-run",
      "--no-default-browser-check",
      "--disable-extensions",
      "--disable-background-timer-throttling",
      "--disable-backgrounding-occluded-windows",
      "--disable-renderer-backgrounding",
      "--disable-dev-shm-usage",
      "--mute-audio",
      `--window-size=${width},${height}`,
      "about:blank",
    ],
    { stdio: ["ignore", "ignore", "pipe"] },
  );

  let log = "";
  proc.stderr.setEncoding("utf8");
  const wsUrl = await new Promise((resolve, reject) => {
    const timer = setTimeout(
      () => reject(new Error(`Chrome did not open a DevTools port.\n${log}`)),
      20_000,
    );
    proc.stderr.on("data", (chunk) => {
      log += chunk;
      const match = /DevTools listening on (ws:\/\/\S+)/.exec(log);
      if (match === null) return;
      clearTimeout(timer);
      resolve(match[1]);
    });
    proc.once("exit", (code) => {
      clearTimeout(timer);
      reject(new Error(`Chrome exited (${code}).\n${log}`));
    });
  });

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

  const send = (method, params = {}, sessionId) => {
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
      try {
        proc.kill("SIGKILL");
      } catch {
        // Already exited.
      }
      rmSync(profile, { recursive: true, force: true });
    },
  };
  return browser;
}

export { sleep };
