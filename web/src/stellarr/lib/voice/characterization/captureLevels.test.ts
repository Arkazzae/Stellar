/**
 * The capture record, continued: loud and clipped input, clicks, whispers.
 * See captureVoice.test.ts and web/bench/voice/README.md.
 */
import { describe } from "vitest";

import { captureCases } from "./captureCases";

describe("capture record", () => {
  captureCases(["talker-loud-after-quiet", "clipping", "speech-clicks", "whisper", "keyboard"]);
});
