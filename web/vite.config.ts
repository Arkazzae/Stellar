import { fileURLToPath } from "node:url";

import react from "@vitejs/plugin-react";
import { defineConfig, loadEnv, type ProxyOptions } from "vite";

import { previewMiddleware } from "../sdk/dev/instance.js";
import { stellarrArtifactSizes } from "./artifactSizes";
import { compileLocales, watchLocales } from "./localization";
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
 * production. The dev and preview servers forward requests to `apiOrigin`.
 * /hubs/ preserves the browser's Host for the WebSocket origin check.
 * The desktop handshake at /.well-known/stellarr/client uses X-Forwarded-Host
 * to report the instance's origin. Uploads stream through to the API.
 */
function apiProxy(apiOrigin: string): Record<string, ProxyOptions> {
  const forward = (ws: boolean): ProxyOptions => ({
    target: apiOrigin,
    changeOrigin: !ws,
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
compileLocales();
const [{ BUILD_DIRECTORY, COCKPIT_DOCUMENT, stellarrDocuments, WEB_DOCUMENT }, { bundleReport }, { stellarrRelease }] =
  await Promise.all([import("./documents"), import("./bundleReport"), import("./release")]);

export default defineConfig(({ mode, command, isPreview }) => {
  const env = loadEnv(mode, here("."), "");
  const proxy = apiProxy(env.STELLARR_API_ORIGIN || "http://127.0.0.1:5000");
  const previewEnabled = command === "serve" && !isPreview && env.STELLARR_ACTIVITY_DEV_ENABLED === "true";
  const allowedHosts = commaSeparated(env.STELLARR_DEV_ALLOWED_HOSTS);

  return {
    appType: "mpa",
    define: { __ACTIVITY_PREVIEW__: JSON.stringify(previewEnabled) },
    plugins: [
      react(),
      watchLocales(),
      {
        name: "stellarr-activity-preview",
        apply: "serve",
        configureServer(server) {
          if (!previewEnabled) return;
          const preview = previewMiddleware({
            origin: env.STELLARR_ACTIVITY_DEV_ORIGIN || "http://localhost:3000",
            domain: env.STELLARR_ACTIVITY_DEV_DOMAIN || "activity.localhost",
            apiOrigin: env.STELLARR_API_ORIGIN || "http://127.0.0.1:5000",
          });
          server.middlewares.use(preview.middleware);
          server.httpServer?.once("close", preview.close);
        },
      },
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
      // Realtime and voice arrive with the signed-in app. The web document's
      // initial download budget is enforced separately in bundleReport.ts.
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
