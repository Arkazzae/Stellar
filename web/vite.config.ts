import { fileURLToPath } from "node:url";

import react from "@vitejs/plugin-react";
import { defineConfig, loadEnv, type ProxyOptions } from "vite";

import { stellarrArtifactSizes } from "./artifactSizes";
import { bundleReport } from "./bundleReport";
import { BUILD_DIRECTORY, COCKPIT_DOCUMENT, stellarrDocuments, WEB_DOCUMENT } from "./documents";
import { stellarrRelease } from "./release";
import { workerBuild } from "./worklets";

const here = (path: string) => fileURLToPath(new URL(path, import.meta.url));

function commaSeparated(value: string | undefined): string[] {
  return (value ?? "")
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}

/**
 * The same-origin paths the client calls on the API, which Nginx proxies in
 * production. The dev and preview servers proxy them to `apiOrigin`, as the
 * API's own host, and say which host the browser asked in X-Forwarded-Host:
 * the desktop handshake at /.well-known/stellarr/client reports the instance's
 * origin from it. /hubs/ carries SignalR's WebSocket, without which SignalR
 * falls back to slower transports. Requests stream through; nothing is
 * buffered, so an upload has no limit here beyond the API's own.
 */
function apiProxy(apiOrigin: string): Record<string, ProxyOptions> {
  const forward = (ws: boolean): ProxyOptions => ({
    target: apiOrigin,
    changeOrigin: true,
    ws,
    configure(proxy) {
      proxy.on("proxyReq", (request, incoming) => request.setHeader("X-Forwarded-Host", incoming.headers.host ?? ""));
      proxy.on("proxyReqWs", (request, incoming) => request.setHeader("X-Forwarded-Host", incoming.headers.host ?? ""));
    },
  });
  return {
    "^/api(/|$)": forward(false),
    "^/assets(/|$)": forward(false),
    "^/health(/|$)": forward(false),
    "^/hubs(/|$)": forward(true),
    "^/\\.well-known/stellarr/client(\\?|$)": forward(false),
  };
}

/**
 * The web client and Cockpit, two documents built together: `index.html`
 * at / and `cockpit/index.html` at /cockpit. `npm run dev` serves both on
 * :3000 with the API proxied, `npm run build` writes `dist/`, and
 * `npm run preview` serves `dist/` the same way on :4173.
 *
 * The scripts load this file with `--configLoader runner`, which resolves
 * imports as Vite does: `documents.ts` reads the pre-paint scripts from the
 * app's own modules, whose imports name no file extensions.
 *
 * The settings come from the environment or `web/.env`:
 *
 *   STELLARR_API_ORIGIN          where the API listens, http://127.0.0.1:5000 by default
 *   STELLARR_LANGUAGE            the instance default, as /stellarr-instance.js
 *   STELLARR_DEV_ALLOWED_HOSTS   further host names the servers answer, comma-separated,
 *                                for an IDE or remote preview; localhost and IP
 *                                addresses always work
 *   STELLARR_DEV_POLLING         "true" to poll for changes, where file events do not
 *                                cross a bind mount
 *   VITE_ICE_SERVERS             written into the web client's bundle
 */
export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, here("."), "");
  const proxy = apiProxy(env.STELLARR_API_ORIGIN || "http://127.0.0.1:5000");
  const allowedHosts = commaSeparated(env.STELLARR_DEV_ALLOWED_HOSTS);

  return {
    appType: "mpa",
    plugins: [
      react(),
      stellarrDocuments({ instanceLanguage: env.STELLARR_LANGUAGE, publicDir: here("public") }),
      stellarrArtifactSizes(),
      bundleReport(),
      stellarrRelease({ publicDir: here("public") }),
    ],
    resolve: {
      alias: { "@": here("src/stellarr") },
    },
    // Only the two documents lead to the apps' code; the transcript benchmark
    // has a page of its own and builds it itself.
    optimizeDeps: {
      entries: [WEB_DOCUMENT, COCKPIT_DOCUMENT],
    },
    // The speech detector's and Speex's AudioWorklets, each bundled apart and
    // written to its own address under /audio/; see worklets.ts.
    worker: workerBuild(BUILD_DIRECTORY),
    build: {
      outDir: "dist",
      emptyOutDir: true,
      assetsDir: BUILD_DIRECTORY,
      // The largest chunks are the signed-in app, about 490 kB, and the web
      // client's entry and the chunk it shares with the app, about 320 kB
      // each: the providers every page needs and React. SignalR and voice are
      // a chunk of their own, about 140 kB, which arrives with the app, and
      // each message catalog is one too, of which a page loads its language's.
      // What the web document loads up front is held to a budget in
      // bundleReport.ts, which this limit does not replace.
      chunkSizeWarningLimit: 500,
      rolldownOptions: {
        input: {
          web: here(WEB_DOCUMENT),
          cockpit: here(COCKPIT_DOCUMENT),
        },
      },
    },
    server: {
      port: 3000,
      strictPort: true,
      allowedHosts,
      proxy,
      watch: env.STELLARR_DEV_POLLING === "true" ? { usePolling: true, interval: 300 } : {},
    },
    preview: {
      port: 4173,
      strictPort: true,
      allowedHosts,
      proxy,
    },
  };
});
