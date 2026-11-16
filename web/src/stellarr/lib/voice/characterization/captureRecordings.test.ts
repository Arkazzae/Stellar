/**
 * The capture record over real recordings, when there are any in the
 * git-ignored web/bench/voice/recordings/. A recording without a record gets
 * one written beside it and is reported as skipped, not as passed. See
 * web/bench/voice/README.md.
 */
import { describe, expect, it } from "vitest";

import { checkRecording, listRecordings } from "../../../../../bench/voice/capture/recordings";

const recordings = listRecordings();

/**
 * A recording runs through ten configurations, about four seconds of work
 * for half a minute of speech on an idle machine; on a loaded one (a load
 * average of 200 on 10 cores) one took 131 s. This leaves room for that and
 * for a longer recording.
 */
const RECORDING_TIMEOUT_MS = 600_000;

describe("capture record of local recordings", () => {
  if (recordings.length === 0) it.skip("finds no recordings to run over", () => undefined);

  for (const name of recordings) {
    it(
      `matches the record kept beside ${name}`,
      async (context) => {
        const outcome = await checkRecording(name);
        if (outcome.written) context.skip(`recorded ${name} for the first time:\n${outcome.summary}`);
        expect(outcome.problems, `${name}:\n  ${outcome.problems.join("\n  ")}`).toEqual([]);
      },
      RECORDING_TIMEOUT_MS,
    );
  }
});
