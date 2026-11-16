import { fileURLToPath } from "node:url";

import type { UserConfig } from "vite";

/*
 * The AudioWorklet modules the build makes from the app's own source. An
 * AudioWorklet runs in a global scope of its own and can share no chunk with
 * the app, so each is bundled by itself through Vite's worker pipeline
 * (`import url from "./entry?worker&url"`), which builds every worker entry
 * apart from the app. This names their files: a worklet keeps one address
 * across releases, beside the prebuilt ones in /audio/, and nginx serves
 * everything there with Cache-Control: no-cache (infra/nginx.conf), so a
 * browser asks again at each load and never runs a worklet of another
 * release.
 *
 * Plain enough for Node to load as it is, which the playback benchmark's
 * runner does.
 */

/** Each worklet entry, by its path from web/, and the file the build writes it to. */
export const WORKLETS: Readonly<Record<string, string>> = Object.freeze({
  "src/stellarr/lib/voice/capture/detector/processor.ts": "audio/detector-worklet.js",
});

const entries = new Map(
  Object.entries(WORKLETS).map(([source, file]) => [fileURLToPath(new URL(source, import.meta.url)), file]),
);

/**
 * Vite's `worker` options for a build of the app: ES modules, as an
 * AudioWorklet loads its module, written under the names above. A worker that
 * is not one of these keeps Vite's own name, below `assetsDir`.
 */
export function workerBuild(assetsDir: string): NonNullable<UserConfig["worker"]> {
  return {
    format: "es",
    rolldownOptions: {
      output: {
        entryFileNames: (chunk) =>
          entries.get(chunk.facadeModuleId ?? "") ?? `${assetsDir}/[name]-[hash].js`,
      },
    },
  };
}
