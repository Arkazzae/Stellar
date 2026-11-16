import path from "node:path";

import type { NextConfig } from "next";

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
 * `node_modules` and the shared `packages/`. File tracing and Turbopack must
 * start there, or the standalone output misses the hoisted dependencies and the
 * shared package. The output then nests the app as `web/server.js`.
 */
const workspaceRoot = path.resolve(import.meta.dirname, "..");

const IMMUTABLE = "public, max-age=31536000, immutable";
/** The browser may keep a copy, but asks whether it is still current before each use. */
const REVALIDATE = "no-cache";

function cacheControl(source: string, value: string) {
  return { source, headers: [{ key: "Cache-Control", value }] };
}

const nextConfig: NextConfig = {
  reactStrictMode: true,
  poweredByHeader: false,
  output: "standalone",
  outputFileTracingRoot: workspaceRoot,
  turbopack: { root: workspaceRoot },
  // @stellarr/ui is consumed as TypeScript source, with no build step.
  transpilePackages: ["@stellarr/ui"],
  allowedDevOrigins,
  experimental: {
    // Dev rewrites buffer uploads in the Next server. 128 MB is the transport
    // ceiling of Kestrel and Nginx, so a video within the API's own 100 MB
    // media limit still arrives as one complete multipart request.
    proxyClientMaxBodySize: "128mb",
    // The web client and Cockpit each have a root layout, so an address no
    // route claims is answered by src/app/global-not-found.tsx, a whole
    // document of its own. A catch-all route that called notFound() instead
    // would send an empty error shell and leave the page to the browser.
    globalNotFound: true,
  },
  /**
   * `next dev` serves the UI on :3000 and the API runs on :5000, so the
   * same-origin paths the client uses in production are proxied here. Nginx
   * does that in production, where this returns nothing.
   */
  async rewrites() {
    if (process.env.NODE_ENV === "production") return [];
    return [
      {
        source: "/.well-known/stellarr/client",
        destination: `${apiOrigin}/.well-known/stellarr/client`,
      },
      { source: "/api/:path*", destination: `${apiOrigin}/api/:path*` },
      { source: "/assets/:path*", destination: `${apiOrigin}/assets/:path*` },
      { source: "/health/:path*", destination: `${apiOrigin}/health/:path*` },
      { source: "/hubs/:path*", destination: `${apiOrigin}/hubs/:path*` },
    ];
  },
  /**
   * Files under `public/` keep their names across releases and are served
   * with `public, max-age=0` unless named here. The build's own files under
   * `/_next/static/` are named by their content and immutable already.
   */
  async headers() {
    return [
      // Self-hosted fonts never change in place.
      cacheControl("/fonts/:path*", IMMUTABLE),
      // The worklets, the Bergamot runtime and the translation models keep
      // their names from release to release, so each must be revalidated
      // before use. Otherwise a browser could pair a cached worklet with a new
      // WASM, or keep running an old pair after an upgrade. The models are
      // downloaded once into IndexedDB, so revalidation costs nothing on later
      // use. The worker loads the files above and the registry, so it follows
      // their policy.
      cacheControl("/audio/:path*", REVALIDATE),
      cacheControl("/codecs/:path*", REVALIDATE),
      cacheControl("/bergamot-models/:path*", REVALIDATE),
      cacheControl("/bergamot-worker.js", REVALIDATE),
    ];
  },
};

export default nextConfig;
