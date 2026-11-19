/**
 * Where the browser loads the Speex worklet from.
 *
 * `?worker&url` makes Vite bundle `processor.ts` on its own, with everything
 * it imports and nothing of the app's, and hand back the address of that
 * file. web/worklets.ts names the file /audio/speex-worklet.js in the build;
 * the dev server serves the source module itself.
 */
import speexWorklet from "./processor?worker&url";

export const SPEEX_WORKLET_URL: string = speexWorklet;
