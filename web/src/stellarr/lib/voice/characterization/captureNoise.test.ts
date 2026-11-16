/**
 * The capture record, continued: steady and unvoiced noise, alone and under
 * speech. See captureVoice.test.ts and web/bench/voice/README.md.
 */
import { describe } from "vitest";

import { captureCases } from "./captureCases";

describe("capture record", () => {
  captureCases(["fan-speech", "speech-crunching", "buzz", "mains-hum", "breath-chewing"]);
});
