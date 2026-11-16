/**
 * The capture record's test cases, shared by the files that split the
 * fixtures between them so that Vitest runs them side by side.
 */
import { describe, expect, it } from "vitest";

import {
  checkFixture,
  checkFixtureDigest,
  checkIndex,
  explain,
  type Outcome,
} from "../../../../../bench/voice/capture/check";

/**
 * How long one fixture through every configuration may take. It is about a
 * second of work on an idle machine, but the suite also runs on loaded ones:
 * with 10 cores at a load average of 200 to 265 these took up to 34 s, past
 * the suite's 20 s guard against a hung test, and this leaves five times that.
 */
export const CAPTURE_TIMEOUT_MS = 180_000;

function settle(outcome: Outcome, skip: (note: string) => void): void {
  // A run with STELLARR_VOICE_RECORD=1 writes the record; it is reported as
  // skipped, never as passed.
  if (outcome.written) skip(`recorded ${outcome.name}`);
  expect(outcome.problems, explain(outcome)).toEqual([]);
}

export function captureIndexCase(): void {
  it(
    "delays the audio by the latency recorded for each configuration",
    async (context) => {
      settle(await checkIndex(), (note) => context.skip(note));
    },
    CAPTURE_TIMEOUT_MS,
  );
}

export function captureCases(names: readonly string[]): void {
  for (const name of names) {
    describe(`fixture "${name}"`, () => {
      it("is the audio its record was made from", (context) => {
        settle(checkFixtureDigest(name), (note) => context.skip(note));
      });

      it(
        "goes through every detector configuration as recorded",
        async (context) => {
          settle(await checkFixture(name), (note) => context.skip(note));
        },
        CAPTURE_TIMEOUT_MS,
      );
    });
  }
}
