import { describe, expect, it } from "vitest";

import { LEVEL_STEPS, VadPublisher } from "./messages";

describe("what the detector posts", () => {
  it("posts a speaking change at once, whatever the level rate limit, with nothing but speaking and level", () => {
    const publisher = new VadPublisher({ minFramesBetweenLevelPosts: 1000 });
    expect(publisher.consider({ speaking: true, level: 0.5 })).toEqual({ speaking: true, level: 0.5 });
    expect(publisher.consider({ speaking: false, level: 0 })).toEqual({ speaking: false, level: 0 });
    const message = publisher.consider({ speaking: true, level: 0 });
    expect(Object.keys(message ?? {}).sort()).toEqual(["level", "speaking"]);
  });

  it("posts one baseline, then a level change at most every N frames, in the meter's steps", () => {
    const publisher = new VadPublisher({ minFramesBetweenLevelPosts: 4 });
    const posts = [];
    for (let i = 0; i < 40; i += 1) {
      const message = publisher.consider({ speaking: false, level: 0.001 });
      if (message) posts.push(message);
    }
    expect(posts).toEqual([{ speaking: false, level: 0 }]);

    // Prime past the baseline so the count starts from a known post.
    const limited = new VadPublisher({ minFramesBetweenLevelPosts: 4 });
    while (limited.consider({ speaking: false, level: 0 }) === null) {
      /* the baseline post */
    }
    expect(limited.consider({ speaking: false, level: 0.25 })).toBeNull();
    expect(limited.consider({ speaking: false, level: 0.375 })).toBeNull();
    expect(limited.consider({ speaking: false, level: 0.5 })).toBeNull();
    expect(limited.consider({ speaking: false, level: 0.4213 })).toEqual({
      speaking: false,
      level: Math.round(0.4213 * LEVEL_STEPS) / LEVEL_STEPS,
    });

    const unlimited = new VadPublisher({ minFramesBetweenLevelPosts: 0 });
    expect(unlimited.consider({ speaking: false, level: 4 })?.level).toBe(1);
    expect(unlimited.consider({ speaking: true, level: -3 })?.level).toBe(0);
  });

  it("re-announces after a reset so a new subscriber is not left stale", () => {
    const publisher = new VadPublisher({ minFramesBetweenLevelPosts: 0 });
    publisher.consider({ speaking: true, level: 0.5 });
    expect(publisher.consider({ speaking: true, level: 0.5 })).toBeNull();
    publisher.reset();
    expect(publisher.consider({ speaking: true, level: 0.5 })).toEqual({ speaking: true, level: 0.5 });
  });

  it("publishes one clean release but no remote room-noise levels", () => {
    const publisher = new VadPublisher({ minFramesBetweenLevelPosts: 0, publishSilentLevels: false });
    expect(publisher.consider({ speaking: false, level: 0.5 })).toBeNull();
    expect(publisher.consider({ speaking: true, level: 0.75 })).toEqual({ speaking: true, level: 0.75 });
    expect(publisher.consider({ speaking: false, level: 0.5 })).toEqual({ speaking: false, level: 0 });
    expect(publisher.consider({ speaking: false, level: 0.25 })).toBeNull();
  });
});
