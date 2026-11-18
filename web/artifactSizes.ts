import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import type { Plugin } from "vite";

/*
 * The size of every prebuilt file the browser loads at run time, as
 * artifacts/manifest.json records it, keyed by the path it is served under
 * ("audio/rnnoise.wasm"). The manifest is the one place those sizes are
 * written, and artifacts/assemble.mjs refuses a served file of another size,
 * so a client that shows how far a download has come takes its total from
 * here rather than from the response: nginx serves the .gz beside a file,
 * which makes Content-Length the compressed size while the body arrives
 * decoded.
 *
 * The build reads the manifest and writes only this map into the bundle.
 */

/** What the client imports; `env.d.ts` gives it its type. */
export const ARTIFACT_SIZES_MODULE = "virtual:artifact-sizes";

const resolvedId = `\0${ARTIFACT_SIZES_MODULE}`;
const manifestFile = fileURLToPath(new URL("../artifacts/manifest.json", import.meta.url));

interface Manifest {
  artifacts: ReadonlyArray<{ path: string; size: number }>;
}

export function stellarrArtifactSizes(): Plugin {
  return {
    name: "stellarr-artifact-sizes",
    resolveId(source) {
      return source === ARTIFACT_SIZES_MODULE ? resolvedId : null;
    },
    load(id) {
      if (id !== resolvedId) return null;
      // A changed manifest reaches the dev server without a restart.
      this.addWatchFile(manifestFile);
      const manifest = JSON.parse(readFileSync(manifestFile, "utf8")) as Manifest;
      const sizes = Object.fromEntries(manifest.artifacts.map((entry) => [entry.path, entry.size]));
      return `export default Object.freeze(${JSON.stringify(sizes)});\n`;
    },
  };
}
