/**
 * The capture record: the speech detector, the gate and the leveller as
 * `vad-processor.js` composes them in its worklet, held to
 * web/bench/voice/records/capture/. The fixtures are split over three files
 * so that they run side by side. See web/bench/voice/README.md.
 */
import { describe } from "vitest";

import { captureCases, captureIndexCase } from "./captureCases";

describe("capture record", () => {
  captureIndexCase();
  captureCases(["speech", "talker-quiet", "cold-start", "silence"]);
});
