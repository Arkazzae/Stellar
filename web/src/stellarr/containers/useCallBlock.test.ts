import { describe, expect, it } from "vitest";

import { callBlockOf } from "./useCallBlock";

const on = { encryption: { state: "on" as const, requestedById: null, requestedAt: null, enabledAt: "2026-01-01T00:00:00.000Z", currentEpochId: null } };
const requested = { encryption: { ...on.encryption, state: "requested" as const } };

describe("callBlockOf", () => {
  it("blocks only an encrypted conversation's call, for the first reason this browser cannot encrypt it", () => {
    expect(callBlockOf(on, "supported", false, true)).toBeNull();
    expect(callBlockOf(on, "insecure", true, false)).toBe("insecure");
    expect(callBlockOf(on, "unsupported", false, true)).toBe("unsupported");
    expect(callBlockOf(on, "supported", false, false)).toBe("unsupported");
    expect(callBlockOf(on, "supported", true, true)).toBe("device-limit");
    // Not decided yet: the voice client refuses if it comes to that.
    expect(callBlockOf(on, "checking", false, true)).toBeNull();
  });

  it("never blocks a conversation that is not encrypted, whatever this browser lacks", () => {
    expect(callBlockOf(requested, "unsupported", true, false)).toBeNull();
    expect(callBlockOf({}, "insecure", false, false)).toBeNull();
    expect(callBlockOf(null, "unsupported", false, false)).toBeNull();
  });
});
