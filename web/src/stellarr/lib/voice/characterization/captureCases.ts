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

function settle(outcome: Outcome, skip: (note: string) => void): void {
  // A run with STELLARR_VOICE_RECORD=1 writes the record; it is reported as
  // skipped, never as passed.
  if (outcome.written) skip(`recorded ${outcome.name}`);
  expect(outcome.problems, explain(outcome)).toEqual([]);
}

export function captureIndexCase(): void {
  it("delays the audio by the latency recorded for each configuration", async (context) => {
    settle(await checkIndex(), (note) => context.skip(note));
  });
}

export function captureCases(names: readonly string[]): void {
  for (const name of names) {
    describe(`fixture "${name}"`, () => {
      it("is the audio its record was made from", (context) => {
        settle(checkFixtureDigest(name), (note) => context.skip(note));
      });

      it("goes through every detector configuration as recorded", async (context) => {
        settle(await checkFixture(name), (note) => context.skip(note));
      });
    });
  }
}
