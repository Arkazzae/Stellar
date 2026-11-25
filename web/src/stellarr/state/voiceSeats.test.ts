import { describe, expect, it, vi } from "vitest";

import type { VoicePeer } from "@/lib/voice/types";

import { VoiceSeats } from "./voiceSeats";

function peer(userId: string, patch: Partial<VoicePeer> = {}): VoicePeer {
  return {
    userId,
    displayName: userId,
    muted: false,
    speaking: false,
    playbackVolume: 100,
    gated: false,
    suppressed: false,
    self: false,
    screen: null,
    camera: null,
    ...patch,
  };
}

describe("VoiceSeats", () => {
  it("tells only the person whose ring or microphone changed, and keeps everyone else's seat", () => {
    const seats = new VoiceSeats();
    seats.take([peer("ada"), peer("bo")]);
    const ada = vi.fn();
    const bo = vi.fn();
    seats.subscribe("ada", ada);
    seats.subscribe("bo", bo);
    const before = seats.get("bo");

    seats.take([peer("ada", { speaking: true }), peer("bo", { displayName: "Bo", playbackVolume: 40 })]);

    expect(ada).toHaveBeenCalledTimes(1);
    expect(seats.get("ada")).toEqual({ speaking: true, muted: false, sharing: false, camera: false });
    expect(bo).not.toHaveBeenCalled();
    expect(seats.get("bo")).toBe(before);
  });

  it("takes the seat of whoever leaves the call, and tells them", () => {
    const seats = new VoiceSeats();
    seats.take([peer("ada", { muted: true })]);
    const ada = vi.fn();
    seats.subscribe("ada", ada);

    seats.take([]);

    expect(ada).toHaveBeenCalledTimes(1);
    expect(seats.get("ada")).toBeNull();
  });
});
