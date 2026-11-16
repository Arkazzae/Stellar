/**
 * The capture record: the speech detector, the gate and the leveller as
 * the detector worklet (`capture/detector/processor.ts`) composes them, held to
 * web/bench/voice/records/capture/. The fixtures are split over three files
 * so that they run side by side. See web/bench/voice/README.md.
 */
import { describe } from "vitest";

import { captureCases, captureIndexCase } from "./captureCases";

describe("capture record", () => {
  captureIndexCase();
  captureCases(["speech", "talker-quiet", "cold-start", "silence"]);
});
