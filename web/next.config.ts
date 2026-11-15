import path from "node:path";

import type { NextConfig } from "next";
import { PHASE_PRODUCTION_BUILD } from "next/constants";

const apiOrigin = process.env.STELLARR_API_ORIGIN ?? "http://127.0.0.1:5000";

function commaSeparated(value: string | undefined): string[] {
  return (value ?? "")
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}

/**
 * Next 16 rejects its own dev chunks when the page is opened from a host other
 * than `localhost`, which leaves only the server-rendered skeleton. Loopback
 * addresses must keep working, and IDE or remote previews are added by name.
 */
const allowedDevOrigins = [
  ...new Set(["127.0.0.1", ...commaSeparated(process.env.STELLARR_ALLOWED_DEV_ORIGINS)]),
];

/**
 * `web/` is a workspace of the repository root, which holds the hoisted
 * `node_modules` and the shared `packages/`, so Turbopack must start there.
 */
const workspaceRoot = path.resolve(import.meta.dirname, "..");

/**
 * `next build` writes a static export to `out/`, which Nginx serves in the
 * container: there is no Node server in production. Nginx also does what this
 * file did for the server it replaced, so its configuration
 * (`infra/nginx.conf`) holds the legacy redirects, the cache policy and the
 * compression.
 *
 * `next dev` stays a server, without export mode. It proxies the API, which
 * an export rejects, and renders any invite code, where export mode would
 * accept only the one the build renders.
 */
export default function config(phase: string): NextConfig {
  const exporting = phase === PHASE_PRODUCTION_BUILD;

  return {
    reactStrictMode: true,
    poweredByHeader: false,
    turbopack: { root: workspaceRoot },
    // @stellarr/ui is consumed as TypeScript source, with no build step.
    transpilePackages: ["@stellarr/ui"],
    allowedDevOrigins,
    ...(exporting
      ? { output: "export" }
      : {
          experimental: {
            // Dev rewrites buffer uploads in the Next server. 128 MB is the
            // transport ceiling of Kestrel and Nginx, so a video within the
            // API's own 100 MB media limit still arrives as one complete
            // multipart request.
            proxyClientMaxBodySize: "128mb",
          },
          /**
           * `next dev` serves the UI on :3000 and the API runs on :5000, so
           * the same-origin paths the client uses in production are proxied
           * here, as Nginx proxies them in the container.
           */
          async rewrites() {
            return [
              {
                source: "/.well-known/stellarr/client",
                destination: `${apiOrigin}/.well-known/stellarr/client`,
              },
              { source: "/api/:path*", destination: `${apiOrigin}/api/:path*` },
              { source: "/assets/:path*", destination: `${apiOrigin}/assets/:path*` },
              { source: "/hubs/:path*", destination: `${apiOrigin}/hubs/:path*` },
            ];
          },
        }),
  };
}
