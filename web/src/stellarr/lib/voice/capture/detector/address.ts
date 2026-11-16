/**
 * Where the browser loads the detector worklet from.
 *
 * `?worker&url` makes Vite bundle `processor.ts` on its own, with everything
 * it imports and nothing of the app's, and hand back the address of that
 * file. web/worklets.ts names the file /audio/detector-worklet.js in the
 * build; the dev server serves the source module itself.
 */
import detectorWorklet from "./processor?worker&url";

export const DETECTOR_WORKLET_URL: string = detectorWorklet;
