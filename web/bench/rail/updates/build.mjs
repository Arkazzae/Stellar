// Builds the web client unminified into web/.bench-dist/<name>, so the probe
// can name the components React renders. The up-front size budget is left out:
// an unminified build is over it by design.
//
//   node bench/rail/updates/build.mjs [name]
import { resolve } from "node:path";

import { build } from "vite";

const webRoot = resolve(import.meta.dirname, "..", "..", "..");
const name = process.argv[2] ?? "rail-updates";
// Relative to the root: the release plugin joins the two itself.
const outDir = `.bench-dist/${name}`;

await build({
  root: webRoot,
  configFile: resolve(webRoot, "vite.config.ts"),
  configLoader: "runner",
  mode: "production",
  logLevel: "warn",
  build: { outDir, emptyOutDir: true, minify: false, sourcemap: false, reportCompressedSize: false },
  plugins: [
    {
      name: "probe-drop-budget",
      configResolved(config) {
        const index = config.plugins.findIndex((plugin) => plugin.name === "stellarr-bundle-report");
        if (index >= 0) config.plugins.splice(index, 1);
      },
    },
  ],
});
console.log(outDir);
